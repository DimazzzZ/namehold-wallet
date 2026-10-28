import { displayName } from "../../lib/idn";
import { BUYING_OFF_REASON, OWN_LISTING, approxWait, hns, hnsExact, sentence } from "./marketText";
import type { HiddenRow, MarketPage, MarketRow, PriceStepView, ShakedexHidden } from "../../types";

/** Where a listing came from: the market page, a listing file (or pasted text), or a market link. */
export type Origin = "market" | "file" | "link";

/** A listing the node verified: it has a price and can be bought. */
export interface ListedEntry {
  kind: "listed";
  key: string;
  name: string;
  origin: Origin;
  /** Its place in "Newest": imports first, then market order. */
  order: number;
  row: MarketRow;
}

/** A listing the node could not answer for: no price, a Retry. */
export interface UncheckedEntry {
  kind: "unchecked";
  key: string;
  name: string;
  origin: Origin;
  order: number;
  listingJson: string;
  /** The node's own reason, as the backend gave it. */
  reason: string;
}

export type Entry = ListedEntry | UncheckedEntry;

/**
 * FNV-1a over UTF-16 code units, twice with different offset bases: 64 bits,
 * enough that a few thousand listings never share a key.
 */
function shortHash(text: string): string {
  let a = 0x811c9dc5;
  let b = 0x01000193;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    a = Math.imul(a ^ c, 0x01000193);
    b = Math.imul(b ^ c, 0x01000193);
  }
  return (a >>> 0).toString(16).padStart(8, "0") + (b >>> 0).toString(16).padStart(8, "0");
}

/**
 * A listing's key in the list, also its row's DOM id: where it came from and
 * a hash of its file. The file itself holds spaces and quotes, which no
 * IDREF (`aria-activedescendant`) can carry.
 */
export function entryKey(origin: Origin, listingJson: string): string {
  return `${origin}:${shortHash(listingJson)}`;
}

/** A listing that cannot be bought, behind the footer toggle. */
export interface HiddenEntry {
  name: string | null;
  reason: ShakedexHidden;
}

export interface ImportedRow {
  row: MarketRow;
  origin: Exclude<Origin, "market">;
}

export interface ListInput {
  /** Newest import first. */
  imported: ImportedRow[];
  /** The market pages loaded so far, in order. */
  pages: MarketPage[];
  /**
   * The node's answer to a Retry, by listing file, for the pages given:
   * the caller drops it once the market is fetched again.
   */
  rechecked: Map<string, MarketRow>;
  /**
   * Listings bought in this session, as they were when bought, by key. The
   * market reads a lock coin spent in the mempool as sold (hsd's `getCoin`
   * takes mempool spends into account), so its next fetch hides the row.
   */
  sent?: Map<string, ListedEntry>;
}

/**
 * One list out of the imports and every loaded page: the verified listings
 * and the ones the node could not check (with their file, so Retry can
 * check them again) are listed; the rest is hidden with its reason. A
 * Retry's answer stands in for a listing the page could not check, and a
 * listing bought here stays as it was bought, whatever the page says now.
 */
export function buildList({ imported, pages, rechecked, sent = new Map() }: ListInput): {
  entries: Entry[];
  hidden: HiddenEntry[];
} {
  const entries: Entry[] = [];
  const hidden: HiddenEntry[] = [];
  const keep = (origin: Origin, listingJson: string | null): boolean => {
    const bought = listingJson == null ? undefined : sent.get(entryKey(origin, listingJson));
    if (bought) entries.push({ ...bought, order: entries.length });
    return bought != null;
  };
  const place = (row: MarketRow, origin: Origin) => {
    if (keep(origin, row.listingJson)) return;
    const v = row.verdict;
    const base = {
      key: entryKey(origin, row.listingJson),
      name: row.name,
      origin,
      order: entries.length,
    };
    if (v.verdict === "buyable") entries.push({ ...base, kind: "listed", row });
    else if (v.kind === "couldNotCheck") {
      entries.push({ ...base, kind: "unchecked", listingJson: row.listingJson, reason: v.reason });
    } else hidden.push({ name: row.name, reason: v });
  };
  const placeHidden = (h: HiddenRow) => {
    if (keep("market", h.listingJson)) return;
    if (h.reason.kind !== "couldNotCheck" || h.listingJson == null || h.name == null) {
      hidden.push({ name: h.name, reason: h.reason });
      return;
    }
    const again = rechecked.get(h.listingJson);
    if (again) place(again, "market");
    else
      entries.push({
        kind: "unchecked",
        key: entryKey("market", h.listingJson),
        name: h.name,
        origin: "market",
        order: entries.length,
        listingJson: h.listingJson,
        reason: h.reason.reason,
      });
  };
  for (const { row, origin } of imported) place(row, origin);
  for (const page of pages) {
    for (const row of page.rows) {
      const v = row.verdict;
      const unchecked = v.verdict === "hidden" && v.kind === "couldNotCheck";
      place((unchecked && rechecked.get(row.listingJson)) || row, "market");
    }
    for (const h of page.hiddenRows) placeHidden(h);
  }
  return { entries, hidden };
}

