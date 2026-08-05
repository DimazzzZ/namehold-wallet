/**
 * Web-QA mock backend.
 *
 * When running in a plain browser (no Tauri), every `invoke()` call is routed
 * here.  The mock returns realistic placeholder data so the full UI renders
 * and can be clicked through without crashing.
 *
 * Commands not listed in the map fall through to a console warning and return
 * `null`, which keeps the app functional (queries degrade gracefully).
 */
/* eslint-disable @typescript-eslint/no-unused-vars */

type Handler = (args?: Record<string, unknown>) => unknown;

// ═══════════════════════════════════════════════════════════════════════════
// Block-driven auction lifecycle engine
// ═══════════════════════════════════════════════════════════════════════════
//
// The mock used to be static: every read returned canned data and a build/
// broadcast changed nothing, so lifecycle flows (open → bid → reveal → close)
// could not be exercised in the browser at all.
//
// This engine gives the mock a single mutable `chainHeight` plus a per-name
// auction record. Reads derive the on-chain phase from `chainHeight` vs each
// name's phase-window heights, and writes (build_*_draft + broadcast) mutate
// the record. A dev-only "mine blocks" control (`__webqa_mine`, also exposed
// as the `webqa_mine_blocks` invoke command) advances `chainHeight` so a QA
// session can walk a name through its whole lifecycle deterministically.
//
// SCOPE / LIMITS (intentional):
//  - State lives in module memory only — a full page reload resets it. There
//    is no localStorage persistence (explicitly out of scope).
//  - One bidder (this wallet). We don't simulate rival bidders beyond a single
//    canned "other" bid so win/lose can be demoed.
//  - Block times are compressed: phase windows are short so "mine a few blocks"
//    crosses a phase boundary. These are NOT real Handshake durations.

/** Compressed phase-window lengths (blocks) — short so QA can mine across them. */
const OPEN_BLOCKS = 3;
const BID_BLOCKS = 5;
const REVEAL_BLOCKS = 5;
/** Heuristic hsd-ish minutes per block, only for the *hoursUntil* display. */
const MINUTES_PER_BLOCK = 10;

type MockPhase =
  | "AVAILABLE"
  | "OPENING"
  | "BIDDING"
  | "REVEAL"
  | "CLOSED";

type RevealDraftStatus =
  | "none"
  | "broadcasted"
  | "confirmed"
  | "dropped";

interface MockAuction {
  name: string;
  /** Height at which OPEN was broadcast; phase windows are relative to it. */
  openHeight: number | null;
  /** This wallet has a bid commitment (placed a bid). */
  hasBid: boolean;
  /** This wallet's true bid value (doos). */
  bidValueDoos: number;
  /** This wallet's lockup (doos) — the public blind, ≥ bid. */
  lockupDoos: number;
  /** Reveal lifecycle for THIS wallet's bid. */
  revealStatus: RevealDraftStatus;
  /** Reveal txid once broadcast (stamped on the commitment row analogue). */
  revealTxid: string | null;
  /** Height the reveal confirmed at (for "confirmed in block N"). */
  revealConfirmedHeight: number | null;
  /** A canned rival bid so CLOSED can resolve to won/lost. */
  rivalBidDoos: number;
}

interface MockChain {
  height: number;
  auctions: Map<string, MockAuction>;
}

const chain: MockChain = {
  height: 100_000,
  auctions: new Map(),
};

function ensureAuction(name: string): MockAuction {
  let a = chain.auctions.get(name);
  if (!a) {
    a = {
      name,
      openHeight: null,
      hasBid: false,
      bidValueDoos: 0,
      lockupDoos: 0,
      revealStatus: "none",
      revealTxid: null,
      revealConfirmedHeight: null,
      rivalBidDoos: 3_000_000, // 3 HNS rival — beatable, so wins are demoable
    };
    chain.auctions.set(name, a);
  }
  return a;
}

