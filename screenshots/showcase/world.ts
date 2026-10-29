/**
 * The showcase wallet: one coherent mainnet wallet that every scene reads.
 *
 * Every value is typed against the app's own `src/types`, so a backend shape
 * change breaks `pnpm typecheck` here instead of leaving a screenshot that the
 * real app can no longer produce. The next-action words and countdown labels
 * are the backend's (`next_action_for_task` in `commands/names.rs`,
 * `extract_countdown` in `commands/names_pure.rs`), not the browser mock's.
 *
 * Names are invented everyday words, addresses are well-formed but belong to
 * nobody (derived from fixed seeds), and the chain sits at a plausible
 * mainnet height with the real mainnet auction windows.
 */
import type {
  AuctionTaskState,
  HsdName,
  NameResource,
  NameActionCapabilities,
  NameActionCapability,
  ReceiveAddressRow,
  RenewalsResponse,
  ShakedexListingActions,
  ShakedexListingState,
  ShakedexMyListing,
  ShakedexNameListing,
  ShakedexStoredStep,
  WalletBalances,
  WalletProfileSummary,
} from "../../src/types";
import type { NamebaseHistoryRow, NamebaseHistorySummary } from "../../src/queries/namebase";
import LISTING_ACTIONS from "../../src/test/fixtures/listing-actions.json";

/** `hnsToDollarydoos` (src/lib/utils.ts), kept local: importing utils pulls in the IDN encoder. */
const hns = (n: number) => Math.round(n * 1_000_000);

/** A plausible mainnet tip: about 6.7 years of 10-minute blocks since launch. */
export const HEIGHT = 352_416;
/** Mainnet `bidding_period` / `reveal_period` (`noncustodial/network.rs`). */
const BIDDING_BLOCKS = 720;
const REVEAL_BLOCKS = 1440;
/** Mainnet `renewal_window`: a name expires this long after its last renewal. */
const RENEWAL_WINDOW = 105_120;
/** Mainnet's 10-minute blocks: 6 an hour, 144 a day; rounded as hsd's `toFixed(2)`. */
const hoursFor = (blocks: number) => Math.round((blocks / 6) * 100) / 100;
const daysFor = (blocks: number) => Math.round((blocks / 144) * 100) / 100;

const PROFILE_ID = "showcase-main";

/** Receive addresses, bech32 v0 over fixed seeds (see screenshots/README.md). */
const RECEIVE = [
  "hs1q5zwwkm5kkaewrn7hysmd9vp5u2hjd79lhjlxs8",
  "hs1q7gfhenkwaqefuzygsc6hv640suu0re346490kq",
  "hs1q2lsfqgznadjcqnsfsc9szyma5fhc3h9evrmxes",
  "hs1qfpzvglwlkezwdkwzj83fum7x3lnxnlkvk0dmkh",
  "hs1qkyrhm8yekjy3dunzclmn2ysq90jc2t4pa09uqe",
  "hs1qqk9t0exhlqdymug0xfda9jjf6r77wm3msg860n",
];
const XPUB =
  "xpub6CKY2eLKyjtgJmUypiUQUojgHWryWgTuvXzH6KjH1fqsq3Ci5kqMz9MbVaF5dsJX7x2gBzoQxYamshqfgtvAnzMRZoxq5ca35TXdaknqstb";

export function profiles(now: Date): WalletProfileSummary[] {
  return [
    {
      id: PROFILE_ID,
      label: "Main",
      kind: "mnemonic_hot",
      network: "mainnet",
      accountXpub: XPUB,
      accountIndex: 0,
      receiveDepth: RECEIVE.length,
      changeDepth: 4,
      receiveAddress: RECEIVE[RECEIVE.length - 1]!,
      lastSyncedHeight: HEIGHT,
      lastSyncedAt: now.toISOString(),
      lastExplorerSyncAt: now.toISOString(),
      watchOnly: false,
      hasPassphrase: true,
      active: true,
    },
  ];
}

