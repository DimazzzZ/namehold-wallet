import { useNameBids } from "../../queries/read";
import { Badge } from "../ui/Badge";
import { Tooltip } from "../ui/Tooltip";
import { formatHns } from "../../lib/utils";
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
  suppressEmptyHint = false,
}: {
  name: string;
  profileId: string | null;
  phase: string;
  /**
   * When true, the "No bids yet" empty-state hint is not rendered even in
   * OPENING/BIDDING. Set this while the bidding window has not opened
   * (`taskState === "waitingForBidding"`): the guided panel above already
   * says when bidding starts, so an empty explorer bid list (mock /
   * not-yet-indexed) rendering "No bids yet" right below it reads as a
   * direct contradiction rather than as the obvious consequence.
   */
  suppressEmptyHint?: boolean;
}) {
  const { data, isLoading, isError } = useNameBids(name, profileId);

  if (isLoading || isError) return null;

  const bids = data?.bids ?? [];
  const showEmptyHint = phase === "OPENING" || phase === "BIDDING";

  if (bids.length === 0) {
    if (!showEmptyHint || suppressEmptyHint) return null;
    return (
      <div className="text-xs text-gray-400" data-testid="name-bids">
        No bids yet
      </div>
    );
  }

  const isRevealPhase = phase === "REVEAL" || phase === "CLOSED";
  const myBidCount = data?.myBidCount ?? bids.filter((b) => b.mine).length;
  // "N bids so far" counts what the chain has; a bid of ours still in the
  // mempool is not one of them yet, and is called out separately.
  const pendingCount = bids.filter((b) => b.pending).length;
  const onChainCount = bids.length - pendingCount;

  return (
    <div className="text-sm" data-testid="name-bids">
      <div className="text-xs font-medium text-gray-600 mb-1">Bids</div>

      {!isRevealPhase && (
        <div className="text-xs text-gray-500 mb-1">
          {onChainCount} bids so far · yours: {myBidCount}
          {pendingCount > 0 && ` · ${pendingCount} of yours waiting for a block`}
        </div>
      )}

      {isRevealPhase && data?.highest != null && (
        <div className="text-xs text-gray-500 mb-1">High bid: {formatHns(data.highest)} HNS</div>
      )}

      <ul className="space-y-1">
        {bids.map((bid, i) => (
          <BidRow key={bid.txid ?? `${bid.index ?? i}`} bid={bid} />
        ))}
      </ul>
    </div>
  );
}

/**
 * The "lockup is not the bid" caveat, on the word it qualifies.
 *
 * It used to sit inline on every row as "(max, not the actual bid)" — the
 * single most repeated string in the panel, and dead weight to anyone who
 * already knows how a Vickrey auction works. On the word itself it stays one
 * hover away for anyone who does not.
 */
function LockupLabel() {
  return (
    <Tooltip
      hint
      content="The most this bidder could have bid. The true bid stays sealed until the reveal phase."
    >
      lockup
    </Tooltip>
  );
}

function BidRow({ bid }: { bid: NameBid }) {
  const revealed = bid.revealed === true;

  // A bid that belongs to THIS wallet is highlighted so it's unmistakable in
  // the shared list — a tinted background + left accent bar + rounded padding.
  // Multi-bid: several rows can be `mine`, each independently distinguished.
  const rowClass = bid.mine
    ? "flex items-center gap-2 text-xs text-gray-800 bg-blue-50 border-l-2 border-blue-400 rounded px-1.5 py-0.5"
    : "flex items-center gap-2 text-xs text-gray-700";

  // Badges live in one right-aligned group, with "You" last, so it sits in the
  // same column on every row instead of drifting with the figures beside it.
  const badges = (
    <span className="ml-auto flex items-center gap-2">
      {bid.pending && (
        <Badge
          variant="warning"
          title="Sent to the network. It joins the auction once a block includes it."
        >
          waiting for a block
        </Badge>
      )}
      {bid.win === true && <Badge variant="success">Winner</Badge>}
      {bid.mine && <Badge variant="info">You</Badge>}
    </span>
  );

  // Ours, sent, not yet in a block. The chain knows nothing about it, so the
  // row states that rather than sitting among the confirmed ones unmarked.
  if (bid.pending) {
    return (
      <li className={rowClass} data-testid="name-bid-row-pending">
        <span>
          <LockupLabel />: {formatHns(bid.lockup)} HNS
        </span>
        <span>your bid: {formatHns(bid.myValue)} HNS</span>
        {badges}
      </li>
    );
  }

  if (!revealed) {
    // BIDDING-style row: only the public lockup is knowable — a competitor's
    // `value` is hidden (or 0) pre-reveal, and rendering it as "their bid"
    // would mislead the user into overpaying. Our own plaintext bid
    // (`myValue`) is a local secret, not derived from the explorer, so it's
    // safe to show.
    return (
      <li className={rowClass} data-testid={bid.mine ? "name-bid-row-mine" : "name-bid-row"}>
        <span>
          <LockupLabel />: {formatHns(bid.lockup)} HNS
        </span>
        {bid.mine && <span>your bid: {formatHns(bid.myValue)} HNS</span>}
        {badges}
      </li>
    );
  }

  // REVEAL/CLOSED-style row: the true value is public.
  return (
    <li className={rowClass} data-testid={bid.mine ? "name-bid-row-mine" : "name-bid-row"}>
      <span>bid: {formatHns(bid.value)} HNS</span>
      {badges}
    </li>
  );
}