/** Derive the current on-chain phase for a name from `chain.height`. */
function phaseOf(a: MockAuction): MockPhase {
  if (a.openHeight == null) return "AVAILABLE";
  const elapsed = chain.height - a.openHeight;
  if (elapsed < OPEN_BLOCKS) return "OPENING";
  if (elapsed < OPEN_BLOCKS + BID_BLOCKS) return "BIDDING";
  if (elapsed < OPEN_BLOCKS + BID_BLOCKS + REVEAL_BLOCKS) return "REVEAL";
  return "CLOSED";
}

/** True once the reveal tx is considered confirmed at the current height. */
function revealIsConfirmed(a: MockAuction): boolean {
  return (
    a.revealStatus === "confirmed" &&
    a.revealConfirmedHeight != null &&
    chain.height >= a.revealConfirmedHeight
  );
}

/** Blocks remaining until the next phase boundary + a rough hours estimate. */
function countdownFor(a: MockAuction): {
  label: string | null;
  blocks: number | null;
  hours: number | null;
} {
  if (a.openHeight == null) return { label: null, blocks: null, hours: null };
  const elapsed = chain.height - a.openHeight;
  const phase = phaseOf(a);
  let label: string | null = null;
  let boundary: number | null = null;
  if (phase === "OPENING") {
    label = "Bidding starts in";
    boundary = OPEN_BLOCKS;
  } else if (phase === "BIDDING") {
    label = "Reveal starts in";
    boundary = OPEN_BLOCKS + BID_BLOCKS;
  } else if (phase === "REVEAL") {
    label = "Auction closes in";
    boundary = OPEN_BLOCKS + BID_BLOCKS + REVEAL_BLOCKS;
  } else {
    return { label: null, blocks: null, hours: null };
  }
  const blocks = Math.max(0, boundary - elapsed);
  const hours = Math.round((blocks * MINUTES_PER_BLOCK) / 60);
  return { label, blocks, hours };
}

function draftId(a: MockAuction): string {
  return `draft-reveal-${a.name}`;
}

/** Advance the mock chain by `n` blocks and settle any pending reveals. */
function mineBlocks(n: number): number {
  const count = Math.max(1, Math.floor(n || 1));
  chain.height += count;
  // A broadcast reveal confirms on the next mined block.
  for (const a of chain.auctions.values()) {
    if (a.revealStatus === "broadcasted") {
      a.revealStatus = "confirmed";
      a.revealConfirmedHeight = chain.height;
    }
  }
  return chain.height;
}

// Expose a dev control on the window so QA (and Playwright) can advance blocks
// without a real chain. Guarded so it's a no-op outside a browser.
declare global {
  // eslint-disable-next-line no-var
  var __webqa_mine: ((n?: number) => number) | undefined;
}
if (typeof globalThis !== "undefined") {
  globalThis.__webqa_mine = (n?: number) => mineBlocks(n ?? 1);
}

// ── Capability builder (mirrors the Rust task-state derivation) ─────────────

function cap(allowed: boolean, reason: string | null = null) {
  return { allowed, reason };
}

function taskStateOf(a: MockAuction): {
  taskState: string;
  nextActionKey: string | null;
  nextActionLabel: string | null;
} {
  const phase = phaseOf(a);
  switch (phase) {
    case "AVAILABLE":
      return {
        taskState: "availableToOpen",
        nextActionKey: "OPEN",
        nextActionLabel: "Open Auction",
      };
    case "OPENING":
      return {
        taskState: "waitingForBidding",
        nextActionKey: null,
        nextActionLabel: "Wait for Bidding",
      };
    case "BIDDING":
      return a.hasBid
        ? {
            taskState: "waitingForBidding",
            nextActionKey: null,
            nextActionLabel: "Wait for Bidding",
          }
        : {
            taskState: "readyToBid",
            nextActionKey: "BID",
            nextActionLabel: "Bid",
          };
    case "REVEAL": {
      if (!a.hasBid) {
        return {
          taskState: "unavailableOther",
          nextActionKey: null,
          nextActionLabel: null,
        };
      }
      // Reveal state machine: broadcasted → pending, confirmed → done,
      // dropped/none → still ready to reveal.
      if (a.revealStatus === "broadcasted") {
        return {
          taskState: "revealBroadcastPending",
          nextActionKey: null,
          nextActionLabel: "View",
        };
      }
      if (revealIsConfirmed(a)) {
        return {
          taskState: "revealDoneWaitingForClose",
          nextActionKey: null,
          nextActionLabel: "View",
        };
      }
      return {
        taskState: "readyToReveal",
        nextActionKey: "REVEAL",
        nextActionLabel: "Reveal Bid",
      };
    }
    case "CLOSED": {
      if (!a.hasBid) {
        return {
          taskState: "unavailableOther",
          nextActionKey: null,
          nextActionLabel: null,
        };
      }
      const won = revealIsConfirmed(a) && a.bidValueDoos > a.rivalBidDoos;
      return won
        ? {
            taskState: "wonNeedsRegister",
            nextActionKey: "REGISTER",
            nextActionLabel: "Register",
          }
        : {
            taskState: "lostNeedsRedeem",
            nextActionKey: "REDEEM",
            nextActionLabel: "Redeem",
          };
    }
  }
}

