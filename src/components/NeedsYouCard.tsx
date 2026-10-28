import { displayName } from "../lib/idn";
import type { NeedsYouItem } from "../lib/needsYou";
import { TONE_DOT } from "./market/marketText";
import { Button } from "./ui/Button";
import { Tooltip } from "./ui/Tooltip";

interface NeedsYouCardProps {
  items: NeedsYouItem[];
  isLoading: boolean;
  isDegraded: boolean;
  onOpenName: (name: string) => void;
  /** Runs a ready purchase's finalize (usePurchaseFinalize in the page). */
  onFinalizePurchase: (item: NeedsYouItem) => void;
  /** Purchase id of the finalize in flight, or null. */
  finalizingId: string | null;
  /** Why this wallet can't finalize a purchase, or null when it can. */
  finalizeRefusal: string | null;
  /** The "All names" link; omitted until the Names page exists. */
  allNamesLink?: React.ReactNode;
}

/**
 * One row per task the wallet owes the chain, most urgent first. Signing
 * happens on the name's own surface, where the write gate and Unlock live —
 * except a ready purchase, which is finalized right here as in Owned Names.
 */
export function NeedsYouCard({
  items,
  isLoading,
  isDegraded,
  onOpenName,
  onFinalizePurchase,
  finalizingId,
  finalizeRefusal,
  allNamesLink,
}: NeedsYouCardProps) {
  return (
    <section
      className="bg-white rounded-lg border border-gray-200 overflow-hidden"
      data-testid="needs-you"
    >
      <div className="px-4 py-3 border-b border-gray-100 flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-gray-700">
          Needs you <span className="font-normal text-gray-500">· {items.length}</span>
        </h3>
        {allNamesLink}
      </div>
      {isDegraded && (
        <div
          className="bg-amber-50 border-b border-amber-200 text-xs text-amber-800 px-4 py-2"
          data-testid="urgent-tasks-degraded"
        >
          Couldn't verify urgent auction tasks — data may be stale.
        </div>
      )}
      {items.length > 0 ? (
        <ul>
          {items.map((item, i) => (
            <li
              key={`${item.kind}:${item.name}`}
              className={`grid grid-cols-[8px_minmax(120px,190px)_1fr_auto_auto] items-center gap-x-3 px-4 py-2.5 ${
                i > 0 ? "border-t border-gray-100" : ""
              }`}
              data-testid="needs-you-row"
            >
              <span className={`h-2 w-2 rounded-full ${TONE_DOT[item.tone]}`} />
              {item.kind === "finalizePurchase" ? (
                // Not ours yet: nothing to open.
                <span className="font-mono text-[13px] text-gray-900 truncate">
                  .{displayName(item.name)}
                </span>
              ) : (
                <Tooltip content="Open the name page" className="min-w-0">
                  <button
                    type="button"
                    className="font-mono text-[13px] text-gray-900 hover:text-blue-700 hover:underline truncate text-left"
                    onClick={() => onOpenName(item.name)}
                    data-testid="needs-you-name"
                  >
                    .{displayName(item.name)}
                  </button>
                </Tooltip>
              )}
              <span className="text-sm text-gray-700">{item.text}</span>
              <span
                className={`text-xs tabular-nums whitespace-nowrap ${
                  item.tone === "bad"
                    ? "text-red-600"
                    : item.when === "No rush"
                      ? "text-gray-400"
                      : "text-gray-500"
                }`}
              >
                {item.when}
              </span>
              {item.kind === "finalizePurchase" ? (
                <Tooltip content={finalizeRefusal}>
                  <Button
                    size="sm"
                    variant="secondary"
                    className="min-w-[76px]"
                    data-testid="needs-you-action"
                    disabled={finalizingId !== null || finalizeRefusal !== null}
                    onClick={() => onFinalizePurchase(item)}
                  >
                    {finalizingId === item.purchaseId ? "Finalizing…" : item.actionLabel}
                  </Button>
                </Tooltip>
              ) : (
                <Button
                  size="sm"
                  variant="secondary"
                  className="min-w-[76px]"
                  data-testid="needs-you-action"
                  onClick={() => onOpenName(item.name)}
                >
                  {item.actionLabel}
                </Button>
              )}
            </li>
          ))}
        </ul>
      ) : isLoading ? (
        <div className="px-4 py-3 text-sm text-gray-500">Checking your names…</div>
      ) : isDegraded ? null : (
        <div className="px-4 py-3 text-sm text-gray-500" data-testid="needs-you-empty">
          Nothing needs you right now.
        </div>
      )}
    </section>
  );
}
