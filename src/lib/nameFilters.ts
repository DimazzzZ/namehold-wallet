/**
 * Sort and filter the Names table (pure). The rows arrive in urgency order
 * from buildNameRows; "status" keeps that order.
 */
import { displayName } from "./idn";
import type { NameRow } from "./nameRows";

export type NameSort = "status" | "name" | "value" | "expires";
export type SortDir = "asc" | "desc";
export type ExpiresWithin = "any" | "30" | "90" | "365";
export type LengthBucket = "any" | "1-3" | "4-5" | "6-10" | "11+";
export type CharKind = "any" | "letters" | "digits" | "hyphen" | "idn";
export type NamesChip = "all" | "needs" | "auction" | "owned" | "watching";

export interface NameFilters {
  expires: ExpiresWithin;
  len: LengthBucket;
  chars: CharKind;
  tags: string[];
}

export const NO_FILTERS: NameFilters = { expires: "any", len: "any", chars: "any", tags: [] };

export const NATURAL_DIR: Record<NameSort, SortDir> = {
  status: "asc",
  name: "asc",
  value: "desc",
  expires: "asc",
};

const SORTS: NameSort[] = ["status", "name", "value", "expires"];
const EXPIRES: ExpiresWithin[] = ["any", "30", "90", "365"];
const LENGTHS: LengthBucket[] = ["any", "1-3", "4-5", "6-10", "11+"];
const CHARS: CharKind[] = ["any", "letters", "digits", "hyphen", "idn"];
const CHIPS: NamesChip[] = ["all", "needs", "auction", "owned", "watching"];

/** On regtest/simnet the Expires keys mean blocks: only blocks exist there. */
export const EXPIRES_BLOCKS: Record<Exclude<ExpiresWithin, "any">, number> = {
  "30": 100,
  "90": 1_000,
  "365": 10_000,
};

/** What a name is made of, checked in this order: IDN, hyphen, digits, else letters. */
export function charKind(name: string): Exclude<CharKind, "any"> {
  const shown = displayName(name);
  // eslint-disable-next-line no-control-regex
  if (shown !== name || name.startsWith("xn--") || /[^\x00-\x7f]/.test(shown)) return "idn";
  if (shown.includes("-")) return "hyphen";
  if (/\d/.test(shown)) return "digits";
  return "letters";
}

/** Characters of the Unicode name, without the dot. */
export function nameLength(name: string): number {
  return Array.from(displayName(name)).length;
}

function inLength(n: number, b: LengthBucket): boolean {
  switch (b) {
    case "1-3":
      return n <= 3;
    case "4-5":
      return n >= 4 && n <= 5;
    case "6-10":
      return n >= 6 && n <= 10;
    case "11+":
      return n >= 11;
    default:
      return true;
  }
}

/**
 * Whether a row passes the Filters popover. Expires compares the backend's
 * daysUntilExpire where blocks keep time (`clock`), and blocks otherwise —
 * never blocks × a block time (DESIGN_SYSTEM §1.9).
 */
export function passesFilters(r: NameRow, f: NameFilters, clock: boolean): boolean {
  if (f.expires !== "any") {
    if (clock) {
      if (r.expiresDays == null || r.expiresDays > Number(f.expires)) return false;
    } else if (r.expiresBlocks == null || r.expiresBlocks > EXPIRES_BLOCKS[f.expires]) {
      return false;
    }
  }
  if (!inLength(nameLength(r.name), f.len)) return false;
  if (f.chars !== "any" && charKind(r.name) !== f.chars) return false;
  if (f.tags.length > 0 && !f.tags.some((t) => r.tags.includes(t))) return false;
  return true;
}

/**
 * Sort by a column. Stable; "status" is the input (urgency) order, reversed
 * for desc. Rows without the column's value go last in either direction,
 * and ties keep the urgency order.
 */
export function sortRows(rows: NameRow[], sort: NameSort, dir: SortDir): NameRow[] {
  const indexed = rows.map((r, i) => ({ r, i }));
  if (sort === "status") {
    return dir === "asc" ? rows.slice() : rows.slice().reverse();
  }
  const key = (r: NameRow): string | number | null =>
    sort === "name" ? displayName(r.name) : sort === "value" ? r.valueDoos : r.expiresBlocks;
  const sign = dir === "asc" ? 1 : -1;
  indexed.sort((a, b) => {
    const ka = key(a.r);
    const kb = key(b.r);
    if (ka == null && kb == null) return a.i - b.i;
    if (ka == null) return 1;
    if (kb == null) return -1;
    const c =
      typeof ka === "string" ? ka.localeCompare(kb as string) : (ka as number) - (kb as number);
    return c !== 0 ? c * sign : a.i - b.i;
  });
  return indexed.map((x) => x.r);
}

export interface NamesViewState {
  filter: NamesChip;
  sort: NameSort;
  dir: SortDir;
  filters: NameFilters;
}

const pick = <T extends string>(v: string | null, all: T[], fallback: T): T =>
  all.includes(v as T) ? (v as T) : fallback;

/** URL → view state; unknown values fall back to the defaults. */
export function parseNamesParams(p: URLSearchParams): NamesViewState {
  const sort = pick(p.get("sort"), SORTS, "status");
  return {
    filter: pick(p.get("filter"), CHIPS, "all"),
    sort,
    dir: pick(p.get("dir"), ["asc", "desc"] as SortDir[], NATURAL_DIR[sort]),
    filters: {
      expires: pick(p.get("expires"), EXPIRES, "any"),
      len: pick(p.get("len"), LENGTHS, "any"),
      chars: pick(p.get("chars"), CHARS, "any"),
      tags: [...new Set(p.getAll("tag").filter(Boolean))],
    },
  };
}

/** View state → URL, omitting defaults. */
export function namesParams(v: NamesViewState): URLSearchParams {
  const p = new URLSearchParams();
  if (v.filter !== "all") p.set("filter", v.filter);
  if (v.sort !== "status") p.set("sort", v.sort);
  if (v.dir !== NATURAL_DIR[v.sort]) p.set("dir", v.dir);
  if (v.filters.expires !== "any") p.set("expires", v.filters.expires);
  if (v.filters.len !== "any") p.set("len", v.filters.len);
  if (v.filters.chars !== "any") p.set("chars", v.filters.chars);
  for (const t of v.filters.tags) p.append("tag", t);
  return p;
}

/** How many filters are on (the Filters button badge). */
export function activeFilterCount(f: NameFilters): number {
  return (
    (f.expires !== "any" ? 1 : 0) +
    (f.len !== "any" ? 1 : 0) +
    (f.chars !== "any" ? 1 : 0) +
    f.tags.length
  );
}
