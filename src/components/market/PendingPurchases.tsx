import { Button } from "../ui/Button";
import { displayName } from "../../lib/idn";
import { formatHnsShort } from "../../lib/utils";
import { lockupLeftText } from "./marketText";
import { SpeedUpButton } from "../SpeedUp";
import type { ShakedexNameState } from "../../types";

export interface PendingPurchase {
  name: string;
  shakedex: ShakedexNameState;
}

interface PendingPurchasesProps {
  purchases: PendingPurchase[];
  /** Why this profile cannot finalize (`shakedexRefusal`), or null. Never the mainnet gate. */
  finalizeRefusal: string | null;
  /** The purchase whose finalize is in flight. */
  finalizingId: string | null;
  /** Whether Finalize is the page's one filled button: no banner or Buy beside it. */
  primary: boolean;
  onFinalize: (p: PendingPurchase) => void;
}

const GRID =
  "grid grid-cols-[minmax(120px,1fr)_minmax(0,1.4fr)_120px] items-center gap-4 px-4 py-2.5";

/** Where a purchase stands: how far its lockup is, and the words for it. */
function progress(s: ShakedexNameState): { share: number; text: string; ready: boolean } {
  if (s.state === "unconfirmed") {
    return { share: 0, text: "Waiting for the purchase to be mined", ready: false };
  }
  if (s.blocksRemaining == null) {
    return { share: 0, text: "Mined · the wait is counted at the next sync", ready: false };
  }
  if (s.blocksRemaining === 0) return { share: 1, text: "Ready to finalize", ready: true };
  const done = Math.max(0, s.lockupBlocks - s.blocksRemaining);
  return {
    share: s.lockupBlocks > 0 ? done / s.lockupBlocks : 0,
    text: `${done} of ${s.lockupBlocks} blocks`,
    ready: false,
  };
}

/**
 * Purchases bought and not finalized yet, each with its lockup's progress
 * and, once it is over, Finalize. Hidden when there are none.
 */
export function PendingPurchases({
  purchases,
  finalizeRefusal,
  finalizingId,
  primary,
  onFinalize,
}: PendingPurchasesProps) {
  if (purchases.length === 0) return null;
  return (
    <section
      data-testid="pending-purchases"
      className="rounded-md border border-gray-200 bg-white"
      aria-labelledby="pending-purchases-title"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-2 border-b border-gray-100 px-4 py-2.5">
        <h3 id="pending-purchases-title" className="text-sm font-semibold text-gray-700">
          Your purchases
        </h3>
        <p className="text-xs text-gray-500 [text-wrap:pretty]">
          A bought name becomes yours once you finalize it, when its wait is over.
        </p>
      </div>
      {finalizeRefusal && (
        <p
          id="pending-purchases-refusal"
          className="border-b border-gray-100 px-4 py-2 text-xs text-amber-800"
        >
          {finalizeRefusal}
        </p>
      )}
      <ul className="divide-y divide-gray-100">
        {purchases.map((p) => {
          const pr = progress(p.shakedex);
          const busy = finalizingId === p.shakedex.purchaseId;
          return (
            <li key={p.shakedex.purchaseId} className={GRID} data-testid="pending-purchase">
              <div className="min-w-0">
                <div className="truncate font-mono text-sm font-medium text-gray-900">
                  .{displayName(p.name)}
                </div>
                <div className="text-xs tabular-nums text-gray-500">
                  Paid {formatHnsShort(p.shakedex.priceDoos)} HNS
                </div>
              </div>
              <div className="min-w-0">
                <div className="h-1.5 overflow-hidden rounded-full bg-gray-100" aria-hidden>
                  <div
                    className={`h-full rounded-full ${pr.ready ? "bg-green-500" : "bg-blue-600"}`}
                    style={{ width: `${Math.round(pr.share * 100)}%` }}
                  />
                </div>
                <div
                  className="mt-1 text-xs tabular-nums text-gray-600"
                  data-testid="pending-purchase-progress"
                >
                  {pr.text}
                </div>
              </div>
              <div className="text-right text-xs text-gray-500">
                {pr.ready ? (
                  <Button
                    variant={primary ? "primary" : "secondary"}
                    size="sm"
                    data-testid="pending-purchase-finalize"
                    disabled={finalizeRefusal !== null || finalizingId !== null}
                    aria-describedby={finalizeRefusal ? "pending-purchases-refusal" : undefined}
                    onClick={() => onFinalize(p)}
                  >
                    {busy ? "Finalizing…" : "Finalize"}
                  </Button>
                ) : p.shakedex.blocksRemaining != null &&
                  p.shakedex.state === "awaitingFinalize" ? (
                  lockupLeftText(p.shakedex.blocksRemaining, p.shakedex.wallClockTiming)
                ) : p.shakedex.state === "unconfirmed" && p.shakedex.purchaseDraftId ? (
                  // An Unconfirmed purchase can be sped up (spec 2026-10-21 R16).
                  <SpeedUpButton draftId={p.shakedex.purchaseDraftId} isPurchase />
                ) : null}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