export function receiveAddresses(now: Date): ReceiveAddressRow[] {
  const day = 86_400_000;
  return RECEIVE.map((address, index) => ({
    index,
    address,
    used: index < RECEIVE.length - 1,
    firstSeenAt: new Date(now.getTime() - (RECEIVE.length - index) * 41 * day).toISOString(),
  })).reverse();
}

// ── Names ───────────────────────────────────────────────────────────────────

/** Registered names this wallet owns, with blocks left until each expires. */
const OWNED: [string, number][] = [
  ["atlas", 61_204],
  ["lighthouse", 88_930],
  ["orchard", 23_517],
  ["copperleaf", 97_002],
];

/** Auctions this wallet takes part in, at their phase on `HEIGHT`. */
interface Auction {
  name: string;
  phase: "OPENING" | "BIDDING" | "REVEAL" | "CLOSED";
  task: AuctionTaskState;
  /** Blocks to the next phase boundary (OPENING/BIDDING/REVEAL). */
  blocksLeft?: number;
  bid?: number;
  lockup?: number;
  revealed?: boolean;
}

const AUCTIONS: Auction[] = [
  {
    name: "saltmarsh",
    phase: "REVEAL",
    task: "readyToReveal",
    blocksLeft: 94,
    bid: 42,
    lockup: 60,
  },
  {
    name: "tidewater",
    phase: "REVEAL",
    task: "revealDoneWaitingForClose",
    blocksLeft: 1_012,
    bid: 18.5,
    lockup: 25,
    revealed: true,
  },
  { name: "meridian", phase: "BIDDING", task: "readyToBid", blocksLeft: 312, bid: 75, lockup: 120 },
  { name: "driftwood", phase: "OPENING", task: "waitingForBidding", blocksLeft: 23 },
  {
    name: "northwind",
    phase: "CLOSED",
    task: "wonNeedsRegister",
    bid: 130,
    lockup: 150,
    revealed: true,
  },
  { name: "emberly", phase: "CLOSED", task: "lostNeedsRedeem", bid: 9, lockup: 12, revealed: true },
];

/** Names bought on the market, still on their way (the purchase lockup). */
const PURCHASES: [string, number, number][] = [
  ["pixelart", 410.2, 0],
  ["harbor", 320, 142],
];

export function names(): HsdName[] {
  const owned: HsdName[] = OWNED.map(([name, blocksUntilExpire], i) => ({
    name,
    state: "CLOSED",
    height: HEIGHT - RENEWAL_WINDOW + blocksUntilExpire - 2_000 * (i + 1),
    renewal: HEIGHT - RENEWAL_WINDOW + blocksUntilExpire,
    owner: { hash: `${"0".repeat(56)}${(i + 1).toString(16).padStart(8, "0")}`, index: 0 },
    value: hns([64, 210, 18, 95][i]!),
    highest: hns([80, 240, 25, 110][i]!),
    stats: {
      renewalPeriodStart: HEIGHT - RENEWAL_WINDOW + blocksUntilExpire,
      renewalPeriodEnd: HEIGHT + blocksUntilExpire,
      blocksUntilExpire,
      daysUntilExpire: daysFor(blocksUntilExpire),
    },
    registered: true,
    expired: false,
    listing: nameListing(name),
  }));
  const inAuction: HsdName[] = AUCTIONS.filter((a) => a.task !== "lostNeedsRedeem").map((a) => ({
    name: a.name,
    state: a.phase,
    height: HEIGHT - 5_000,
    renewal: null,
    owner: a.task === "wonNeedsRegister" ? { hash: "f".repeat(64), index: 1 } : null,
    value: a.phase === "CLOSED" ? hns(a.bid! * 0.8) : null,
    highest: a.phase === "CLOSED" ? hns(a.bid!) : null,
    stats: countdownStats(a),
    registered: a.phase === "CLOSED" ? false : null,
    expired: false,
  }));
  const bought: HsdName[] = PURCHASES.map(([name, price, blocksRemaining]) => ({
    name,
    state: null,
    height: null,
    renewal: null,
    owner: null,
    registered: true,
    expired: null,
    stats: null,
    shakedex: {
      state: "awaitingFinalize",
      blocksRemaining,
      purchaseId: `purchase-${name}`,
      purchaseDraftId: `draft-purchase-${name}`,
      priceDoos: hns(price),
      lockupBlocks: 288,
      wallClockTiming: true,
    },
  }));
  return [...owned, ...inAuction, ...bought];
}