/**
 * A reverse auction's prices from now on, as hsd will take them: the current
 * step (the cheapest valid one, wherever the file puts it), then each step
 * that will lower the price, soonest first. A step valid at the same time as
 * a cheaper one, or no cheaper than one before it, never sets the price and
 * is left out: the chain `template.rs::next_step_index` walks.
 */
export function stepsAhead(row: MarketRow): PriceStepView[] {
  const v = row.verdict;
  const current = v.verdict === "buyable" ? row.steps[v.currentStep] : undefined;
  if (!current) return [];
  const later = row.steps
    .filter((s) => (s.validInSecs ?? 0) > 0 && s.price < current.price)
    .sort((a, b) => a.validInSecs! - b.validInSecs! || a.price - b.price);
  const chain = [current];
  for (const s of later) if (s.price < chain[chain.length - 1]!.price) chain.push(s);
  return chain;
}

export type ListFilter = "all" | "fixed" | "drops";
export type ListSort = "price" | "drop" | "newest";

export interface ListQuery {
  query: string;
  filter: ListFilter;
  sort: ListSort;
}

/** The text a search matches: the name, without a leading dot, in lower case. */
function needle(query: string): string {
  return query.trim().replace(/^\./, "").toLowerCase();
}

/** Sort keys: a missing value sorts last. */
const LAST = Number.POSITIVE_INFINITY;

function sortKey(e: Entry, sort: ListSort): number {
  if (e.kind === "unchecked") return LAST;
  if (sort === "price") return e.row.currentPrice ?? LAST;
  if (sort === "drop") return e.row.nextValidInSecs ?? LAST;
  return e.order;
}

/**
 * The entries a search, a filter and a sort leave, in order. A listing the
 * node could not check has no known kind, so only "All" shows it; it sorts
 * last by price or drop. Ties keep the listing order.
 */
export function visibleEntries(entries: Entry[], { query, filter, sort }: ListQuery): Entry[] {
  const q = needle(query);
  return entries
    .filter((e) => {
      if (
        q &&
        !e.name.toLowerCase().includes(q) &&
        !displayName(e.name).toLowerCase().includes(q)
      ) {
        return false;
      }
      if (filter === "all") return true;
      if (e.kind === "unchecked") return false;
      return filter === "fixed" ? e.row.kind === "buyNow" : e.row.kind === "reverseAuction";
    })
    .sort((a, b) => {
      if (sort === "newest") return a.order - b.order;
      // An unchecked listing sorts after every listed one, a fixed price
      // after every drop; listing order breaks every tie.
      return sortKey(a, sort) - sortKey(b, sort) || a.order - b.order;
    });
}

/** "3 sold": a count and its reason, the verb agreeing with the count. */
const HIDDEN_LABEL: Record<ShakedexHidden["kind"], (n: number) => string> = {
  soldOrCancelled: (n) => `${n} sold`,
  failedVerification: (n) => `${n} failed the check`,
  expiresBeforeFinalize: (n) => `${n} ${n === 1 ? "expires" : "expire"} too soon to finalize`,
  notYetValid: (n) => `${n} not valid yet`,
  couldNotCheck: (n) => `${n} couldn't be checked`,
  unverified: (n) => `${n} couldn't be checked`,
};

const HIDDEN_ORDER: ShakedexHidden["kind"][] = [
  "soldOrCancelled",
  "failedVerification",
  "expiresBeforeFinalize",
  "notYetValid",
  "couldNotCheck",
  "unverified",
];

