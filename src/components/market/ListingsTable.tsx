import { ChevronDown, ChevronRight } from "lucide-react";
import { Icon } from "../ui/Icon";
import type { RefObject } from "react";
import { Button } from "../ui/Button";
import { Spinner } from "../ui/Spinner";
import { displayName } from "../../lib/idn";
import { hiddenReasonText } from "./marketText";
import {
  hiddenToggleText,
  type Entry,
  type HiddenEntry,
  type ListFilter,
  type ListSort,
} from "./marketModel";
import { LISTING_GRID, ListingRow } from "./ListingRow";

/** What the rows area says above the rows (the imports, while the market's own cannot be shown), if anything. */
export type ListState =
  | { kind: "loading"; text: string }
  | { kind: "message"; title: string; text: string; testId: string }
  | { kind: "rows" };

interface ListingsTableProps {
  state: ListState;
  rows: Entry[];
  hidden: HiddenEntry[];
  hiddenOpen: boolean;
  onToggleHidden: () => void;
  query: string;
  onQuery: (q: string) => void;
  filter: ListFilter;
  onFilter: (f: ListFilter) => void;
  sort: ListSort;
  onSort: (s: ListSort) => void;
  searchRef: RefObject<HTMLInputElement | null>;
  selectedKey: string | null;
  onSelect: (key: string) => void;
  isBought: (e: Entry) => boolean;
  checkingKey: string | null;
  onRetry: (e: Entry) => void;
  /** Load more: null when this network has no market to page through. */
  more: { hasMore: boolean; loading: boolean; onLoadMore: () => void } | null;
}

const FILTERS: { key: ListFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "fixed", label: "Fixed price" },
  { key: "drops", label: "Price drops" },
];

/** Centered title and a line saying what will make the rows appear. */
function ListMessage({ title, text, testId }: { title: string; text: string; testId: string }) {
  return (
    <div className="px-6 py-10 text-center" data-testid={testId}>
      <p className="text-sm font-semibold text-gray-700">{title}</p>
      <p className="mx-auto mt-1 max-w-[380px] text-sm text-gray-500 [text-wrap:pretty]">{text}</p>
    </div>
  );
}

/**
 * The listings card: toolbar, header, a state if any, the rows, the hidden
 * rows, and the footer. A state stands above the rows, never in their place:
 * the imports, the hidden toggle and Load more stay reachable through it.
 */