function countdownStats(a: Auction): HsdName["stats"] {
  const b = a.blocksLeft ?? null;
  switch (a.phase) {
    case "OPENING":
      return { blocksUntilBidding: b, hoursUntilBidding: hoursFor(b!) };
    case "BIDDING":
      return { blocksUntilReveal: b, hoursUntilReveal: hoursFor(b!) };
    case "REVEAL":
      return { blocksUntilClose: b, hoursUntilClose: hoursFor(b!) };
    default:
      return null;
  }
}

/** Names with an auction position (bids, opens) that the Auctions page lists. */
export function auctionPositionNames(): string[] {
  return AUCTIONS.map((a) => a.name);
}

export function renewals(): RenewalsResponse {
  return {
    walletProfileId: PROFILE_ID,
    currentHeight: HEIGHT,
    heightSource: "node",
    expiringSoonThresholdDays: 30,
    names: OWNED.map(([name, blocksUntilExpire]) => ({
      name,
      state: "CLOSED",
      renewalHeight: HEIGHT - RENEWAL_WINDOW + blocksUntilExpire,
      expiresAtHeight: HEIGHT + blocksUntilExpire,
      blocksUntilExpire,
      daysUntilExpire: daysFor(blocksUntilExpire),
      source: "chain",
      expiringSoon: false,
    })),
  };
}

// ── Capabilities ────────────────────────────────────────────────────────────

/** `next_action_for_task` (commands/names.rs), word for word. */
const NEXT_ACTION: Partial<Record<AuctionTaskState, [string | null, string, string]>> = {
  availableToOpen: ["OPEN", "Open Auction", "Start the auction for this name."],
  waitingForBidding: ["WAIT", "Wait for Bidding", "The auction opens for bidding soon."],
  readyToBid: ["BID", "Place Bid", "Place a blind bid before the bidding period ends."],
  readyToReveal: [
    "REVEAL",
    "Reveal Bid",
    "Reveal your bid now, or your lockup can't be reclaimed.",
  ],
  revealDoneWaitingForClose: [
    null,
    "Revealed — waiting for close",
    "Your reveal is confirmed. The auction stays open for reveals until the window closes.",
  ],
  wonNeedsRegister: [
    "REGISTER",
    "Register Name",
    "You won the auction! Register the name to finalize ownership.",
  ],
  lostNeedsRedeem: [
    "REDEEM",
    "Redeem Lockup",
    "Your bid lost. Redeem your reveal coin to reclaim the funds.",
  ],
  ownedNoUrgentAction: [
    "MANAGE",
    "Manage Name",
    "You own this name. Manage DNS, renew, or transfer.",
  ],
};

/** `extract_countdown` (commands/names_pure.rs) labels. */
const COUNTDOWN_LABEL = {
  OPENING: "Bidding opens in",
  BIDDING: "Reveal starts in",
  REVEAL: "Auction closes in",
} as const;

const yes: NameActionCapability = { allowed: true, reason: null };
const no: NameActionCapability = { allowed: false, reason: null };

function base(name: string, phase: string, taskState: AuctionTaskState): NameActionCapabilities {
  const [key, label, reason] = NEXT_ACTION[taskState] ?? [null, null, null];
  return {
    name,
    phase,
    taskState,
    ownsName: false,
    nameIsRegistered: false,
    transferPending: false,
    hasBidCommitment: false,
    hasBidCoin: false,
    hasRevealCoin: false,
    hasOwnerCoin: false,
    revealTxid: null,
    bidValueDoos: null,
    lockupValueDoos: null,
    myBidCount: 0,
    canOpen: no,
    canBid: no,
    canReveal: no,
    canRedeem: no,
    canRegister: no,
    canUpdate: no,
    canTransfer: no,
    canFinalize: no,
    canCancelTransfer: no,
    canRenew: no,
    canRevoke: no,
    nextActionKey: key,
    nextActionLabel: label,
    nextActionReason: reason,
    countdownLabel: null,
    countdownBlocks: null,
    countdownHours: null,
    auctionBiddingBlocks: BIDDING_BLOCKS,
    auctionRevealBlocks: REVEAL_BLOCKS,
    pendingBroadcastAction: null,
    strandedBidCount: 0,
    strandedLockupDoos: 0,
    strandedUncheckedCount: 0,
    redeemWindowClosing: false,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    lockedForSale: false,
  };
}

