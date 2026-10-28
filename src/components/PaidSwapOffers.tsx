import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { displayName } from "../lib/idn";
import { formatHnsShort } from "../lib/utils";
import { Tooltip } from "./ui/Tooltip";
import type { PaidSwapOffer } from "../types";

/**
 * Offers recorded by the paid swap flow before it was withdrawn
 * (docs/specs/2026-09-21-paid-name-swaps.md), read-only: nothing can create,
 * claim or remove one any more. Shown so a seller still sees the buyer and
 * price agreed; hidden when there are none.
 */
export function PaidSwapOffers({ onNameClick }: { onNameClick?: (name: string) => void } = {}) {
  const { data: offers = [] } = useQuery<PaidSwapOffer[]>({
    queryKey: ["paid-swap-offers"],
    queryFn: async () => (await invoke<PaidSwapOffer[] | null>("list_paid_swap_offers")) ?? [],
    staleTime: Infinity,
  });
  if (offers.length === 0) return null;
  return (
    <section
      data-testid="paid-swap-offers"
      className="rounded-md border border-gray-200 bg-white"
      aria-labelledby="paid-swap-offers-title"
    >
      <div className="border-b border-gray-100 px-4 py-3">
        <h3 id="paid-swap-offers-title" className="text-sm font-semibold text-gray-900">
          Paid swap offers
        </h3>
        <p className="mt-0.5 text-xs text-gray-500 [text-wrap:pretty]">
          Recorded before paid swaps were withdrawn, kept for reference. To sell a name, use Sell in
          its actions.
        </p>
      </div>
      <ul className="divide-y divide-gray-100 text-sm">
        {offers.map((o) => (
          <li
            key={o.name}
            data-testid="paid-swap-offer"
            className="grid grid-cols-[minmax(120px,1fr)_minmax(0,2fr)_auto_auto] items-center gap-4 px-4 py-2.5"
          >
            {onNameClick ? (
              <Tooltip content="View name info" className="inline-flex min-w-0">
                <button
                  type="button"
                  className="truncate text-left font-mono font-medium text-gray-900 hover:text-blue-700 hover:underline"
                  onClick={() => onNameClick(o.name)}
                >
                  .{displayName(o.name)}
                </button>
              </Tooltip>
            ) : (
              <span className="truncate font-mono font-medium text-gray-900">
                .{displayName(o.name)}
              </span>
            )}
            <span className="truncate font-mono text-xs text-gray-600">{o.buyerAddress}</span>
            <span className="text-right tabular-nums text-gray-900">
              {formatHnsShort(o.priceDoos)} HNS
            </span>
            <span className={`text-xs ${o.claimed ? "text-green-700" : "text-gray-500"}`}>
              {o.claimed ? "Claimed" : "Not claimed"}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
