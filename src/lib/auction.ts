// Vickrey-auction phase derivation + countdown helpers.
//
// hsd `getnameinfo` reports a `state` (OPENING / BIDDING / REVEAL / CLOSED …)
// and, in `stats`, the block/time distance to the next phase. We turn those into
// a UI badge + a human countdown and a recommended next action. All inputs are
// optional/nullable — the explorer path may omit the auction stats entirely, so
// every function degrades to "unknown" rather than throwing.

import type { HsdNameStats, AuctionTaskState, NameActionCapabilities } from "../types";

export type AuctionPhase =
  | "AVAILABLE"
  | "OPENING"
  | "BIDDING"
  | "REVEAL"
  | "CLOSED"
  | "REVOKED"
  | "TRANSFER"
  | "OTHER";

export interface PhaseBadge {
  phase: AuctionPhase;
  label: string;
  variant: "default" | "success" | "warning" | "error" | "info";
}

/** Map a raw hsd `state` string to a phase + display badge. */
export function auctionPhase(state: string | null | undefined): PhaseBadge {
  switch ((state ?? "").toUpperCase()) {
    case "OPENING":
      return { phase: "OPENING", label: "Opening", variant: "info" };
    case "BIDDING":
      return { phase: "BIDDING", label: "Bidding", variant: "warning" };
    case "REVEAL":
      return { phase: "REVEAL", label: "Reveal", variant: "warning" };
    case "CLOSED":
      return { phase: "CLOSED", label: "Closed", variant: "success" };
    case "REVOKED":
      return { phase: "REVOKED", label: "Revoked", variant: "error" };
    case "TRANSFER":
      return { phase: "TRANSFER", label: "Transfer", variant: "info" };
    case "":
    case "AVAILABLE":
      return { phase: "AVAILABLE", label: "Available", variant: "default" };
    default:
      return { phase: "OTHER", label: state ?? "—", variant: "default" };
  }
}

export interface PhaseCountdown {
  /** What the countdown is measuring, e.g. "Reveal starts in". */
  label: string;
  blocks: number;
  hours: number | null;
}

/**
 * The distance to this name's next phase transition, picked from `stats` by the
 * current phase. Returns null when the relevant stat isn't present (e.g. an
 * explorer payload without auction stats, or a terminal/unknown state).
 */
export function nextTransition(
  state: string | null | undefined,
  stats: HsdNameStats | null | undefined,
): PhaseCountdown | null {
  if (!stats) return null;
  const { phase } = auctionPhase(state);
  const pick = (
    label: string,
    blocks: number | null | undefined,
    hours: number | null | undefined,
  ): PhaseCountdown | null =>
    blocks == null ? null : { label, blocks, hours: hours ?? null };

  switch (phase) {
    case "OPENING":
      return pick("Bidding opens in", stats.blocksUntilBidding, stats.hoursUntilBidding);
    case "BIDDING":
      return pick("Reveal starts in", stats.blocksUntilReveal, stats.hoursUntilReveal);
    case "REVEAL":
      return pick("Auction closes in", stats.blocksUntilClose, stats.hoursUntilClose);
    case "CLOSED":
      return pick("Expires in", stats.blocksUntilExpire, null);
    default:
      return null;
  }
}

/** "12 blocks (~2h)" / "1 block (~10m)" — compact countdown for a badge/line. */
export function formatCountdown(c: PhaseCountdown): string {
  const blocks = `${c.blocks} block${c.blocks === 1 ? "" : "s"}`;
  if (c.hours == null) return blocks;
  const time =
    c.hours >= 1
      ? `~${Math.round(c.hours)}h`
      : `~${Math.max(1, Math.round(c.hours * 60))}m`;
  return `${blocks} (${time})`;
}

/**
 * The action most relevant to the current phase — used to highlight one button
 * in the actions modal. Other actions stay available under "All actions".
 */