export function capabilities(name: string): NameActionCapabilities {
  const owned = OWNED.find(([n]) => n === name);
  if (owned) {
    // R27: a name locking or locked for sale has every owner action refused.
    const forSale = LISTINGS.some((l) => l.name === name);
    const owner = forSale ? { allowed: false, reason: LISTING_ACTIONS.nameLockedForSale } : yes;
    return {
      ...base(name, "CLOSED", "ownedNoUrgentAction"),
      ownsName: true,
      nameIsRegistered: true,
      hasOwnerCoin: true,
      lockedForSale: forSale,
      canUpdate: owner,
      canTransfer: owner,
      canRenew: owner,
      canRevoke: owner,
      countdownLabel: "Expires in",
      countdownBlocks: owned[1],
    };
  }
  const a = AUCTIONS.find((x) => x.name === name);
  if (!a) return { ...base(name, "AVAILABLE", "availableToOpen"), canOpen: yes };
  const caps = base(name, a.phase, a.task);
  if (a.phase !== "CLOSED" && a.blocksLeft != null) {
    caps.countdownLabel = COUNTDOWN_LABEL[a.phase];
    caps.countdownBlocks = a.blocksLeft;
    caps.countdownHours = hoursFor(a.blocksLeft);
  }
  if (a.bid != null) {
    caps.hasBidCommitment = true;
    caps.myBidCount = 1;
    caps.bidValueDoos = hns(a.bid);
    caps.lockupValueDoos = hns(a.lockup!);
    caps.hasBidCoin = !a.revealed;
    caps.hasRevealCoin = !!a.revealed;
    if (a.revealed) caps.revealTxid = "9c4e".padEnd(64, "a1");
  }
  switch (a.task) {
    case "readyToBid":
      caps.canBid = yes;
      break;
    case "readyToReveal":
      caps.canReveal = yes;
      break;
    case "wonNeedsRegister":
      caps.ownsName = true;
      caps.hasOwnerCoin = true;
      caps.canRegister = yes;
      break;
    case "lostNeedsRedeem":
      caps.canRedeem = yes;
      caps.redeemableRevealCount = 1;
      caps.redeemableValueDoos = hns(a.lockup!);
      break;
  }
  return caps;
}

// ── Shakedex listings of ours ───────────────────────────────────────────────

/** Our listings: what My listings shows, and which owned names are locked for sale. */
interface Listing {
  name: string;
  state: ShakedexListingState;
  mode: "buyNow" | "reverseAuction";
  /** HNS per step, and the hours from the first step to each (a reverse auction drops daily). */
  prices: number[];
  /** Blocks left in the mainnet transfer lockup, while Locking. */
  lockupLeft?: number;
  withFile: boolean;
  published: boolean;
}

const LISTINGS: Listing[] = [
  {
    name: "copperleaf",
    state: "listed",
    mode: "buyNow",
    prices: [450],
    withFile: true,
    published: true,
  },
  {
    name: "atlas",
    state: "listed",
    mode: "reverseAuction",
    prices: [900, 825, 750, 675, 600],
    withFile: true,
    published: true,
  },
  {
    name: "orchard",
    state: "locking",
    mode: "buyNow",
    prices: [],
    lockupLeft: 201,
    withFile: false,
    published: true,
  },
];
/** Mainnet's 288-block transfer lockup (`noncustodial/network.rs`). */
const TRANSFER_LOCKUP = 288;
/** `sell::LOCK_TIME_UNIT_SECS`. */
const LOCK_TIME_UNIT_SECS = 512;

