import { Check, TrendingDown } from "lucide-react";
import { Icon } from "../ui/Icon";
import { Badge } from "../ui/Badge";
import { displayName } from "../../lib/idn";
import { fromInteractiveChild } from "../../lib/rowClick";
import { formatHnsShort } from "../../lib/utils";
import { nextPriceDrops, nextPriceText } from "./marketText";
import { rowTag, type Entry } from "./marketModel";

/** The listings' columns: Name · Price now · Next price · Check, shared by the header, rows and skeleton. */
export const LISTING_GRID =
  "grid grid-cols-[minmax(120px,1.3fr)_100px_minmax(110px,1fr)_100px] items-center gap-4 px-4";

interface ListingRowProps {
  entry: Entry;
  selected: boolean;
  bought: boolean;
  /** A Retry of this row is on its way. */
  checking: boolean;
  onSelect: () => void;
  onRetry: () => void;
}

/** One listing: one click target that selects it; Retry inside handles its own click. */
export function ListingRow({
  entry,
  selected,
  bought,
  checking,
  onSelect,
  onRetry,
}: ListingRowProps) {
  const tag = rowTag(entry, bought);
  const listed = entry.kind === "listed" ? entry.row : null;
  return (
    <div
      role="option"
      id={`listing-${entry.key}`}
      aria-selected={selected}
      data-testid="market-row"
      data-selected={selected || undefined}
      onClick={(e) => {
        if (!fromInteractiveChild(e)) onSelect();
      }}
      className={`${LISTING_GRID} cursor-pointer border-t border-gray-100 py-2.5 ${
        selected ? "bg-blue-50 shadow-[inset_2px_0_0_#2563eb]" : "hover:bg-gray-50"
      }`}
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <span className="break-all font-mono text-sm font-medium text-gray-900">
          .{displayName(entry.name)}
        </span>
        {tag && (
          <Badge variant={tag.variant} data-testid="market-row-tag">
            {tag.label}
          </Badge>
        )}
      </div>
      <div className="text-right text-sm tabular-nums" data-testid="market-row-price">
        {listed?.currentPrice != null ? (
          <span className="font-semibold text-gray-900">
            {formatHnsShort(listed.currentPrice)} HNS
          </span>
        ) : (
          <span className="text-gray-400">—</span>
        )}
      </div>
      <div
        className={`text-xs tabular-nums ${listed?.kind === "reverseAuction" ? "text-gray-700" : "text-gray-400"}`}
      >
        {listed ? (
          <span className="flex items-center gap-1">
            {nextPriceDrops(listed) && <Icon icon={TrendingDown} size={13} />}
            {nextPriceText(listed)}
          </span>
        ) : null}
      </div>
      <div className="text-xs" data-testid="market-row-check">
        {entry.kind === "listed" ? (
          <span className="inline-flex items-center gap-1 text-green-700">
            <Icon icon={Check} size={13} /> Verified
          </span>
        ) : (
          <span className="flex items-center gap-1.5 text-amber-700">
            <span>Couldn't check</span>
            <button
              type="button"
              data-testid="market-row-retry"
              disabled={checking}
              onClick={onRetry}
              className="cursor-pointer text-blue-600 hover:underline disabled:cursor-default disabled:text-gray-400"
            >
              {checking ? "Checking…" : "Retry"}
            </button>
          </span>
        )}
      </div>
    </div>
  );
}
