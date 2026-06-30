// Vickrey-auction phase derivation + countdown helpers.
//
// hsd `getnameinfo` reports a `state` (OPENING / BIDDING / REVEAL / CLOSED …)
// and, in `stats`, the block/time distance to the next phase. We turn those into
// a UI badge + a human countdown and a recommended next action. All inputs are
// optional/nullable — the explorer path may omit the auction stats entirely, so
// every function degrades to "unknown" rather than throwing.

import type { HsdNameStats } from "../types";

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
