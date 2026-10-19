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

import { DEFAULT_SETTINGS } from "./settingsDefaults";
import listingActionsFixture from "../test/fixtures/listing-actions.json";
import type {
  NodeConnectionCheck,
  Settings,
  ShakedexListingActions,
  ShakedexListingMode,
  ShakedexListingPrices,
  ShakedexListingState,
  ShakedexListingSummary,
  ShakedexMyListing,
  ShakedexNameListing,
  ShakedexPendingSend,
  ShakedexStoredStep,
} from "../types";

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

type MockPhase = "AVAILABLE" | "OPENING" | "BIDDING" | "REVEAL" | "CLOSED";

type RevealDraftStatus = "none" | "broadcasted" | "confirmed" | "dropped";

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
  /** Our Shakedex listings, by listing id (see "Shakedex selling" below). */
  listings: Map<string, MockListing>;
  /** Each listing draft's listing and send state, by draft id. */
  listingDrafts: Map<string, MockListingDraft>;
}

const chain: MockChain = {
  height: 100_000,
  auctions: new Map(),
  listings: new Map(),
  listingDrafts: new Map(),
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
  const firstMined = chain.height + 1;
  chain.height += count;
  settleListings(firstMined);
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
  // R27: a name locking or locked for sale has no owner actions here
  // (`commands/names.rs`, `NAME_LOCKED_FOR_SALE`); Sell follows Transfer.
  const listing = openListingOf(name);
  const ownerAction = (allowed: boolean) =>
    listing ? cap(false, ACTIONS.nameLockedForSale) : cap(allowed);
  // R19: until the FINALIZE into the lock, Cancel transfer of the mined lock
  // TRANSFER is the listing's abort (`ListingState::aborts_by_cancel_transfer`);
  // one already staged or sent is not built twice.
  const abortable =
    listing !== undefined &&
    (listing.state === "locking" || listing.state === "readyToFinalize") &&
    listing.lockMinedAt != null &&
    !abortMayStillLand(listing);
  return {
    name,
    phase,
    taskState,
    // A name with an open listing stays ours, its owner actions refused
    // (`derive_name_ownership`), even when the mock never won it (a restore).
    ownsName: won || listing !== undefined,
    lockedForSale: listing !== undefined,
    // The mock's won names are registered: it allows update/transfer on them.
    nameIsRegistered: won || listing !== undefined,
    transferPending: false,
    // A lost auction leaves exactly one reveal to reclaim in this mock — the
    // same condition `canRedeem` below is built from.
    redeemableRevealCount: closed && !won && a.hasBid ? 1 : 0,
    redeemableValueDoos: closed && !won && a.hasBid ? a.bidValueDoos : 0,
    hasBidCommitment: a.hasBid,
    hasBidCoin,
    hasRevealCoin,
    hasOwnerCoin: won,
    // `revealTxid` is the ONLY new field surfaced for the reveal card.
    revealTxid: a.revealTxid,
    // The user's own bid value (doos), so the confirm panel can show it.
    bidValueDoos: a.hasBid ? a.bidValueDoos : null,
    // Latest lockup + how many bids this wallet holds (mock: 0 or 1).
    lockupValueDoos: a.hasBid ? a.lockupDoos : null,
    myBidCount: a.hasBid ? 1 : 0,
    canOpen: cap(phase === "AVAILABLE"),
    // Multi-bid (Namebase-style): another independent bid is allowed for the
    // whole BIDDING window, even after this wallet has already bid.
    canBid: cap(phase === "BIDDING"),
    canReveal: cap(
      phase === "REVEAL" && a.hasBid && hasBidCoin && a.revealStatus !== "broadcasted",
    ),
    canRedeem: cap(closed && !won && a.hasBid),
    canRegister: ownerAction(won),
    canUpdate: ownerAction(won),
    canTransfer: ownerAction(won),
    canFinalize: cap(false),
    canCancelTransfer: cap(abortable),
    canRenew: ownerAction(won),
    canRevoke: ownerAction(won),
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

// ═══════════════════════════════════════════════════════════════════════════
// Shakedex selling
// ═══════════════════════════════════════════════════════════════════════════
//
// Our listings walk the backend's states on the mock chain: a lock draft is
// sent and mined, the transfer lockup runs out (Locking → ReadyToFinalize),
// Finalize & sign stages the FINALIZE into the lock (Finalizing → Listed),
// a cancel is sent and mined (Cancelling → CancelAwaitingFinalize) and its
// FINALIZE brings the name home (CancelFinalizing → Cancelled). Before the
// FINALIZE into the lock, a Cancel transfer of the name (`build_cancel_draft`)
// is the listing's abort: Aborted once it is mined (R19). A draft moves its
// listing on the first block mined after its broadcast. What each row can do,
// and every refusal sentence, comes from the backend's own output
// (`src/test/fixtures/listing-actions.json`, written and checked by
// `shakedex_my_listings_tests::listing_actions_fixture_matches_the_backend`),
// so a row never offers what the backend would not and its words cannot
// drift. No buyer is simulated: Sold, SalePending and Expired are never
// reached here.

/** Regtest's transfer lockup (blocks), compressed like the auction windows above. */
const TRANSFER_LOCKUP = 10;
const DOOS_PER_HNS = 1_000_000;
/** `LISTING_LIFETIME_SECS`: a listing file expires a year after it is written. */
const LISTING_LIFETIME_SECS = 365 * 24 * 3600;
/** `sell::LOCK_TIME_UNIT_SECS`: a Buy Now's lock time is the MTP minus this. */
const LOCK_TIME_UNIT_SECS = 512;

type ListingDraftKind = "lock" | "lockFinalize" | "cancel" | "cancelFinalize" | "abort";

/** Each listing draft's action (`sell.rs` / `cancel.rs` `*_ACTION`). */
const LISTING_DRAFT_ACTION: Record<ListingDraftKind, string> = {
  lock: "shakedex_lock",
  lockFinalize: "shakedex_lock_finalize",
  cancel: "shakedex_cancel",
  cancelFinalize: "shakedex_cancel_finalize",
  // The listing's abort is the name's own Cancel transfer (`build_cancel_draft`).
  abort: "cancel",
};

interface ActionsFixture {
  nameLockedForSale: string;
  cancelTransferPending: string;
  cancelFinalizeWait: string;
  cancelFinalizeWaitOne: string;
  cancelLockupNotCounted: string;
  cases: {
    state: string;
    withFile: boolean;
    published: boolean;
    actions: ShakedexListingActions;
  }[];
}
const ACTIONS = listingActionsFixture as ActionsFixture;

interface MockListingDraft {
  listingId: string;
  kind: ListingDraftKind;
  signed: boolean;
  sent: boolean;
}

interface MockListing {
  id: string;
  name: string;
  mode: ShakedexListingMode;
  state: ShakedexListingState;
  publish: boolean;
  steps: ShakedexStoredStep[];
  /** Our own listing file: written at Finalize & sign or imported. */
  hasFile: boolean;
  expiresAt: number | null;
  /** The block the lock TRANSFER was mined in. */
  lockMinedAt: number | null;
  /** The block the cancel was mined in. */
  cancelMinedAt: number | null;
  lockDraftId: string | null;
  finalizeDraftId: string | null;
  cancelDraftId: string | null;
  cancelFinalizeDraftId: string | null;
  /** The name's Cancel transfer, this listing's abort before the lock (R19). */
  abortDraftId: string | null;
}

/** A median time that moves with the mock chain, ten minutes a block. */
function mockMtp(): number {
  return 1_700_000_000 + (chain.height - 100_000) * MINUTES_PER_BLOCK * 60;
}

/** `sell::buy_now_lock_time`: valid at once at the mock median time. */
function buyNowLockTime(): number {
  return mockMtp() - LOCK_TIME_UNIT_SECS;
}

/** hsd judges a FINALIZE at tip + 1 (`NameParams::blocks_until_finalize`). */
function blocksUntilFinalize(transferHeight: number): number {
  return Math.max(0, transferHeight + TRANSFER_LOCKUP - (chain.height + 1));
}

/** HNS as typed → doos, refused as the backend's price parser would. */
function parsePrice(text: unknown): number {
  const t = String(text ?? "").trim();
  if (!/^\d+(\.\d{1,6})?$/.test(t)) throw new Error(`Invalid input: '${t}' is not an HNS amount`);
  const doos = Math.round(Number(t) * DOOS_PER_HNS);
  if (doos <= 0) throw new Error("Invalid input: a price must be above 0");
  return doos;
}

/** `doos_to_hns_string`. */
function hnsText(doos: number): string {
  return `${Math.floor(doos / DOOS_PER_HNS)}.${String(doos % DOOS_PER_HNS).padStart(6, "0")} HNS`;
}

function mockStep(price: number, lockTime: number): ShakedexStoredStep {
  return { price, lockTime, signature: "00".repeat(65) };
}

function stageListingDraft(l: MockListing, kind: ListingDraftKind, signed: boolean): string {
  const id = `draft-${kind}-${l.id}`;
  chain.listingDrafts.set(id, { listingId: l.id, kind, signed, sent: false });
  return id;
}

function listingDraftSummary(id: string, l: MockListing, kind: ListingDraftKind, status: string) {
  const action = LISTING_DRAFT_ACTION[kind];
  return {
    id,
    walletProfileId: "webqa-profile",
    action,
    status,
    summary: {
      action,
      name: l.name,
      sendTotalDoos: 0,
      feeDoos: 100_000,
      changeDoos: 0,
      inputTotalDoos: 100_000,
      numInputs: 1,
      recipientAddress: null,
      txid: null,
      warnings: [],
    },
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  };
}

function listingById(args?: Record<string, unknown>): MockListing {
  const l = chain.listings.get(String(args?.listingId ?? ""));
  if (!l) throw new Error("Not found: no such listing");
  return l;
}

/** The profile's open listing of `name` (not Cancelled, Sold, Aborted or Expired). */
function openListingOf(name: string): MockListing | undefined {
  const ended: ShakedexListingState[] = ["cancelled", "sold", "aborted", "expired"];
  return [...chain.listings.values()].find((l) => l.name === name && !ended.includes(l.state));
}

function newListing(name: string, mode: ShakedexListingMode, state: ShakedexListingState) {
  let id = `mock-listing-${name}`;
  for (let n = 2; chain.listings.has(id); n++) id = `mock-listing-${name}-${n}`;
  const l: MockListing = {
    id,
    name,
    mode,
    state,
    publish: false,
    steps: [],
    hasFile: false,
    expiresAt: null,
    lockMinedAt: null,
    cancelMinedAt: null,
    lockDraftId: null,
    finalizeDraftId: null,
    cancelDraftId: null,
    cancelFinalizeDraftId: null,
    abortDraftId: null,
  };
  chain.listings.set(id, l);
  return l;
}

/** Each sent listing draft mined in `firstMined` (the first new block). */
function settleListings(firstMined: number): void {
  const sent = (id: string | null) => id != null && chain.listingDrafts.get(id)?.sent === true;
  for (const l of chain.listings.values()) {
    if ((l.state === "locking" || l.state === "readyToFinalize") && sent(l.abortDraftId)) {
      l.state = "aborted";
      continue;
    }
    if (l.state === "locking" && l.lockMinedAt == null && sent(l.lockDraftId)) {
      l.lockMinedAt = firstMined;
    }
    if (
      l.state === "locking" &&
      l.lockMinedAt != null &&
      blocksUntilFinalize(l.lockMinedAt) === 0
    ) {
      l.state = "readyToFinalize";
    } else if (l.state === "finalizing" && sent(l.finalizeDraftId)) {
      l.state = "listed";
    } else if (l.state === "cancelling" && sent(l.cancelDraftId)) {
      l.state = "cancelAwaitingFinalize";
      l.cancelMinedAt = firstMined;
    } else if (l.state === "cancelFinalizing" && sent(l.cancelFinalizeDraftId)) {
      l.state = "cancelled";
    }
  }
}

// ── What a listing can do now, from the backend's fixture ──────────────

/**
 * The listing's actions as `my_listing_from_conn` gives them on mainnet for
 * its state, its stored steps and file (the mock keeps both or neither) and
 * its publish flag; the counts the fixture cannot hold are filled in: the
 * lockup left after a mined cancel (`cancel_finalize_wait`; counted when the
 * mock mines the cancel, as the sync that sees it mined does, so
 * `CANCEL_LOCKUP_NOT_COUNTED` only guards a count it never took), and Finalize &
 * sign while a Cancel transfer may still land (`abort_may_still_land`), before
 * the lockup's own reason.
 */
function actionsOf(l: MockListing): ShakedexListingActions {
  const c = ACTIONS.cases.find(
    (x) => x.state === l.state && x.withFile === l.hasFile && x.published === l.publish,
  );
  if (!c) throw new Error(`listing-actions.json has no case for ${l.state}`);
  const actions = structuredClone(c.actions);
  const wait = cancelBlocksRemaining(l);
  if (actions.finalizeCancel.allowed && wait === null) {
    actions.finalizeCancel = { allowed: false, reason: ACTIONS.cancelLockupNotCounted };
  } else if (actions.finalizeCancel.allowed && wait !== null && wait > 0) {
    actions.finalizeCancel = {
      allowed: false,
      reason:
        wait === 1
          ? ACTIONS.cancelFinalizeWaitOne
          : ACTIONS.cancelFinalizeWait.replace("{n}", String(wait)),
    };
  }
  // `finalize_and_sign_refusal`: the pending abort is named first, in
  // Locking as in ReadyToFinalize.
  if ((l.state === "locking" || l.state === "readyToFinalize") && abortMayStillLand(l)) {
    actions.finalizeAndSign = { allowed: false, reason: ACTIONS.cancelTransferPending };
  }
  return actions;
}

/**
 * The listing is linked to a Cancel transfer, staged or sent: once mined the
 * listing is Aborted, and a discarded one is unlinked (`discardListingDraft`).
 */
function abortMayStillLand(l: MockListing): boolean {
  return l.abortDraftId != null;
}

function refuse(reason: string | null): void {
  if (reason !== null) throw new Error(`Invalid input: ${reason}`);
}

function cancelBlocksRemaining(l: MockListing): number | null {
  return l.state === "cancelAwaitingFinalize" && l.cancelMinedAt != null
    ? blocksUntilFinalize(l.cancelMinedAt)
    : null;
}

function listingSummary(l: MockListing): ShakedexListingSummary {
  const finalized = l.state !== "locking" && l.state !== "readyToFinalize";
  return {
    id: l.id,
    name: l.name,
    mode: l.mode,
    state: l.state,
    lockTxid: l.lockMinedAt != null || l.state === "restored" ? "ab".repeat(32) : null,
    lockVout: l.lockMinedAt != null || l.state === "restored" ? 0 : null,
    paymentAddress: finalized && l.state !== "restored" ? "hs1qpayment...(mock)" : null,
    steps: l.steps,
    expiresAt: l.expiresAt,
    finalizeDraftId: l.finalizeDraftId,
    cancelDraftId: l.cancelDraftId,
    cancelFinalizeDraftId: l.cancelFinalizeDraftId,
    cancelBlocksRemaining: cancelBlocksRemaining(l),
    blocksUntilFinalize: lockupLeft(l),
    marketStatus: null,
    marketError: null,
    marketRetryAt: null,
    publish: l.publish,
  };
}

/**
 * `ListingPrices` at the mock median time (`sell::step_view`): the current
 * price is the cheapest step valid now (`template::current_step_index`); the
 * next is, of the steps not valid yet and cheaper than that, the one valid
 * soonest, the cheapest of those at one time (`template::next_step_index`).
 */
function listingPrices(l: MockListing): ShakedexListingPrices {
  if (l.steps.length === 0) return { kind: "noSteps" };
  const mtp = mockMtp();
  const cheapest = (steps: ShakedexStoredStep[]) =>
    steps.reduce<ShakedexStoredStep | null>((m, s) => (m && m.price <= s.price ? m : s), null);
  const current = cheapest(l.steps.filter((s) => s.lockTime < mtp));
  const ahead = l.steps.filter(
    (s) => s.lockTime >= mtp && (current === null || s.price < current.price),
  );
  const soonest = Math.min(...ahead.map((s) => s.lockTime));
  const next = cheapest(ahead.filter((s) => s.lockTime === soonest));
  return {
    kind: "at",
    currentPrice: current?.price ?? null,
    nextPrice: next?.price ?? null,
    nextValidInSecs: next ? next.lockTime - mtp : null,
    floorPrice: Math.min(...l.steps.map((s) => s.price)),
  };
}

/** `pending_send`: the state's own draft while it is not sent. */
function pendingSend(l: MockListing): ShakedexPendingSend | null {
  const id =
    l.state === "locking"
      ? l.lockDraftId
      : l.state === "finalizing"
        ? l.finalizeDraftId
        : l.state === "cancelling"
          ? l.cancelDraftId
          : l.state === "cancelFinalizing"
            ? l.cancelFinalizeDraftId
            : null;
  const d = id ? chain.listingDrafts.get(id) : undefined;
  if (!id || !d || d.sent) return null;
  return { draftId: id, action: LISTING_DRAFT_ACTION[d.kind], signed: d.signed, discardable: true };
}

function myListing(l: MockListing): ShakedexMyListing {
  return {
    listing: listingSummary(l),
    actions: actionsOf(l),
    prices: listingPrices(l),
    pendingSend: pendingSend(l),
  };
}

/**
 * The `listing` of `name`'s Owned Names row (`read_shakedex_listing_names`):
 * its open listing, with the lockup left once the lock is mined.
 */
function nameListing(name: string): ShakedexNameListing | undefined {
  const l = openListingOf(name);
  if (!l) return undefined;
  return {
    listingId: l.id,
    state: l.state,
    blocksUntilFinalize: lockupLeft(l),
  };
}

/**
 * The lockup left on a Locking listing whose lock is mined, for its Owned
 * Names row and its My listings row alike (`locking_blocks_until_finalize`).
 */
function lockupLeft(l: MockListing): number | null {
  return l.state === "locking" && l.lockMinedAt != null ? blocksUntilFinalize(l.lockMinedAt) : null;
}

/** The listing file this wallet writes (`listing_file::write_listing_file`), in short. */
function listingFileJson(l: MockListing): string {
  return JSON.stringify({
    name: l.name,
    feeAddr: null,
    data: l.steps.map((s) => ({ price: s.price, lockTime: s.lockTime, fee: 0 })),
  });
}

/** A listing draft is sent: the next mined block moves its listing. */
function sendListingDraft(id: string): boolean {
  const d = chain.listingDrafts.get(id);
  if (!d) return false;
  d.sent = true;
  return true;
}

/**
 * `delete_tx_draft` of a listing draft never sent puts its listing back
 * (`db::queries::delete_tx_draft`): a Locking listing goes with its lock; an
 * unsent FINALIZE into the lock takes its steps (ReadyToFinalize); an unsent
 * cancel leaves the listing Listed (Restored without a file); an unsent
 * cancel FINALIZE leaves it awaiting its finalize; an unsent Cancel transfer
 * loses its link to the listing it would have aborted.
 */
function discardListingDraft(id: string): void {
  const d = chain.listingDrafts.get(id);
  if (!d || d.sent) return;
  chain.listingDrafts.delete(id);
  const l = chain.listings.get(d.listingId);
  if (!l) return;
  if (d.kind === "lock" && l.state === "locking") {
    chain.listings.delete(l.id);
  } else if (d.kind === "lockFinalize" && l.state === "finalizing") {
    l.state = "readyToFinalize";
    l.steps = [];
    l.hasFile = false;
    l.expiresAt = null;
    l.finalizeDraftId = null;
  } else if (d.kind === "cancel" && l.state === "cancelling") {
    l.state = l.hasFile ? "listed" : "restored";
    l.cancelDraftId = null;
  } else if (d.kind === "cancelFinalize" && l.state === "cancelFinalizing") {
    l.state = "cancelAwaitingFinalize";
    l.cancelFinalizeDraftId = null;
  } else if (d.kind === "abort") {
    l.abortDraftId = null;
  }
}

const sellingHandlers: Record<string, Handler> = {
  shakedex_list_listings: () => [...chain.listings.values()].reverse().map(myListing),

  shakedex_build_lock_draft: (args) => {
    const name = String(args?.name ?? "");
    if (openListingOf(name)) throw new Error(`Invalid input: '${name}' is already locked for sale`);
    const l = newListing(name, (args?.mode as ShakedexListingMode) ?? "buyNow", "locking");
    l.publish = args?.publish === true;
    l.lockDraftId = stageListingDraft(l, "lock", false);
    return listingDraftSummary(l.lockDraftId, l, "lock", "draft");
  },

  shakedex_finalize_and_sign: (args) => {
    const l = listingById(args);
    refuse(actionsOf(l).finalizeAndSign.reason);
    const prices = (args?.prices as { price: string }[] | undefined) ?? [];
    const schedule = (args?.schedule ?? null) as {
      startPrice: string;
      floorPrice: string;
      steps: number;
      intervalSecs: number;
    } | null;
    // R19: a Buy Now's lock time, valid at once; a reverse auction's first
    // step has the same and each next step its interval later, rounded up to
    // R3's 512 s unit (`sell::reverse_schedule`), never valid early.
    const now = buyNowLockTime();
    if (l.mode === "buyNow") {
      if (prices.length !== 1 || schedule) {
        throw new Error("Invalid input: a Buy Now listing has one price and no schedule");
      }
      l.steps = [mockStep(parsePrice(prices[0]!.price), now)];
    } else {
      if (!schedule) {
        throw new Error(
          "Invalid input: a reverse auction needs its schedule: a start price, a floor, the number of steps and the time between them",
        );
      }
      if (prices.length > 0) {
        throw new Error("Invalid input: a reverse auction's prices come from its schedule");
      }
      const start = parsePrice(schedule.startPrice);
      const floor = parsePrice(schedule.floorPrice);
      const n = Math.floor(schedule.steps);
      if (!(n >= 2 && n <= 1000) || floor >= start || schedule.intervalSecs < 900) {
        throw new Error("Invalid input: reverse auction: this schedule cannot be signed");
      }
      // Evenly from the start down to the floor, one interval apart.
      l.steps = Array.from({ length: n }, (_, i) =>
        mockStep(
          start - Math.floor(((start - floor) * i) / (n - 1)),
          i === 0
            ? now
            : Math.ceil((now + i * schedule.intervalSecs) / LOCK_TIME_UNIT_SECS) *
                LOCK_TIME_UNIT_SECS,
        ),
      );
    }
    l.hasFile = true;
    l.expiresAt = now + LISTING_LIFETIME_SECS;
    l.state = "finalizing";
    l.finalizeDraftId = stageListingDraft(l, "lockFinalize", true);
    return listingSummary(l);
  },

  shakedex_cancel_listing: (args) => {
    const l = listingById(args);
    refuse(actionsOf(l).cancel.reason);
    l.state = "cancelling";
    l.cancelDraftId = stageListingDraft(l, "cancel", true);
    return listingDraftSummary(l.cancelDraftId, l, "cancel", "signed");
  },

  shakedex_finalize_cancel: (args) => {
    const l = listingById(args);
    refuse(actionsOf(l).finalizeCancel.reason);
    l.state = "cancelFinalizing";
    l.cancelFinalizeDraftId = stageListingDraft(l, "cancelFinalize", false);
    return listingDraftSummary(l.cancelFinalizeDraftId, l, "cancelFinalize", "draft");
  },

  shakedex_lower_price: (args) => {
    const l = listingById(args);
    refuse(actionsOf(l).lowerPrice.reason);
    const price = parsePrice(args?.price);
    const p = listingPrices(l);
    const current = p.kind === "at" ? (p.currentPrice ?? p.floorPrice) : 0;
    // `sell::check_lower_price`.
    if (price >= current) {
      throw new Error(
        `Invalid input: ${hnsText(price)} is not below the current price ${hnsText(current)}: a price can only be lowered; raising it needs a cancel`,
      );
    }
    // R26: the lowered step joins the schedule, valid at once.
    l.steps = [...l.steps, mockStep(price, buyNowLockTime())];
    return listingSummary(l);
  },

  shakedex_restore_lock: (args) => {
    const name = String(args?.name ?? "");
    if (openListingOf(name)) {
      throw new Error(
        `Invalid input: '${name}' is already tracked by one of this wallet's listings`,
      );
    }
    // R32: the mode is recorded as Buy Now until its file is imported.
    return listingSummary(newListing(name, "buyNow", "restored"));
  },

  shakedex_publish_listing: (args) => {
    const l = listingById(args);
    refuse(actionsOf(l).publish.reason);
    l.publish = true;
    return listingSummary(l);
  },

  shakedex_export_listing_file: (args) => {
    const l = listingById(args);
    refuse(actionsOf(l).export.reason);
    return listingFileJson(l);
  },

  shakedex_save_listing_file: (args) => {
    refuse(actionsOf(listingById(args)).export.reason);
    return null;
  },

  shakedex_import_own_listing_file: (args) => importOwnFile(args?.text),
  shakedex_import_own_listing_from_file: () => importOwnFile(null),
};

/**
 * R32: our own saved listing file makes a Restored lock Listed. Pasted text
 * names its listing; a file chosen on disk goes to the only Restored lock
 * (the browser has no file to read).
 */
function importOwnFile(text: unknown): ShakedexListingSummary {
  let name: string | null = null;
  if (typeof text === "string") {
    try {
      name = (JSON.parse(text) as { name?: string }).name ?? null;
    } catch {
      throw new Error("Invalid input: this is not a listing file");
    }
  }
  const l = [...chain.listings.values()].find(
    (x) => actionsOf(x).importFile.allowed && (name === null || x.name === name),
  );
  if (!l) {
    // No Restored lock to take it: the import's own refusal for any other state.
    const listed = ACTIONS.cases.find((c) => c.state === "listed")!;
    throw new Error(`Invalid input: ${listed.actions.importFile.reason}`);
  }
  l.state = "listed";
  l.hasFile = true;
  l.steps = [mockStep(DOOS_PER_HNS, buyNowLockTime())];
  l.expiresAt = mockMtp() + LISTING_LIFETIME_SECS;
  return listingSummary(l);
}

function demoListing(
  name: string,
  kind: "buyNow" | "reverseAuction",
  currentPrice: number,
  nextPrice: number | null,
  nextValidInSecs: number | null,
  floorPrice: number,
) {
  return {
    listingJson: "{}",
    name,
    verdict: {
      verdict: "buyable",
      currentStep: 0,
      nextStep: nextPrice === null ? null : 1,
      lockValue: currentPrice,
      nameHeight: 100,
      expiryEnd: 100_000,
      warnExpiry: false,
      mtp: 0,
      tip: 0,
    },
    kind,
    currentPrice,
    nextPrice,
    nextValidInSecs,
    floorPrice,
    steps: [],
    expiresAt: null,
    own: false,
  };
}

/** A Cancel transfer of a name that is no listing's abort. */
function cancelDraft() {
  return {
    id: "draft-cancel-001",
    walletProfileId: "webqa-profile",
    action: "cancel",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  };
}

/** Settings changed with `update_setting`, over the defaults. */
const savedSettings: Record<string, string> = {};

const handlers: Record<string, Handler> = {
  // ── Settings ──────────────────────────────────────────────────────────
  // Built from the typed defaults rather than hand-listed: this map used to
  // omit settings the real backend always returns (`allow_remote_broadcast`
  // among them), so a screen that reads one saw `undefined` in browser QA and
  // nowhere else. Spreading the typed object means a new setting cannot be
  // forgotten here.
  get_settings: (): Settings => ({
    ...DEFAULT_SETTINGS,
    onboarding_complete: "true",
    ...savedSettings,
  }),

  // Kept for the page's life, so a setting (the mainnet selling flag among
  // them) can be turned on in browser QA.
  update_setting: (args) => {
    savedSettings[String(args?.key)] = String(args?.value);
    return null;
  },

  // Browser QA has no node to probe. Answer as a reachable, synced, matching
  // node so "Test connection" completes instead of falling through to the
  // unknown-command warning and returning null, which the caller reads as a
  // failure it cannot explain.
  check_node_connection: (): NodeConnectionCheck => ({
    reachable: true,
    height: 100_000,
    headers: 100_000,
    synced: true,
    network: "regtest",
    networkMatches: true,
    error: null,
  }),

  // ── Daemon control ────────────────────────────────────────────────────
  is_background_sync_enabled: () => true,
  set_background_sync_enabled: () => null,
  is_daemon_alive: () => false,

  // ── Updates ───────────────────────────────────────────────────────────
  // Browser QA has no real updater; report a fixed version and "up to date".
  current_version: () => __APP_VERSION__,
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
    immatureDoos: 0,
    immatureInBlocks: null,
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
    // R19: each name's listing; a name in our lock that is not one of ours
    // otherwise gets a row of its own at the lock address.
    const names: Record<string, unknown>[] = [...staticNames, ...engineNames];
    for (const n of names) {
      const listing = nameListing(n.name as string);
      if (listing) n.listing = listing;
    }
    for (const l of chain.listings.values()) {
      const listing = nameListing(l.name);
      if (!listing || names.some((n) => n.name === l.name)) continue;
      names.push({
        name: l.name,
        state: null,
        height: null,
        renewal: null,
        owner: null,
        owner_address: "hs1qlock...(mock)",
        registered: true,
        expired: null,
        stats: null,
        listing,
      });
    }
    return names;
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
  get_name_action_capabilities: (args) => buildCapabilities((args?.name as string) ?? "unknown"),

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
  brute_force_recover_bid: () => ({
    name: "example",
    address: "rs1q...",
    bidValueDoos: 5_000_000,
    lockupValueDoos: 10_000_000,
    tier: "round",
  }),
  save_bid_backup: () => null,

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

  sign_tx_draft: (args) => {
    const listingDraft = chain.listingDrafts.get(args?.draftId as string);
    if (listingDraft) listingDraft.signed = true;
    return {
      id: (args?.draftId as string) ?? "draft-mock-001",
      walletProfileId: "webqa-profile",
      action: listingDraft ? LISTING_DRAFT_ACTION[listingDraft.kind] : "send_hns",
      status: "signed",
      summary: null,
      errorMessage: null,
      txid: null,
      confirmationHeight: null,
      createdAt: new Date().toISOString(),
    };
  },

  broadcast_tx_draft: (args) => {
    const id = (args?.draftId as string) ?? "draft-mock-001";
    // A listing draft moves its listing on the next mined block.
    if (sendListingDraft(id)) {
      return { draftId: id, txid: "cd".repeat(32), status: "broadcasted" };
    }
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

  delete_tx_draft: (args) => {
    discardListingDraft(args?.draftId as string);
    return null;
  },

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

  build_cancel_draft: (args) => {
    const l = openListingOf(String(args?.name ?? ""));
    if (l && (l.state === "locking" || l.state === "readyToFinalize") && l.lockMinedAt != null) {
      // R19: the link commits with the draft; the listing becomes Aborted
      // once this cancel is mined.
      l.abortDraftId = stageListingDraft(l, "abort", false);
      return listingDraftSummary(l.abortDraftId, l, "abort", "draft");
    }
    return cancelDraft();
  },

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

  build_batch_finalize_draft: () => ({
    id: "draft-batch-finalize-001",
    walletProfileId: "webqa-profile",
    action: "batch-finalize",
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

  // ── Shakedex market ───────────────────────────────────────────────────
  shakedex_list_market: () => ({
    rows: [
      demoListing("dexreviews", "buyNow", 250_000_000, null, null, 250_000_000),
      demoListing("coffeeshop", "reverseAuction", 900_000_000, 800_000_000, 21_600, 600_000_000),
    ],
    hidden: {
      soldOrCancelled: 28,
      failedVerification: 4,
      expiresBeforeFinalize: 0,
      notYetValid: 0,
      couldNotCheck: 0,
    },
    verified: true,
    networkHasMarket: true,
    hiddenRows: [
      { name: "oldname", reason: { kind: "soldOrCancelled" } },
      { name: "badname", reason: { kind: "failedVerification", reason: "signature mismatch" } },
    ],
    page: 1,
    pageCount: 1,
  }),
  shakedex_import_listing: () =>
    demoListing("imported", "buyNow", 100_000_000, null, null, 100_000_000),
  shakedex_preview_purchase: () => null,
  shakedex_build_purchase_draft: () => null,
  shakedex_build_purchase_finalize_draft: () => null,
  ...sellingHandlers,

  // ── Watchlist ─────────────────────────────────────────────────────────
  add_to_watchlist: () => null,
  remove_from_watchlist: () => null,
  list_watchlist: () => [],
  is_watched: () => false,
  get_watchlist_status: () => [],
  update_watchlist_tags: () => null,
  export_watchlist_csv: () => 0,
  import_watchlist_csv: () => ({ imported: 0, skipped: 0, errors: [] }),

  // ── Paid swap offers (seller-side tracking) ─────────────────────────
  create_paid_swap_offer: () => null,
  get_paid_swap_offer: () => null,
  remove_paid_swap_offer: () => null,
  claim_paid_transfer: () => ({
    verified: true,
    paidDoos: 1_000_000,
    confirmations: 1,
  }),

  // ── DNS resource ──────────────────────────────────────────────────────
  get_resource: () => ({
    name: "example",
    state: "CLOSED",
    height: 100000,
    renewal: 100000,
    stats: { daysUntilExpire: 300, blocksUntilExpire: 43200 },
    data: {
      records: [{ type: "NS", ns: "ns1.example." }],
    },
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
  console.warn(`[browser QA] No mock handler for invoke("${cmd}") — returning null.`);
  return null as unknown as T;
}

/** Test-only: reset the engine to a known-empty state. Do NOT call from app code. */
export function __resetChainForTests(seedHeight = 100_000): void {
  chain.height = seedHeight;
  chain.auctions.clear();
  chain.listings.clear();
  chain.listingDrafts.clear();
  for (const key of Object.keys(savedSettings)) delete savedSettings[key];
}

/** Test-only: mine `n` blocks (same as the dev control / webqa_mine_blocks). */
export function __mineForTests(n = 1): number {
  return mineBlocks(n);
}