export function ListingsTable(p: ListingsTableProps) {
  const toggle = hiddenToggleText(p.hidden, p.hiddenOpen);
  return (
    <section className="rounded-md border border-gray-200 bg-white" aria-label="Listings">
      <div className="flex flex-wrap items-center gap-3 border-b border-gray-200 px-3 py-2.5">
        <input
          ref={p.searchRef}
          type="search"
          value={p.query}
          onChange={(e) => p.onQuery(e.target.value)}
          placeholder="Find a name"
          aria-label="Find a name"
          data-testid="market-search"
          className="min-w-[140px] flex-1 rounded border border-gray-300 px-2.5 py-1.5 text-[13px] focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <div className="flex overflow-hidden rounded border border-gray-300" role="group">
          {FILTERS.map((f) => (
            <button
              key={f.key}
              type="button"
              aria-pressed={p.filter === f.key}
              data-testid={`market-filter-${f.key}`}
              onClick={() => p.onFilter(f.key)}
              className={`cursor-pointer px-2.5 py-[5px] text-xs ${
                p.filter === f.key
                  ? "bg-gray-900 font-medium text-white"
                  : "bg-white text-gray-700 hover:bg-gray-50"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>
        <select
          value={p.sort}
          onChange={(e) => p.onSort(e.target.value as ListSort)}
          aria-label="Sort"
          data-testid="market-sort"
          className="rounded border border-gray-300 bg-white px-2 py-[5px] text-xs"
        >
          <option value="price">Lowest price</option>
          <option value="drop">Next price drop</option>
          <option value="newest">Newest</option>
        </select>
      </div>

      <div className={`${LISTING_GRID} py-2 text-xs text-gray-500`} aria-hidden>
        <span>Name</span>
        <span className="text-right">Price now</span>
        <span>Next price</span>
        <span>Check</span>
      </div>

      {p.state.kind === "loading" && (
        <div aria-busy="true" data-testid="market-loading">
          <p className="flex items-center gap-2 border-t border-gray-100 px-4 py-2 text-xs text-gray-600">
            <Spinner className="text-blue-600" />
            {p.state.text}
          </p>
          {[0, 1, 2, 3, 4].map((i) => (
            <div
              key={i}
              aria-hidden
              className={`${LISTING_GRID} border-t border-gray-100 py-3.5 motion-safe:animate-pulse`}
            >
              <div className="h-3.5 w-28 rounded bg-gray-200" />
              <div className="ml-auto h-3.5 w-16 rounded bg-gray-200" />
              <div className="h-3 w-24 rounded bg-gray-100" />
              <div className="h-3 w-16 rounded bg-gray-100" />
            </div>
          ))}
        </div>
      )}
      {p.state.kind === "message" && (
        <div className="border-t border-gray-100">
          <ListMessage {...p.state} />
        </div>
      )}

      {p.state.kind === "rows" && p.rows.length === 0 && (
        <div className="border-t border-gray-100">
          <ListMessage
            title="No listings match"
            text="Try another name or filter."
            testId="market-no-match"
          />
        </div>
      )}
      {p.rows.length > 0 && (
        <div
          role="listbox"
          aria-label="Listings"
          tabIndex={0}
          aria-activedescendant={p.selectedKey ? `listing-${p.selectedKey}` : undefined}
          className="focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-500"
        >
          {p.rows.map((e) => (
            <ListingRow
              key={e.key}
              entry={e}
              selected={e.key === p.selectedKey}
              bought={p.isBought(e)}
              checking={p.checkingKey === e.key}
              onSelect={() => p.onSelect(e.key)}
              onRetry={() => p.onRetry(e)}
            />
          ))}
        </div>
      )}
      {p.hiddenOpen && p.hidden.length > 0 && (
        <ul className="bg-[#fafafa] text-xs text-gray-500" data-testid="hidden-rows">
          {p.hidden.map((h, i) => (
            <li
              key={`${h.name}-${i}`}
              data-testid="hidden-row"
              className={`${LISTING_GRID} border-t border-gray-100 py-2`}
            >
              <span className="break-all font-mono">
                {h.name === null ? "Unnamed listing" : `.${displayName(h.name)}`}
              </span>
              <span className="text-right">—</span>
              <span className="col-span-2">{hiddenReasonText(h.reason)}</span>
            </li>
          ))}
        </ul>
      )}

      {(toggle || p.more) && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-gray-200 px-4 py-2.5">
          {toggle ? (
            <button
              type="button"
              data-testid="hidden-toggle"
              aria-expanded={p.hiddenOpen}
              onClick={p.onToggleHidden}
              className="cursor-pointer text-left text-xs text-gray-600 hover:text-gray-900"
            >
              <span className="inline-flex items-center gap-1">
                <Icon icon={p.hiddenOpen ? ChevronDown : ChevronRight} size={13} />
                {toggle}
              </span>
            </button>
          ) : (
            <span />
          )}
          {p.more &&
            (p.more.hasMore ? (
              <Button
                size="sm"
                data-testid="market-load-more"
                loading={p.more.loading}
                onClick={p.more.onLoadMore}
              >
                {p.more.loading ? "Checking…" : "Load more"}
              </Button>
            ) : (
              <span className="text-xs text-gray-500" data-testid="market-all-loaded">
                All LearnHNS listings loaded
              </span>
            ))}
        </div>
      )}
    </section>
  );
}
