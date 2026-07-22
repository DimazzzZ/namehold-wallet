import { useNameBids } from "../../queries/read";
import { Badge } from "../ui/Badge";
import { formatHns } from "../../lib/utils";
import { displayName } from "../../lib/idn";
import type { NameBid } from "../../types";

/**
 * Phase-aware, honest bid list for the auction modal (Task 2).
 *
 * The whole point of this component is to never let a Vickrey bidder mistake
 * a competitor's public LOCKUP (an upper bound they chose to obscure their
 * true bid) for their actual bid. Every row decides its own rendering from
 * that row's own `revealed` flag (not just the overall phase) — a name can
 * sit in REVEAL with some bids already revealed and others not yet.
 *
 * - Not-yet-revealed rows: show only the lockup, explicitly labeled as a
 *   max/deposit, never the actual bid. A `mine` row is the one exception —
 *   we already know our own plaintext bid locally (`myValue`), so it's safe
 *   (and useful) to show it.
 * - Revealed rows: show the true `value`, with a "Winner" badge when
 *   `win === true`.
 *
 * Degrades silently: loading/error/no-data/no-bids never throws and, outside
 * OPENING/BIDDING, renders nothing at all (no empty box cluttering the
 * modal).
 */
export function NameBidsPanel({
  name,
  profileId,
  phase,
}: {
  name: string;
  profileId: string | null;
  phase: string;
}) {
  const { data, isLoading, isError } = useNameBids(name, profileId);

  if (isLoading || isError) return null;

  const bids = data?.bids ?? [];
  const showEmptyHint = phase === "OPENING" || phase === "BIDDING";

  if (bids.length === 0) {
    if (!showEmptyHint) return null;
    return (
      <div className="text-xs text-gray-400" data-testid="name-bids">
        No bids yet
      </div>
    );
  }

  const isRevealPhase = phase === "REVEAL" || phase === "CLOSED";
  const myBidCount = data?.myBidCount ?? bids.filter((b) => b.mine).length;

  return (
    <div className="text-sm" data-testid="name-bids">
      <div className="text-xs font-medium text-gray-600 mb-1">
        Bids for {displayName(name)}
      </div>

      {!isRevealPhase && (
        <div className="text-xs text-gray-500 mb-1">
          {bids.length} bids so far · yours: {myBidCount}
        </div>
      )}

      {isRevealPhase && data?.highest != null && (
        <div className="text-xs text-gray-500 mb-1">
          High bid: {formatHns(data.highest)} HNS
        </div>
      )}

      <ul className="space-y-1">
        {bids.map((bid, i) => (
          <BidRow key={bid.txid ?? `${bid.index ?? i}`} bid={bid} />
        ))}
      </ul>
    </div>
  );
}

function BidRow({ bid }: { bid: NameBid }) {
  const revealed = bid.revealed === true;

  if (!revealed) {
    // BIDDING-style row: only the public lockup is knowable — a competitor's
    // `value` is hidden (or 0) pre-reveal, and rendering it as "their bid"
    // would mislead the user into overpaying. Our own plaintext bid
    // (`myValue`) is a local secret, not derived from the explorer, so it's
    // safe to show.
    return (
      <li className="flex items-center gap-2 text-xs text-gray-700">
        <span>lockup: {formatHns(bid.lockup)} HNS</span>
        <span className="text-gray-400">(max, not the actual bid)</span>
        {bid.mine && (
          <>
            <Badge variant="info">You</Badge>
            <span>your bid: {formatHns(bid.myValue)} HNS</span>
          </>
        )}
      </li>
    );
  }

  // REVEAL/CLOSED-style row: the true value is public.
  return (
    <li className="flex items-center gap-2 text-xs text-gray-700">
      <span>bid: {formatHns(bid.value)} HNS</span>
      {bid.win === true && <Badge variant="success">Winner</Badge>}
      {bid.mine && <Badge variant="info">You</Badge>}
    </li>
  );
}