function nameListing(name: string): ShakedexNameListing | undefined {
  const l = LISTINGS.find((x) => x.name === name);
  return (
    l && { listingId: `listing-${name}`, state: l.state, blocksUntilFinalize: l.lockupLeft ?? null }
  );
}

/** The backend's own actions for a listing in this state (`listing-actions.json`). */
function actionsFor(l: Listing): ShakedexListingActions {
  const c = LISTING_ACTIONS.cases.find(
    (x) => x.state === l.state && x.withFile === l.withFile && x.published === l.published,
  );
  if (!c) throw new Error(`listing-actions.json has no case for ${l.state}`);
  return c.actions as ShakedexListingActions;
}

export function myListings(now: Date): ShakedexMyListing[] {
  // The listings went up a day and a half ago; a reverse auction steps every 24 h.
  const mtp = Math.floor(now.getTime() / 1000) - 3_600;
  const start = mtp - 36 * 3_600;
  return LISTINGS.map((l) => {
    const steps: ShakedexStoredStep[] = l.prices.map((price, i) => ({
      price: hns(price),
      lockTime: start - LOCK_TIME_UNIT_SECS + i * 24 * 3_600,
      signature: "00".repeat(65),
    }));
    const valid = steps.filter((st) => st.lockTime < mtp);
    const ahead = steps.filter((st) => st.lockTime >= mtp);
    const current = valid[valid.length - 1];
    const next = ahead[0];
    const blocksUntilExpire = OWNED.find(([n]) => n === l.name)![1];
    const lockMined = l.state !== "locking" || l.lockupLeft != null;
    return {
      listing: {
        id: `listing-${l.name}`,
        name: l.name,
        mode: l.mode,
        state: l.state,
        lockTxid: lockMined ? "5e7d".padEnd(64, "c3") : null,
        lockVout: lockMined ? 0 : null,
        paymentAddress: l.state === "listed" ? RECEIVE[2]! : null,
        steps,
        expiresAt: steps.length ? start + 365 * 24 * 3_600 : null,
        finalizeDraftId: null,
        cancelDraftId: null,
        cancelFinalizeDraftId: null,
        cancelBlocksRemaining: null,
        blocksUntilFinalize: l.lockupLeft ?? null,
        blocksUntilUnbuyable: l.state === "listed" ? blocksUntilExpire - TRANSFER_LOCKUP : null,
        // `shakedex_jobs::announce` tells the market of a sent lock as a
        // pending listing; the finalize into the lock makes it listed.
        marketStatus: !l.published
          ? null
          : l.state === "listed"
            ? "listed"
            : lockMined
              ? "pending"
              : null,
        marketError: null,
        marketRetryAt: null,
        publish: l.published,
      },
      actions: actionsFor(l),
      prices: steps.length
        ? {
            kind: "at",
            currentPrice: current?.price ?? null,
            nextPrice: next?.price ?? null,
            nextValidInSecs: next ? next.lockTime - mtp : null,
            floorPrice: Math.min(...steps.map((st) => st.price)),
          }
        : { kind: "noSteps" },
      pendingSend: null,
      managedElsewhere: false,
      foreignStepPriceDoos: null,
      soldElsewhere: false,
      cancelledElsewhere: false,
      unbuyableWarnBlocks: 4_320,
      wallClockTiming: true,
    };
  });
}

// ── Balances ────────────────────────────────────────────────────────────────

/** Lockups held by this wallet's open bids and reveals. */
const LOCKED = AUCTIONS.filter((a) => a.task !== "wonNeedsRegister" && a.lockup).reduce(
  (sum, a) => sum + hns(a.lockup!),
  0,
);
const NAME_VALUE = names().reduce((sum, n) => sum + (n.registered && n.value ? n.value : 0), 0);
const LIQUID = hns(12_480.316_204);

export function balances(): WalletBalances {
  return {
    liquidDoos: LIQUID,
    nameControlDoos: NAME_VALUE,
    nameLockupDoos: LOCKED,
    immatureDoos: 0,
    immatureInBlocks: null,
    totalDoos: LIQUID + NAME_VALUE + LOCKED,
  };
}

