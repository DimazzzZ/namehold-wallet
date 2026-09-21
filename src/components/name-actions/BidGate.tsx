import type { ReactNode } from "react";
import { BidForm, type BidFormProps } from "./BidForm";
import { formatCountdown, type PhaseBadge, type PhaseCountdown } from "../../lib/auction";
import type { NameActionCapability } from "../../types";

/**
 * The single gate for the bid + lockup inputs (Diagnostic fix: the advanced
 * auction section used to render {@link BidForm} unconditionally, so the Bid /
 * Lockup fields showed — inviting input — in phases where bidding is
 * impossible, e.g. right next to "Open" during OPENING. That made it look as
 * though Open would also place a bid.
 *
 * The gate is a single predicate — `canBid.allowed` — shared by BOTH call
 * sites (the guided BIDDING panel and the advanced section):
 *
 *   - `canBid.allowed` true  → render the {@link BidForm} (the "dumb" input pair).
 *   - false + a pre-bidding phase (AVAILABLE / OPENING / a BIDDING phase where
 *     the user has already bid) → render a contextual placeholder that says
 *     how long to wait (reusing the existing `countdown` / {@link formatCountdown})
 *     and what to do next, falling back to `canBid.reason`.
 *   - false + any other phase (REVEAL / CLOSED / TRANSFER / REVOKED) → render
 *     nothing; the advanced section already surfaces those actions elsewhere.
 *
 * `BidForm` stays "dumb": it only knows how to draw the fields. All phase
 * reasoning lives here.
 */
export interface BidGateProps extends BidFormProps {
  /** The single visibility predicate: whether bidding is possible right now. */
  canBid: NameActionCapability;
  /** Current auction phase — used to pick the placeholder copy. */
  phase: PhaseBadge["phase"];
  /** Existing countdown for this name; drives "opens in …" / "Reveal opens in …". */
  countdown: PhaseCountdown | null;
}

/** The contextual placeholder shown when the inputs are gated off. Returns
 *  null for phases where a bid-related hint would be noise. */
function bidPlaceholder(
  phase: PhaseBadge["phase"],
  countdown: PhaseCountdown | null,
  reason: string | null,
): ReactNode {
  const cd = countdown ? formatCountdown(countdown) : null;

  let body: ReactNode = null;
  switch (phase) {
    case "AVAILABLE":
      body = "Open the auction first; bidding starts after the opening period.";
      break;
    case "OPENING":
      body = cd
        ? `Bidding opens in ${cd}. You'll set your bid & lockup then.`
        : "Bidding opens after the opening period. You'll set your bid & lockup then.";
      break;
    case "BIDDING":
      // canBid is false in BIDDING only once the user has already placed a bid.
      body = cd ? `Your bid is placed. Reveal opens in ${cd}.` : "Your bid is placed.";
      break;
    default:
      // REVEAL / CLOSED / TRANSFER / REVOKED / OTHER — no bid hint here.
      return null;
  }

  return (
    <div
      className="text-xs text-gray-600 bg-gray-50 border border-gray-200 rounded p-2"
      data-testid="bid-gate-placeholder"
    >
      {body}
      {/* Fall back to the backend-provided reason when our copy is generic. */}
      {reason && !cd && <div className="mt-1 text-gray-500">{reason}</div>}
    </div>
  );
}

export function BidGate({ canBid, phase, countdown, ...formProps }: BidGateProps) {
  if (canBid.allowed) {
    return <BidForm {...formProps} />;
  }
  return <>{bidPlaceholder(phase, countdown, canBid.reason)}</>;
}