/** The footer toggle: "5 unavailable listings hidden: 3 sold, …", or how to hide them. */
export function hiddenToggleText(hidden: HiddenEntry[], open: boolean): string | null {
  if (hidden.length === 0) return null;
  const what = `${hidden.length} unavailable ${hidden.length === 1 ? "listing" : "listings"}`;
  if (open) return `Hide ${what}`;
  // Two kinds read "couldn't be checked": counted under one label.
  const counts = new Map<(n: number) => string, number>();
  for (const kind of HIDDEN_ORDER) {
    const n = hidden.filter((h) => h.reason.kind === kind).length;
    const label = HIDDEN_LABEL[kind];
    if (n > 0) counts.set(label, (counts.get(label) ?? 0) + n);
  }
  const parts = [...counts].map(([label, n]) => label(n));
  return `${what} hidden: ${parts.join(", ")}`;
}

export interface RowTag {
  label: string;
  variant: "success" | "warning" | "default";
}

/** A row's one tag, by priority: Bought, then Expires soon, then where it came from. */
export function rowTag(e: Entry, bought: boolean): RowTag | null {
  if (bought) return { label: "Bought", variant: "success" };
  if (e.kind === "listed" && e.row.verdict.verdict === "buyable" && e.row.verdict.warnExpiry) {
    return { label: "Expires soon", variant: "warning" };
  }
  if (e.origin === "file") return { label: "From file", variant: "default" };
  if (e.origin === "link") return { label: "From link", variant: "default" };
  return null;
}

/** What the detail panel's commit button knows about the selected listing. */
export interface PrimaryInput {
  name: string;
  /** Locked by this device (R33): never bought here. */
  own: boolean;
  /** The profile's or the node's refusal (`shakedexRefusal`), or null. */
  refusal: string | null;
  /** Mainnet buying is off in Settings. */
  gated: boolean;
  /** The previewed total, once priced. */
  totalDoos: number | null;
  /** A preview is on its way: the total on screen may be stale. */
  pricing: boolean;
  previewError: string | null;
  feeRateError: string | null;
  /** Spendable balance; null until known. */
  availableDoos: number | null;
  /** A reverse auction's next, lower price. */
  drop: { price: number; inSecs: number } | null;
  busy: boolean;
}

export type PrimaryAction =
  | { kind: "buy"; label: string; reason: null }
  | { kind: "fixit"; label: string; reason: string }
  | { kind: "disabled"; label: string; reason: string | null };

/**
 * The detail panel's one button, its label and the reason above it, in the
 * order `commands/shakedex.rs::prepare` refuses: the profile or node that
 * cannot buy, then the mainnet gate (whose button turns buying on), then an
 * own listing (R33); then what is wrong with this purchase. Never disabled
 * without a reason, except while busy.
 */
export function primaryAction(i: PrimaryInput): PrimaryAction {
  const named = `Buy .${displayName(i.name)}`;
  if (i.busy) return { kind: "disabled", label: "Sending…", reason: null };
  if (i.refusal) return { kind: "disabled", label: named, reason: sentence(i.refusal) };
  if (i.gated) return { kind: "fixit", label: "Turn on buying", reason: BUYING_OFF_REASON };
  if (i.own) return { kind: "disabled", label: named, reason: sentence(OWN_LISTING) };
  if (i.previewError) return { kind: "disabled", label: named, reason: i.previewError };
  // A stale total, kept on screen while the fee box's preview loads, is
  // not the one Buy would pay.
  if (i.pricing || i.totalDoos == null) {
    return { kind: "disabled", label: named, reason: "Working out the total…" };
  }
  // Exact figures: a shortfall of a few doos must not read "0 HNS short".
  const short = `Buy for ${hnsExact(i.totalDoos)}`;
  if (i.availableDoos != null && i.totalDoos > i.availableDoos) {
    let reason = `You have ${hnsExact(i.availableDoos)} available, ${hnsExact(i.totalDoos - i.availableDoos)} short of the total.`;
    if (i.drop)
      reason += ` The price drops to ${hns(i.drop.price)} in ${approxWait(i.drop.inSecs)}.`;
    return { kind: "disabled", label: short, reason };
  }
  if (i.feeRateError) return { kind: "disabled", label: short, reason: i.feeRateError };
  return { kind: "buy", label: `${named} for ${hnsExact(i.totalDoos)}`, reason: null };
}