export function recommendedAction(
  state: string | null | undefined,
): { key: string; label: string; hint: string } | null {
  switch (auctionPhase(state).phase) {
    case "AVAILABLE":
      return { key: "OPEN", label: "Open", hint: "Start the auction for this name." };
    case "BIDDING":
      return { key: "BID", label: "Bid", hint: "Place a blind bid before bidding closes." };
    case "REVEAL":
      return {
        key: "REVEAL",
        label: "Reveal",
        hint: "Reveal your bid now, or your lockup can't be reclaimed.",
      };
    case "CLOSED":
      return {
        key: "REGISTER",
        label: "Register",
        hint: "You can set DNS records / register the name.",
      };
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Acquisition-flow guidance
// ---------------------------------------------------------------------------

/** One phase's entry in `AUCTION_PHASE_GUIDE`. */
export interface AuctionPhaseGuide {
  title: string;
  description: string;
  action: string;
  actionHint: string;
}

/** Short human description of what each auction step does. */
export const AUCTION_PHASE_GUIDE: Record<AuctionPhase, AuctionPhaseGuide | null> = {
  AVAILABLE: {
    title: "Open Auction",
    description:
      "Start a Vickrey auction for this name. The name enters a ~1-week bidding period.",
    action: "Open",
    actionHint: "Opens the auction on-chain. Costs a small network fee.",
  },
  OPENING: {
    title: "Waiting for Bidding",
    description:
      "The auction is in the pre-bid opening period. Bidding will start automatically.",
    action: "",
    actionHint: "No action needed — bidding opens soon.",
  },
  BIDDING: {
    title: "Place a Bid",
    description:
      "Place a blind bid. Your bid amount is encrypted — nobody sees it until the reveal phase.",
    action: "Bid",
    actionHint: "Enter your bid in HNS. You'll lock up at least this much.",
  },
  REVEAL: {
    title: "Reveal Your Bid",
    description:
      "Reveal your bid to the network. If you don't reveal, you lose your locked funds.",
    action: "Reveal",
    actionHint: "Reveals your bid. Unrevealed bids forfeit their lockup.",
  },
  CLOSED: {
    title: "Register Name",
    description:
      "The auction is over. Register the name to make it yours and set DNS records.",
    action: "Register",
    actionHint: "Finalizes ownership on-chain.",
  },
  REVOKED: null,
  TRANSFER: null,
  OTHER: null,
};

/** The guidance payload returned by [`auctionGuidance`]. */
export interface AuctionGuidance {
  phase: AuctionPhase;
  badge: PhaseBadge;
  title: string;
  description: string;
  action: string;
  actionHint: string;
  countdown: PhaseCountdown | null;
}

/**
 * One-call acquisition guidance for a name's current state.
 *
 * Returns `null` when the phase has no acquisition guidance (REVOKED / TRANSFER
 * / OTHER), letting the caller hide the guided panel entirely.
 */
export function auctionGuidance(
  state: string | null | undefined,
  stats: HsdNameStats | null | undefined,
): AuctionGuidance | null {
  const badge = auctionPhase(state);
  const guide = AUCTION_PHASE_GUIDE[badge.phase];
  if (!guide) return null;
  return {
    phase: badge.phase,
    badge,
    title: guide.title,
    description: guide.description,
    action: guide.action,
    actionHint: guide.actionHint,
    countdown: nextTransition(state, stats),
  };
}

// ---------------------------------------------------------------------------
// Task-driven auction UX helpers (capabilities-based)
// ---------------------------------------------------------------------------


/**
 * Human-readable label for a task state — shown as the primary badge/CTA
 * in the auction list and modal.
 */
export function taskStateLabel(state: AuctionTaskState): string {
  switch (state) {
    case "availableToOpen":
      return "Available to Open";
    case "waitingForBidding":
      return "Waiting for Bidding";
    case "readyToBid":
      return "Ready to Bid";
    case "readyToReveal":
      return "Ready to Reveal";
    case "wonNeedsRegister":
      return "Won — Register Now";
    case "lostNeedsRedeem":
      return "Lost — Redeem Now";
    case "transferPendingFinalize":
      return "Transfer — Finalize";
    case "ownedNoUrgentAction":
      return "Owned";
    case "expiringSoon":
      return "Expiring Soon — Renew";
    case "unavailableOther":
      return "Unavailable";
  }
}

/**
 * Badge variant for a task state — used in both the auctions list and modal.
 */
export function taskStateBadgeVariant(
  state: AuctionTaskState,
): "default" | "success" | "warning" | "error" | "info" {
  switch (state) {
    case "availableToOpen":
      return "info";
    case "waitingForBidding":
      return "default";
    case "readyToBid":
      return "warning";
    case "readyToReveal":
      return "warning";
    case "wonNeedsRegister":
      return "success";
    case "lostNeedsRedeem":
      return "error";
    case "transferPendingFinalize":
      return "info";
    case "ownedNoUrgentAction":
      return "success";
    case "expiringSoon":
      // Missing a renewal loses the name forever — treat as an error-level alert.
      return "error";
    case "unavailableOther":
      return "default";
  }
}

/**
 * Urgency text for a task state — surfaces in WalletView alerts.
 * Returns null when there's no urgency to display.
 */
export function taskStateUrgency(state: AuctionTaskState): string | null {
  switch (state) {
    case "readyToReveal":
      return "Reveal your bid before the window closes or your lockup can't be reclaimed.";
    case "wonNeedsRegister":
      return "You won the auction! Register to finalize ownership and set DNS.";
    case "lostNeedsRedeem":
      return "Your bid lost. Redeem your reveal coin to reclaim the funds.";
    case "transferPendingFinalize":
      return "This name is being transferred. Finalize to complete.";
    case "expiringSoon":
      return "This name is close to expiry. Renew now — an expired Handshake name is lost forever.";
    default:
      return null;
  }
}

/**
 * Build a display summary from the backend capability response.
 */
export interface AuctionTaskSummary {
  taskState: AuctionTaskState;
  label: string;
  variant: "default" | "success" | "warning" | "error" | "info";
  urgency: string | null;
  nextActionKey: string | null;
  nextActionLabel: string | null;
  nextActionReason: string | null;
  countdownLabel: string | null;
  countdownBlocks: number | null;
  countdownHours: number | null;
}

/**
 * Map backend capabilities to a frontend display summary.
 */
export function taskSummaryFromCapabilities(
  caps: NameActionCapabilities | null | undefined,
): AuctionTaskSummary | null {
  if (!caps) return null;
  return {
    taskState: caps.taskState,
    label: taskStateLabel(caps.taskState),
    variant: taskStateBadgeVariant(caps.taskState),
    urgency: taskStateUrgency(caps.taskState),
    nextActionKey: caps.nextActionKey,
    nextActionLabel: caps.nextActionLabel,
    nextActionReason: caps.nextActionReason,
    countdownLabel: caps.countdownLabel,
    countdownBlocks: caps.countdownBlocks,
    countdownHours: caps.countdownHours,
  };
}

/**
 * Sort priority for task states in the Auctions list — lower sorts first.
 * Time-critical states come first so they can't be buried under a long list
 * of "waiting" rows: an unrevealed bid forfeits its entire lockup, a won
 * auction needs registering, and a lost bid's lockup sits unredeemed until
 * the user acts.
 */
export function taskStateUrgencyRank(state: AuctionTaskState): number {
  switch (state) {
    case "readyToReveal":
      return 0;
    case "wonNeedsRegister":
      return 1;
    case "lostNeedsRedeem":
      return 2;
    case "expiringSoon":
      return 3;
    default:
      return 4;
  }
}

/** Result of [`validateBidInputs`] — everything a bid form needs to render
 * inline errors and gate its submit button. */
export interface BidInputValidation {
  bidValid: boolean;
  lockupValid: boolean;
  /** True only when both fields are valid AND bid ≤ lockup. */
  formValid: boolean;
  bidError: string | null;
  lockupError: string | null;
}

/**
 * Client-side bid-form validation (F4 fix): `0 < bid ≤ lockup`, and both
 * inputs must parse as finite numbers (an empty field, "abc", "-1", "NaN",
 * "Infinity", etc. are all rejected). An unrevealed/invalid bid forfeits real
 * locked funds, so this is fast UI feedback ahead of the backend's
 * authoritative check at build time.
 *
 * Pulled out as a pure function (rather than inlined per-component state) so
 * the guided and advanced bid forms — still duplicated until Task 13 — share
 * the exact same rule and can't drift, and so the rule is unit-testable
 * without going through an `<input type="number">`, whose own browser-level
 * sanitization would otherwise make some invalid strings unreachable via a
 * simulated DOM change event.
 */
export function validateBidInputs(bidHns: string, lockupHns: string): BidInputValidation {
  const bidNum = Number(bidHns);
  const lockupNum = Number(lockupHns);
  const bidEntered = bidHns.trim() !== "";
  const lockupEntered = lockupHns.trim() !== "";
  const bidValid = bidEntered && Number.isFinite(bidNum) && bidNum > 0;
  const lockupValid = lockupEntered && Number.isFinite(lockupNum) && lockupNum > 0;
  const bidExceedsLockup = bidValid && lockupValid && bidNum > lockupNum;
  const bidError = bidEntered && !bidValid ? "Enter a bid amount greater than 0" : null;
  const lockupError = !lockupEntered
    ? null
    : !lockupValid
      ? "Enter a lockup amount greater than 0"
      : bidExceedsLockup
        ? "Lockup must be at least the bid amount"
        : null;
  return {
    bidValid,
    lockupValid,
    formValid: bidValid && lockupValid && !bidExceedsLockup,
    bidError,
    lockupError,
  };
}

/** Convert HNS (human-readable) to doos (integer base unit). */
export function hnsToDoos(hns: number): number {
  return Math.round(hns * 1_000_000);
}

/** Convert doos to HNS for display. */
export function doosToHns(doos: number): number {
  return doos / 1_000_000;
}