function buildCapabilities(name: string): Record<string, unknown> {
  const a = ensureAuction(name);
  const phase = phaseOf(a);
  const { taskState, nextActionKey, nextActionLabel } = taskStateOf(a);
  const cd = countdownFor(a);
  const hasBidCoin = a.hasBid && !revealIsConfirmed(a);
  const hasRevealCoin = revealIsConfirmed(a);
  const closed = phase === "CLOSED";
  const won = closed && revealIsConfirmed(a) && a.bidValueDoos > a.rivalBidDoos;
  return {
    name,
    phase,
    taskState,
    ownsName: won,
    hasBidCommitment: a.hasBid,
    hasBidCoin,
    hasRevealCoin,
    hasOwnerCoin: won,
    // `revealTxid` is the ONLY new field surfaced for the reveal card.
    revealTxid: a.revealTxid,
    // The user's own bid value (doos), so the confirm panel can show it.
    bidValueDoos: a.hasBid ? a.bidValueDoos : null,
    canOpen: cap(phase === "AVAILABLE"),
    canBid: cap(phase === "BIDDING" && !a.hasBid),
    canReveal: cap(
      phase === "REVEAL" &&
        a.hasBid &&
        hasBidCoin &&
        a.revealStatus !== "broadcasted",
    ),
    canRedeem: cap(closed && !won && a.hasBid),
    canRegister: cap(won),
    canUpdate: cap(won),
    canTransfer: cap(won),
    canFinalize: cap(false),
    canCancelTransfer: cap(false),
    canRenew: cap(won),
    canRevoke: cap(won),
    nextActionKey,
    nextActionLabel,
    nextActionReason: null,
    countdownLabel: cd.label,
    countdownBlocks: cd.blocks,
    countdownHours: cd.hours,
  };
}

/** Names this wallet has an active auction position in. */
function auctionPositionNames(): string[] {
  const out: string[] = [];
  for (const a of chain.auctions.values()) {
    const phase = phaseOf(a);
    if (a.openHeight != null && (a.hasBid || phase !== "CLOSED")) {
      out.push(a.name);
    }
  }
  return out;
}

// Seed one name that is already in the REVEAL phase with a bid placed, so the
// reported "Ready to Reveal → click Reveal → ???" flow is demoable on load
// without first walking open+bid.
(function seedRevealScenario() {
  const a = ensureAuction("namehold");
  // Place its open far enough back that we're mid-REVEAL right now.
  a.openHeight = chain.height - (OPEN_BLOCKS + BID_BLOCKS + 1);
  a.hasBid = true;
  a.bidValueDoos = 5_000_000; // 5 HNS — beats the 3 HNS rival → will win
  a.lockupDoos = 8_000_000; // 8 HNS lockup
  a.revealStatus = "none";
})();

