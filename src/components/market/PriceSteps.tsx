import { approxWait } from "./marketText";
import { stepsAhead } from "./marketModel";
import { formatHnsShort } from "../../lib/utils";
import type { MarketRow } from "../../types";

/** Past this many steps ahead, the middle ones are left out. */
const MAX_ROWS = 8;

/**
 * A reverse auction's prices from now on (`stepsAhead`): when each step
 * becomes valid, a bar as long as its share of the highest, and the price.
 * The current step is the first and stands out. Steps that will never set
 * the price are not shown.
 */
export function PriceSteps({ row }: { row: MarketRow }) {
  const ahead = stepsAhead(row);
  if (ahead.length === 0) return null;
  const max = Math.max(...ahead.map((s) => s.price));
  const shown =
    ahead.length > MAX_ROWS ? [...ahead.slice(0, MAX_ROWS - 1), ahead[ahead.length - 1]!] : ahead;
  const skipped = ahead.length - shown.length;
  return (
    <ol className="mt-3 space-y-1.5 text-xs tabular-nums" data-testid="price-steps">
      {shown.map((s, i) => {
        const now = i === 0;
        const last = i === shown.length - 1;
        return (
          <li key={`${s.lockTime}-${i}`}>
            {last && skipped > 0 && (
              <div className="mb-1.5 text-gray-400">
                {skipped} more {skipped === 1 ? "step" : "steps"}…
              </div>
            )}
            <div className="grid grid-cols-[56px_1fr_84px] items-center gap-2">
              <span className={now ? "font-semibold text-gray-900" : "text-gray-500"}>
                {now ? "Now" : approxWait(s.validInSecs ?? 0)}
              </span>
              <span className="h-2 rounded-full bg-gray-100" aria-hidden>
                <span
                  className={`block h-full rounded-full ${now ? "bg-blue-600" : "bg-slate-300"}`}
                  style={{ width: `${max > 0 ? (s.price / max) * 100 : 0}%` }}
                />
              </span>
              <span
                className={`text-right ${now ? "font-semibold text-gray-900" : "text-gray-500"}`}
              >
                {formatHnsShort(s.price)} HNS
              </span>
            </div>
          </li>
        );
      })}
    </ol>
  );
}
