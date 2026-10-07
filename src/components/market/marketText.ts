import { formatDate } from "../../lib/utils";
import type { HiddenCounts, ShakedexHidden } from "../../types";

/** Why a market link cannot be imported off mainnet (R5). */
export const MARKET_MAINNET_ONLY = "LearnHNS Market lists mainnet names only";

/** "~6 h" — the wait is measured against the node's median time, so it is only approximate. */
export function approxWait(secs: number): string {
  if (secs < 3600) return `~${Math.max(1, Math.round(secs / 60))} min`;
  if (secs < 48 * 3600) return `~${Math.round(secs / 3600)} h`;
  return `~${Math.round(secs / 86400)} d`;
}

/** The "Hidden N: X already sold or cancelled, ..." line; zero parts are omitted. */
export function hiddenSummary(h: HiddenCounts): string | null {
  const parts: [number, string][] = [
    [h.soldOrCancelled, "already sold or cancelled"],
    [h.failedVerification, "failed verification"],
    [h.expiresBeforeFinalize, "expire before they can be finalized"],
    [h.notYetValid, "not valid yet"],
    [h.couldNotCheck, "could not be checked"],
  ];
  const total = parts.reduce((n, [c]) => n + c, 0);
  if (total === 0) return null;
  const text = parts
    .filter(([c]) => c > 0)
    .map(([c, label]) => `${c} ${label}`)
    .join(", ");
  return `Hidden ${total}: ${text}`;
}

/** A row the profile's chain source (SPV, Explorer) cannot check (R8). */
export const NOT_VERIFIED = "Not verified — needs a full or remote node";

/** A human sentence for why a listing is not buyable. */
export function hiddenReasonText(r: ShakedexHidden): string {
  switch (r.kind) {
    case "soldOrCancelled":
      return "Already sold or cancelled.";
    case "failedVerification":
      return `Failed verification: ${r.reason}`;
    case "expiresBeforeFinalize":
      return "The name expires before a purchase could be finalized.";
    case "notYetValid":
      return `Not valid yet; the first price step becomes valid in ${approxWait(r.firstValidInSecs)}.`;
    case "couldNotCheck":
      return `Could not be checked: ${r.reason}`;
    case "unverified":
      return NOT_VERIFIED;
  }
}

/**
 * "Listed until July 10, 2027 · stays buyable until sold or cancelled": the
 * seller's own `expiresAt` (unix seconds). Shown, never enforced: the signed
 * price steps stay valid on chain until the lock coin is spent, whatever the
 * date says (R2), and the spec has the wallet say so wherever the date is
 * shown (§4). `null` when the value is beyond what a date can hold — it comes
 * from a foreign file as any u64.
 */
export function listedUntilText(expiresAt: number): string | null {
  const d = new Date(expiresAt * 1000);
  if (Number.isNaN(d.getTime())) return null;
  return `Listed until ${formatDate(d.toISOString().slice(0, 10))} · stays buyable until sold or cancelled`;
}
