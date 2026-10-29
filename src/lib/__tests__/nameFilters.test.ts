import { describe, it, expect } from "vitest";
import {
  activeFilterCount,
  charKind,
  NO_FILTERS,
  nameLength,
  namesParams,
  parseNamesParams,
  passesFilters,
  sortRows,
  type NameFilters,
} from "../nameFilters";
import type { NameRow } from "../nameRows";

const row = (name: string, over: Partial<NameRow> = {}): NameRow => ({
  name,
  kind: "owned",
  cats: ["owned"],
  status: "Owned",
  detail: null,
  tone: "ok",
  pending: null,
  statusTitle: "",
  value: null,
  valueDoos: null,
  expires: null,
  expiresBlocks: null,
  expiresDays: null,
  tags: [],
  action: null,
  selectable: true,
  height: null,
  renewal: null,
  watch: null,
  ...over,
});
const f = (over: Partial<NameFilters>): NameFilters => ({ ...NO_FILTERS, ...over });

describe("charKind / nameLength", () => {
  it("classifies IDN first, then hyphen, digits, letters", () => {
    expect(charKind("xn--mnchen-3ya")).toBe("idn");
    expect(charKind("my-name2")).toBe("hyphen");
    expect(charKind("web3")).toBe("digits");
    expect(charKind("coffee")).toBe("letters");
  });
  it("counts the Unicode name's characters", () => {
    expect(nameLength("xn--mnchen-3ya")).toBe(7); // münchen
    expect(nameLength("abc")).toBe(3);
  });
});

describe("passesFilters", () => {
  it("expires: backend days on wall-clock networks, block buckets on regtest", () => {
    const soon = row("a", { expiresDays: 20, expiresBlocks: 2880 });
    const later = row("b", { expiresDays: 200, expiresBlocks: 28800 });
    const none = row("c");
    expect(passesFilters(soon, f({ expires: "30" }), true)).toBe(true);
    expect(passesFilters(later, f({ expires: "90" }), true)).toBe(false);
    expect(passesFilters(none, f({ expires: "365" }), true)).toBe(false);
    // regtest: "90" means ≤ 1,000 blocks, whatever the days say
    expect(
      passesFilters(row("d", { expiresDays: 1, expiresBlocks: 900 }), f({ expires: "90" }), false),
    ).toBe(true);
    expect(passesFilters(soon, f({ expires: "90" }), false)).toBe(false);
  });
  it("length, characters and tags (any of)", () => {
    expect(passesFilters(row("abcd"), f({ len: "4-5" }), true)).toBe(true);
    expect(passesFilters(row("abc"), f({ len: "4-5" }), true)).toBe(false);
    expect(passesFilters(row("abcdefghijk"), f({ len: "11+" }), true)).toBe(true);
    expect(passesFilters(row("ab-c"), f({ chars: "letters" }), true)).toBe(false);
    const tagged = row("t", { tags: ["design", "short"] });
    expect(passesFilters(tagged, f({ tags: ["premium", "design"] }), true)).toBe(true);
    expect(passesFilters(row("u"), f({ tags: ["design"] }), true)).toBe(false);
  });
});

describe("sortRows", () => {
  const rows = [
    row("b", { valueDoos: 5, expiresBlocks: 300 }),
    row("a", { valueDoos: null, expiresBlocks: null }),
    row("c", { valueDoos: 9, expiresBlocks: 100 }),
    row("d", { valueDoos: 5, expiresBlocks: null }),
  ];
  const names = (rs: NameRow[]) => rs.map((r) => r.name).join("");
  it("keeps the urgency order for status, reversed for desc", () => {
    expect(names(sortRows(rows, "status", "asc"))).toBe("bacd");
    expect(names(sortRows(rows, "status", "desc"))).toBe("dcab");
  });
  it("puts rows without the value last in both directions, ties in urgency order", () => {
    expect(names(sortRows(rows, "value", "desc"))).toBe("cbda");
    expect(names(sortRows(rows, "value", "asc"))).toBe("bdca");
    expect(names(sortRows(rows, "expires", "asc"))).toBe("cbad");
    expect(names(sortRows(rows, "expires", "desc"))).toBe("bcad");
  });
  it("sorts names by the Unicode name", () => {
    const r = [row("zeta"), row("xn--mnchen-3ya"), row("alpha")];
    expect(sortRows(r, "name", "asc").map((x) => x.name)).toEqual([
      "alpha",
      "xn--mnchen-3ya",
      "zeta",
    ]);
  });
});

describe("parseNamesParams / namesParams", () => {
  it("round-trips and omits defaults", () => {
    const p = new URLSearchParams(
      "filter=owned&sort=expires&expires=90&len=4-5&chars=letters&tag=design&tag=premium",
    );
    const v = parseNamesParams(p);
    expect(v).toEqual({
      filter: "owned",
      sort: "expires",
      dir: "asc",
      filters: { expires: "90", len: "4-5", chars: "letters", tags: ["design", "premium"] },
    });
    expect(namesParams(v).toString()).toBe(p.toString());
    expect(activeFilterCount(v.filters)).toBe(5);
  });
  it("falls back on bad values", () => {
    const v = parseNamesParams(new URLSearchParams("sort=x&len=9&dir=up&filter=nope"));
    expect(v).toEqual({ filter: "all", sort: "status", dir: "asc", filters: NO_FILTERS });
    expect(namesParams(v).toString()).toBe("");
    expect(namesParams({ ...v, sort: "value", dir: "asc" }).toString()).toBe("sort=value&dir=asc");
  });
});