const handlers: Record<string, Handler> = {
  // ── Settings ──────────────────────────────────────────────────────────
  get_settings: () => ({
    node_rpc_url: "http://127.0.0.1:12037",
    node_rpc_api_key: "",
    hsd_prefix: "",
    hsd_path: "",
    explorer_api_url: "https://e.hnsfans.com",
    address_gap_limit: "20",
    signer_session_timeout_seconds: "900",
    advanced_mode: "false",
    onboarding_complete: "true",
    background_sync_enabled: "1",
    node_mode: "full",
    explorer_fallback_url: "",
    chain_source: "local_node",
  }),

  update_setting: () => null,

  // ── Daemon control ────────────────────────────────────────────────────
  is_background_sync_enabled: () => true,
  set_background_sync_enabled: () => null,
  is_daemon_alive: () => false,

  // ── Updates ───────────────────────────────────────────────────────────
  // Browser QA has no real updater; report a fixed version and "up to date".
  current_version: () => "0.3.0",
  check_for_update: () => null,
  install_update: () => null,

  // ── Wallet profiles ───────────────────────────────────────────────────
  list_wallet_profiles: () => [
    {
      id: "webqa-profile",
      label: "QA Wallet",
      kind: "mnemonic_hot",
      network: "mainnet",
      accountXpub: "xpub6C...(mock)",
      accountIndex: 0,
      receiveDepth: 5,
      changeDepth: 3,
      receiveAddress: "hs1q9g6...(mock)",
      lastSyncedHeight: 100000,
      lastSyncedAt: new Date().toISOString(),
      lastExplorerSyncAt: new Date().toISOString(),
      watchOnly: false,
      hasPassphrase: false,
      active: true,
    },
  ],

  get_signer_session: () => ({
    walletProfileId: "webqa-profile",
    unlocked: true,
    unlockedUntilEpochMs: Date.now() + 3_600_000,
  }),

  get_write_capability: () => ({
    signerUnlocked: true,
    broadcasterAvailable: true,
    canWrite: true,
    reason: null,
  }),

  // ── Balances ──────────────────────────────────────────────────────────
  get_wallet_balances: () => ({
    liquidDoos: 5_000_000_000,
    nameControlDoos: 1_200_000_000,
    nameLockupDoos: 800_000_000,
    totalDoos: 7_000_000_000,
  }),

  read_balance: () => ({
    confirmed: 5_000_000_000,
    unconfirmed: 0,
    locked_unconfirmed: 0,
    locked_confirmed: 800_000_000,
  }),

  // ── Dev control: advance the mock chain ───────────────────────────────
  // Exposed so browser QA / Playwright can mine blocks and walk a name
  // through its lifecycle deterministically.
  webqa_mine_blocks: (args) => mineBlocks((args?.count as number) ?? 1),

  // ── Names ─────────────────────────────────────────────────────────────
  read_names: () => {
    // A static owned name so the wallet/portfolio views aren't empty…
    const staticNames = [
      {
        name: "example",
        state: "CLOSED",
        height: 50000,
        renewal: 100000,
        owner: { hash: "abcd1234", index: 0 },
        value: 100_000_000,
        highest: 100_000_000,
        stats: {
          renewalPeriodStart: 80000,
          renewalPeriodEnd: 110000,
          blocksUntilExpire: 10000,
          daysUntilExpire: 69,
        },
        registered: true,
        expired: false,
      },
    ];
    // …plus every name tracked by the lifecycle engine, rendered at its
    // current derived phase.
    const engineNames = [...chain.auctions.values()].map((a) => {
      const phase = phaseOf(a);
      const cd = countdownFor(a);
      return {
        name: a.name,
        state: phase,
        height: a.openHeight,
        renewal: null,
        owner: null,
        value: phase === "CLOSED" ? a.bidValueDoos : null,
        highest: phase === "CLOSED" ? a.bidValueDoos : null,
        stats: {
          blocksUntilClose: phase === "REVEAL" ? cd.blocks : null,
          hoursUntilClose: phase === "REVEAL" ? cd.hours : null,
          blocksUntilReveal: phase === "BIDDING" ? cd.blocks : null,
          hoursUntilReveal: phase === "BIDDING" ? cd.hours : null,
        },
        registered: false,
        expired: false,
      };
    });
    return [...staticNames, ...engineNames];
  },

  read_renewals: () => ({
    walletProfileId: "webqa-profile",
    currentHeight: 100_000,
    heightSource: "explorer",
    expiringSoonThresholdDays: 30,
    names: [
      {
        name: "example",
        state: "CLOSED",
        renewalHeight: 100_000 - 105_120 + 10_000,
        expiresAtHeight: 110_000,
        blocksUntilExpire: 10_000,
        daysUntilExpire: 69.4,
        source: "chain",
        expiringSoon: false,
      },
      {
        name: "urgent",
        state: "CLOSED",
        renewalHeight: 100_000 - 105_120 + 1_000,
        expiresAtHeight: 101_000,
        blocksUntilExpire: 1_000,
        daysUntilExpire: 6.9,
        source: "chain",
        expiringSoon: true,
      },
      {
        name: "legacycsv",
        state: "CLOSED",
        renewalHeight: null,
        expiresAtHeight: 500_000,
        blocksUntilExpire: null,
        daysUntilExpire: 42.5,
        source: "csv-import",
        expiringSoon: false,
      },
    ],
  }),

  read_name_info: (_args) => {
    const name = (_args?.name as string) ?? "unknown";
    const a = ensureAuction(name);
    const phase = phaseOf(a);
    const cd = countdownFor(a);
    return {
      name,
      state: phase,
      height: a.openHeight,
      renewal: null,
      owner: null,
      value: phase === "CLOSED" ? a.bidValueDoos : null,
      highest: phase === "CLOSED" ? a.bidValueDoos : null,
      stats: {
        blocksUntilClose: phase === "REVEAL" ? cd.blocks : null,
        hoursUntilClose: phase === "REVEAL" ? cd.hours : null,
        blocksUntilReveal: phase === "BIDDING" ? cd.blocks : null,
        hoursUntilReveal: phase === "BIDDING" ? cd.hours : null,
      },
      registered: false,
      expired: false,
    };
  },

  // ── Auction capabilities / positions (lifecycle-engine driven) ────────
  get_name_action_capabilities: (args) =>
    buildCapabilities((args?.name as string) ?? "unknown"),

  get_names_action_capabilities: (args) => {
    const names = (args?.names as string[]) ?? [];
    return names.map((n) => buildCapabilities(n));
  },

  read_auction_position_names: () => auctionPositionNames(),

  read_name_bids: (args) => {
    const name = (args?.name as string) ?? "unknown";
    const a = ensureAuction(name);
    const phase = phaseOf(a);
    const revealed = revealIsConfirmed(a);
    const showValues = phase === "REVEAL" || phase === "CLOSED";
    const bids = [] as unknown[];
    if (a.hasBid) {
      bids.push({
        txid: "bid00000mine",
        index: 0,
        lockup: a.lockupDoos,
        value: revealed ? a.bidValueDoos : null,
        revealed,
        win: phase === "CLOSED" ? a.bidValueDoos > a.rivalBidDoos : null,
        reveal: null,
        time: Date.now(),
        mine: true,
        myValue: a.bidValueDoos,
      });
    }
    if (a.openHeight != null) {
      bids.push({
        txid: "bid0000rival",
        index: 0,
        lockup: a.rivalBidDoos + 1_000_000,
        value: showValues ? a.rivalBidDoos : null,
        revealed: showValues,
        win: phase === "CLOSED" ? a.rivalBidDoos >= a.bidValueDoos : null,
        reveal: null,
        time: Date.now(),
        mine: false,
        myValue: null,
      });
    }
    return {
      name,
      state: phase,
      highest: showValues ? Math.max(a.bidValueDoos, a.rivalBidDoos) : null,
      value: showValues ? a.bidValueDoos : null,
      bids,
      myBidCount: a.hasBid ? 1 : 0,
    };
  },

  // ── Transactions ──────────────────────────────────────────────────────
  read_transactions: () => [
    {
      hash: "deadbeef0001",
      direction: "receive",
      amountDoos: 1_000_000_000,
      amountHns: 10,
      address: "hs1q9g6...(mock)",
      confirmed: true,
      confirmations: 120,
      height: 99880,
      timestamp: new Date(Date.now() - 86_400_000).toISOString(),
      tone: "success",
    },
    {
      hash: "deadbeef0002",
      direction: "send",
      amountDoos: -500_000_000,
      amountHns: -5,
      address: "hs1qxy...(mock)",
      confirmed: true,
      confirmations: 60,
      height: 99940,
      timestamp: new Date(Date.now() - 43_200_000).toISOString(),
      tone: "default",
    },
  ],

  // ── Node ──────────────────────────────────────────────────────────────
  node_status: () => ({
    binary: "hsd",
    binary_found: false,
    version: "webqa-mock",
    data_dir: "",
    network: "mainnet",
    process_alive: false,
    // Report as connected+synced so `useNodeLive()` returns true and the new
    // capabilities/positions polling actually fires under the mock.
    connected: true,
    height: chain.height,
    verification_progress: 1,
    headers: chain.height,
    last_error: null,
    index_mismatch: false,
    read_source: "local",
    running: true,
    tip: "000000000000...(mock)",
    peers: 8,
    error: null,
  }),

  start_hsd: () => ({
    running: true,
    network: "mainnet",
    height: 100000,
    tip: "000000000000...(mock)",
    peers: 8,
    version: "webqa-mock",
    message: "Started (mock)",
  }),

  stop_hsd: () => null,
  resync_hsd_chain: () => null,

  // ── Drafts ────────────────────────────────────────────────────────────
  list_tx_drafts: () => {
    const drafts: unknown[] = [];
    for (const a of chain.auctions.values()) {
      if (a.revealStatus !== "none" && a.revealTxid) {
        drafts.push({
          id: draftId(a),
          walletProfileId: "webqa-profile",
          action: "reveal",
          status:
            a.revealStatus === "broadcasted"
              ? "broadcasted"
              : a.revealStatus === "confirmed"
                ? "confirmed"
                : "dropped",
          summary: {
            action: "reveal",
            sendTotalDoos: a.bidValueDoos,
            feeDoos: 100_000,
            changeDoos: a.lockupDoos - a.bidValueDoos,
            inputTotalDoos: a.lockupDoos,
            numInputs: 1,
            recipientAddress: null,
            txid: a.revealTxid,
            warnings: [],
            name: a.name,
          },
          errorMessage: null,
          txid: a.revealTxid,
          confirmationHeight: a.revealConfirmedHeight,
          createdAt: new Date().toISOString(),
        });
      }
    }
    return drafts;
  },

  // ── Bid commitment recovery / backup ────────────────────────────────────
  recover_bid_commitment: () => {
    throw new Error("bid value doesn't match any unspent bid coin for this name");
  },
  export_bid_commitments: () => "[]",

  build_send_hns_draft: (_args) => ({
    id: "draft-mock-001",
    walletProfileId: "webqa-profile",
    action: "send_hns",
    status: "draft",
    summary: {
      action: "send_hns",
      sendTotalDoos: (_args?.valueDoos as number) ?? 100_000_000,
      feeDoos: 100_000,
      changeDoos: 0,
      inputTotalDoos: ((_args?.valueDoos as number) ?? 100_000_000) + 100_000,
      numInputs: 1,
      recipientAddress: (_args?.toAddress as string) ?? "hs1q...(mock)",
      txid: null,
      warnings: [],
    },
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  sign_tx_draft: (args) => ({
    id: (args?.draftId as string) ?? "draft-mock-001",
    walletProfileId: "webqa-profile",
    action: "send_hns",
    status: "signed",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  broadcast_tx_draft: (args) => {
    const id = (args?.draftId as string) ?? "draft-mock-001";
    // Any reveal draft flips its auction to "broadcasted"; the next mined
    // block will settle it via `mineBlocks()`.
    if (id.startsWith("draft-reveal-")) {
      const name = id.slice("draft-reveal-".length);
      const a = ensureAuction(name);
      const txid =
        "revealtxid" +
        Math.floor(Math.random() * 1e12)
          .toString(16)
          .padStart(12, "0");
      a.revealStatus = "broadcasted";
      a.revealTxid = txid;
      return { draftId: id, txid, status: "broadcasted" };
    }
    return {
      draftId: id,
      txid: "cafebabe0001",
      status: "broadcasted",
    };
  },

  refresh_tx_confirmations: () => null,

  // The signer session helpers behave as already-unlocked, so `useExecuteDraft`
  // skips its unlock leg and jumps straight to sign → broadcast in the mock.
  unlock_local_signer: () => ({
    walletProfileId: "webqa-profile",
    unlocked: true,
    unlockedUntilEpochMs: Date.now() + 3_600_000,
  }),

  // ── Name action drafts (lifecycle-engine driven) ──────────────────────
  build_open_draft: (args) => {
    const name = (args?.name as string) ?? "unknown";
    const a = ensureAuction(name);
    // Broadcasting OPEN pins the phase-window origin at the current height.
    a.openHeight = chain.height;
    return {
      id: `draft-open-${name}`,
      walletProfileId: "webqa-profile",
      action: "open",
      status: "draft",
      summary: {
        action: "open",
        sendTotalDoos: 0,
        feeDoos: 100_000,
        changeDoos: 0,
        inputTotalDoos: 100_000,
        numInputs: 1,
        recipientAddress: null,
        txid: null,
        warnings: [],
        name,
      },
      errorMessage: null,
      txid: null,
      confirmationHeight: null,
      createdAt: new Date().toISOString(),
    };
  },

  build_bid_draft: (args) => {
    const name = (args?.name as string) ?? "unknown";
    const a = ensureAuction(name);
    const bidValueDoos = (args?.bidValueDoos as number) ?? 5_000_000;
    const lockupDoos = (args?.lockupDoos as number) ?? bidValueDoos * 2;
    a.hasBid = true;
    a.bidValueDoos = bidValueDoos;
    a.lockupDoos = lockupDoos;
    return {
      id: `draft-bid-${name}`,
      walletProfileId: "webqa-profile",
      action: "bid",
      status: "draft",
      summary: {
        action: "bid",
        sendTotalDoos: lockupDoos,
        feeDoos: 100_000,
        changeDoos: 0,
        inputTotalDoos: lockupDoos + 100_000,
        numInputs: 1,
        recipientAddress: null,
        txid: null,
        warnings: [],
        name,
      },
      errorMessage: null,
      txid: null,
      confirmationHeight: null,
      createdAt: new Date().toISOString(),
    };
  },

  build_reveal_draft: (args) => {
    const name = (args?.name as string) ?? "unknown";
    const a = ensureAuction(name);
    return {
      id: draftId(a),
      walletProfileId: "webqa-profile",
      action: "reveal",
      status: "draft",
      summary: {
        action: "reveal",
        sendTotalDoos: a.bidValueDoos,
        feeDoos: 100_000,
        changeDoos: a.lockupDoos - a.bidValueDoos,
        inputTotalDoos: a.lockupDoos,
        numInputs: 1,
        recipientAddress: null,
        txid: null,
        warnings: [],
        name,
      },
      errorMessage: null,
      txid: null,
      confirmationHeight: null,
      createdAt: new Date().toISOString(),
    };
  },

  build_redeem_draft: () => ({
    id: "draft-redeem-001",
    walletProfileId: "webqa-profile",
    action: "redeem",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_register_draft: () => ({
    id: "draft-register-001",
    walletProfileId: "webqa-profile",
    action: "register",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_update_draft: () => ({
    id: "draft-update-001",
    walletProfileId: "webqa-profile",
    action: "update",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_renew_draft: () => ({
    id: "draft-renew-001",
    walletProfileId: "webqa-profile",
    action: "renew",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_transfer_draft: () => ({
    id: "draft-transfer-001",
    walletProfileId: "webqa-profile",
    action: "transfer",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_finalize_draft: () => ({
    id: "draft-finalize-001",
    walletProfileId: "webqa-profile",
    action: "finalize",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_cancel_draft: () => ({
    id: "draft-cancel-001",
    walletProfileId: "webqa-profile",
    action: "cancel",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_revoke_draft: () => ({
    id: "draft-revoke-001",
    walletProfileId: "webqa-profile",
    action: "revoke",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_batch_renew_draft: () => ({
    id: "draft-batch-renew-001",
    walletProfileId: "webqa-profile",
    action: "batch-renew",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_batch_reveal_draft: () => ({
    id: "draft-batch-reveal-001",
    walletProfileId: "webqa-profile",
    action: "batch-reveal",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_batch_redeem_draft: () => ({
    id: "draft-batch-redeem-001",
    walletProfileId: "webqa-profile",
    action: "batch-redeem",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_finalize_with_payment_draft: () => ({
    id: "draft-finalize-payment-001",
    walletProfileId: "webqa-profile",
    action: "finalize-with-payment",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  // ── Watchlist ─────────────────────────────────────────────────────────
  add_to_watchlist: () => null,
  remove_from_watchlist: () => null,
  list_watchlist: () => [],
  is_watched: () => false,

  // ── DNS resource ──────────────────────────────────────────────────────
  get_resource: () => ({
    records: [{ type: "NS", ns: "ns1.example." }],
  }),

  // ── Assets / Batches (portfolio) ──────────────────────────────────────
  list_assets: () => [],
  get_asset: () => null,
  update_asset: () => null,
  bulk_update_status: () => null,
  bulk_update_tags: () => null,
  delete_asset: () => null,
  import_csv: () => ({ imported: 0, skipped: 0, errors: [] }),
  export_csv: () => 0,

  list_batches: () => [],
  get_batch_with_assets: () => ({ id: 0, name: "", assets: [] }),
  create_batch: () => 1,
  update_batch: () => null,
  delete_batch: () => null,
  add_to_batch: () => null,
  remove_from_batch: () => null,

  // ── Audit / Sync ──────────────────────────────────────────────────────
  get_audit_log: () => [],
  compare_inventory_with_provider: () => ({
    matched: [],
    missing: [],
    extra: [],
  }),

  // ── Namebase (stubs) ──────────────────────────────────────────────────
  get_namebase_status: () => ({ connected: false, has_cookie: false }),
  fetch_namebase_domains: () => ({ domains: [] }),
  fetch_namebase_staked: () => ({ stakedDomains: [] }),
  connect_namebase: () => null,
  disconnect_namebase: () => null,
  import_from_namebase: () => ({ imported: 0, staked_count: 0 }),
  namebase_transfer_domain: () => null,
  fetch_namebase_domain_withdrawals: () => [],
  fetch_namebase_renewals: () => [],
  fetch_namebase_withdrawals: () => [],
  namebase_withdraw_hns: () => null,

  // ── Secure prompt ─────────────────────────────────────────────────────
  secure_prompt_submit: () => null,
  secure_prompt_fetch: () => ({
    promptId: "",
    kind: "info",
    message: "Mock",
  }),
  secure_reveal_backup_phrase: () =>
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
};

/**
 * Execute a mock invoke command.
 * Returns `null` with a console warning for any command not in the map.
 */
export function mockInvoke<T>(cmd: string, args?: Record<string, unknown>): T {
  const handler = handlers[cmd];
  if (handler) {
    return handler(args) as T;
  }
  console.warn(
    `[browser QA] No mock handler for invoke("${cmd}") — returning null.`,
  );
  return null as unknown as T;
}

/** Test-only: reset the engine to a known-empty state. Do NOT call from app code. */
export function __resetChainForTests(seedHeight = 100_000): void {
  chain.height = seedHeight;
  chain.auctions.clear();
}

/** Test-only: mine `n` blocks (same as the dev control / webqa_mine_blocks). */
export function __mineForTests(n = 1): number {
  return mineBlocks(n);
}
