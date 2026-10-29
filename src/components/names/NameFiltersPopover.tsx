import { useState } from "react";
import { SlidersHorizontal } from "lucide-react";
import {
  activeFilterCount,
  NO_FILTERS,
  type CharKind,
  type IcannFilter,
  type ExpiresWithin,
  type LengthBucket,
  type NameFilters,
} from "../../lib/nameFilters";
import { cn } from "../../lib/utils";
import { Button } from "../ui/Button";
import { Icon } from "../ui/Icon";
import { Popover } from "../ui/Popover";

export function expiresLabels(clock: boolean): Record<ExpiresWithin, string> {
  return clock
    ? { any: "Any time", "30": "Within 30 days", "90": "Within 90 days", "365": "Within a year" }
    : {
        any: "Any",
        "30": "Within 100 blocks",
        "90": "Within 1,000 blocks",
        "365": "Within 10,000 blocks",
      };
}

export const LENGTH_LABELS: Record<LengthBucket, string> = {
  any: "Any",
  "1-3": "1–3",
  "4-5": "4–5",
  "6-10": "6–10",
  "11+": "11+",
};

export const ICANN_LABELS: Record<IcannFilter, string> = {
  any: "Any",
  collision: "Applied for as gTLD",
};

export const CHAR_LABELS: Record<CharKind, string> = {
  any: "Any",
  letters: "Letters only",
  digits: "Has digits",
  hyphen: "Has a hyphen",
  idn: "Unicode (IDN)",
};

function Pills<K extends string>({
  name,
  labels,
  value,
  onChange,
}: {
  name: string;
  labels: Record<K, string>;
  value: K;
  onChange: (k: K) => void;
}) {
  return (
    <div className="flex flex-wrap gap-1.5">
      {(Object.keys(labels) as K[]).map((k) => (
        <button
          key={k}
          type="button"
          aria-pressed={value === k}
          onClick={() => onChange(k)}
          data-testid={`names-filter-${name}-${k}`}
          className={cn(
            "rounded-full border px-2.5 py-0.5 text-xs",
            value === k
              ? "bg-gray-900 text-white border-gray-900"
              : "bg-white text-gray-700 border-gray-300 hover:bg-gray-50",
          )}
        >
          {labels[k]}
        </button>
      ))}
    </div>
  );
}

/** The Filters button and its popover: expiry, length, characters, tags. Changes apply at once. */
export function NameFiltersPopover({
  filters,
  onChange,
  tagCounts,
  clock,
}: {
  filters: NameFilters;
  onChange: (f: NameFilters) => void;
  /** Tags in use and how many rows carry each. */
  tagCounts: [string, number][];
  clock: boolean;
}) {
  const [open, setOpen] = useState(false);
  const count = activeFilterCount(filters);
  const set = (patch: Partial<NameFilters>) => onChange({ ...filters, ...patch });

  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      align="right"
      role="dialog"
      aria-label="Filter names"
      data-testid="names-filters-popover"
      panelClassName="w-[340px] py-0 rounded-lg shadow-lg"
      trigger={({ toggle }) => (
        <button
          type="button"
          onClick={toggle}
          aria-expanded={open}
          data-testid="names-filters-button"
          className={cn(
            "inline-flex items-center gap-1.5 rounded border px-2.5 py-1 text-sm",
            count > 0
              ? "bg-blue-50 border-blue-300 text-blue-700"
              : "bg-white border-gray-300 text-gray-900 hover:bg-gray-50",
          )}
        >
          <Icon icon={SlidersHorizontal} size={14} /> Filters
          {count > 0 && (
            <span className="ml-0.5 rounded-full bg-blue-600 px-1.5 text-[11px] font-semibold text-white tabular-nums">
              {count}
            </span>
          )}
        </button>
      )}
    >
      {({ close }) => (
        <div className="text-[13px]">
          <div className="flex flex-col gap-3.5 p-3.5">
            <section className="flex flex-col gap-1.5">
              <div className="font-medium text-gray-900">Expires</div>
              <Pills
                name="expires"
                labels={expiresLabels(clock)}
                value={filters.expires}
                onChange={(expires) => set({ expires })}
              />
              <p className="text-xs text-gray-500">
                Names without an expiry (auctions, most watched names) are hidden while this is on.
              </p>
            </section>
            <section className="flex flex-col gap-1.5">
              <div className="font-medium text-gray-900">
                Length{" "}
                <span className="font-normal text-gray-500">· characters, without the dot</span>
              </div>
              <Pills
                name="len"
                labels={LENGTH_LABELS}
                value={filters.len}
                onChange={(len) => set({ len })}
              />
            </section>
            <section className="flex flex-col gap-1.5">
              <div className="font-medium text-gray-900">Characters</div>
              <Pills
                name="chars"
                labels={CHAR_LABELS}
                value={filters.chars}
                onChange={(chars) => set({ chars })}
              />
            </section>
            <section className="flex flex-col gap-1.5">
              <div className="font-medium text-gray-900">ICANN 2026 round</div>
              <Pills
                name="icann"
                labels={ICANN_LABELS}
                value={filters.icann}
                onChange={(icann) => set({ icann })}
              />
            </section>
            <section className="flex flex-col gap-1.5">
              <div className="font-medium text-gray-900">
                Tags <span className="font-normal text-gray-500">· any of</span>
              </div>
              {tagCounts.length === 0 ? (
                <p className="text-xs text-gray-500">
                  No tags yet. Add tags to watched names from their row.
                </p>
              ) : (
                tagCounts.map(([tag, n]) => (
                  <label key={tag} className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={filters.tags.includes(tag)}
                      onChange={(e) =>
                        set({
                          tags: e.target.checked
                            ? [...filters.tags, tag]
                            : filters.tags.filter((t) => t !== tag),
                        })
                      }
                      data-testid={`names-filter-tag-${tag}`}
                    />
                    <span className="bg-gray-100 text-gray-600 text-xs rounded px-1.5">{tag}</span>
                    <span className="ml-auto text-xs text-gray-500 tabular-nums">{n}</span>
                  </label>
                ))
              )}
            </section>
          </div>
          <div className="flex items-center justify-between border-t border-gray-100 px-3.5 py-2.5">
            <button
              type="button"
              className="text-xs text-blue-600 hover:underline"
              onClick={() => onChange(NO_FILTERS)}
              data-testid="names-filters-clear"
            >
              Clear all
            </button>
            <Button size="sm" variant="secondary" onClick={close}>
              Done
            </Button>
          </div>
        </div>
      )}
    </Popover>
  );
}