export function legacyBalance() {
  return { confirmed: LIQUID, unconfirmed: 0, locked_unconfirmed: 0, locked_confirmed: LOCKED };
}

// ── Namebase history (an export imported before the 2026-10-01 shutdown) ───

const NB_EVENTS: [string, string, string | null, Partial<NamebaseHistoryRow>][] = [
  ["2024-03-02T10:14:00Z", "wallet:deposit:1", null, { hnsDoos: hns(15_000) }],
  [
    "2024-03-04T18:40:00Z",
    "auctions:place-bid:4",
    "atlas",
    { bidDoos: hns(64), stakeDoos: hns(90), feeDoos: 1_000_283 },
  ],
  [
    "2024-03-09T07:02:00Z",
    "auctions:place-bid:4",
    "lighthouse",
    { bidDoos: hns(210), stakeDoos: hns(260), feeDoos: 1_000_283 },
  ],
  ["2024-03-19T12:30:00Z", "auctions:charge-fee:0", "atlas", { feeDoos: hns(2.5) }],
  ["2024-03-24T09:11:00Z", "auctions:charge-fee:0", "lighthouse", { feeDoos: hns(2.5) }],
  ["2024-06-11T16:45:00Z", "subdomains:stake-domain:0", "lighthouse", {}],
  [
    "2024-09-27T20:05:00Z",
    "subdomains:confirm-transfer:2",
    "keeper.lighthouse",
    { usdCents: 2_900, hnsDoos: hns(4_832.72125) },
  ],
  [
    "2025-01-15T11:20:00Z",
    "subdomains:confirm-transfer:2",
    "beacon.lighthouse",
    { usdCents: 4_500, hnsDoos: hns(6_102.4) },
  ],
  ["2025-03-04T08:00:00Z", "auctions:charge-renewal-fee:0", "atlas", { feeDoos: hns(1) }],
  ["2025-03-09T08:00:00Z", "auctions:charge-renewal-fee:0", "lighthouse", { feeDoos: hns(1) }],
  [
    "2025-08-21T14:52:00Z",
    "auctions:place-bid:4",
    "orchard",
    { bidDoos: hns(18), stakeDoos: hns(30), feeDoos: 1_000_283 },
  ],
  ["2025-09-03T10:31:00Z", "auctions:charge-fee:0", "orchard", { feeDoos: hns(2.5) }],
];

export function namebaseHistory(): NamebaseHistoryRow[] {
  return NB_EVENTS.map(([createdAt, type, name, amounts], i) => {
    const [family, verb] = type.split(":") as [string, string];
    return {
      id: 188_600_000 + i * 7_919,
      createdAt,
      type,
      family,
      verb,
      name,
      feeDoos: null,
      bidDoos: null,
      stakeDoos: null,
      usdCents: null,
      hnsDoos: null,
      auctionId: null,
      bidId: null,
      saleId: null,
      dataJson: "{}",
      importedAt: "2026-06-12T09:00:00Z",
      ...amounts,
    };
  }).reverse();
}

export function namebaseSummary(): NamebaseHistorySummary {
  const rows = namebaseHistory();
  return {
    eventCount: rows.length,
    nameCount: new Set(rows.map((r) => r.name?.split(".").pop()).filter(Boolean)).size,
    totalFeeDoos: rows.reduce((s, r) => s + (r.feeDoos ?? 0), 0),
    totalUsdCents: rows.reduce((s, r) => s + (r.usdCents ?? 0), 0),
    earliest: rows[rows.length - 1]!.createdAt,
    latest: rows[0]!.createdAt,
  };
}

/** The DNS records of an owned name, as the Records preview on its page reads them. */
export function records(name: string): NameResource {
  if (!OWNED.some(([n]) => n === name)) return { records: [] };
  return {
    records: [
      { type: "NS", ns: `ns1.${name}.` },
      { type: "GLUE4", ns: `ns1.${name}.`, address: "203.0.113.10" },
      { type: "TXT", txt: ["v=spf1 -all"] },
    ],
    ttl: 3600,
  };
}
