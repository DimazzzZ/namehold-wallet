import { describe, it, expect } from "vitest";
import { makeAuctionRow, makeMarketPage, makeMarketRow } from "../../test/fixtures/listings";
import {
  buildList,
  entryKey,
  hiddenToggleText,
  primaryAction,
  rowTag,
  stepsAhead,
  visibleEntries,
  type ListedEntry,
  type PrimaryInput,
} from "../market/marketModel";
import type { HiddenRow, MarketRow } from "../../types";

const couldNotCheck = (name: string, reason = "Node RPC error: request timed out"): HiddenRow => ({
  name,
  reason: { kind: "couldNotCheck", reason },
  listingJson: JSON.stringify({ version: 2, name }),
});

describe("buildList", () => {
  it("lists imports first, then each page's rows and the listings it could not check", () => {
    const { entries, hidden } = buildList({
      imported: [{ row: makeMarketRow({ name: "mine" }), origin: "file" }],
      pages: [
        makeMarketPage({
          rows: [makeMarketRow({ name: "a" })],
          hiddenRows: [
            couldNotCheck("b"),
            { name: "c", reason: { kind: "soldOrCancelled" }, listingJson: "{}" },
          ],
        }),
        makeMarketPage({ rows: [makeMarketRow({ name: "d" })], page: 2, pageCount: 2 }),
      ],
      rechecked: new Map(),
    });
    expect(entries.map((e) => [e.kind, e.name, e.origin])).toEqual([
      ["listed", "mine", "file"],
      ["listed", "a", "market"],
      ["unchecked", "b", "market"],
      ["listed", "d", "market"],
    ]);
    expect(hidden.map((h) => h.name)).toEqual(["c"]);
  });

  it("keeps a listing it could not check by its node's reason", () => {
    const { entries } = buildList({
      imported: [],
      pages: [makeMarketPage({ rows: [], hiddenRows: [couldNotCheck("b")] })],
      rechecked: new Map(),
    });
    expect(entries[0]).toMatchObject({
      kind: "unchecked",
      reason: "Node RPC error: request timed out",
    });
  });

  it("hides a listing it could not check that carries no file to check again", () => {
    const { entries, hidden } = buildList({
      imported: [],
      pages: [
        makeMarketPage({ rows: [], hiddenRows: [{ ...couldNotCheck("b"), listingJson: null }] }),
      ],
      rechecked: new Map(),
    });
    expect(entries).toEqual([]);
    expect(hidden).toHaveLength(1);
  });

  it("puts a checked-again listing where the node's new answer puts it", () => {
    const b = couldNotCheck("b");
    const c = couldNotCheck("c");
    const d = couldNotCheck("d");
    const rechecked = new Map<string, MarketRow>([
      [b.listingJson!, makeMarketRow({ name: "b" })],
      [
        c.listingJson!,
        makeMarketRow({ name: "c", verdict: { verdict: "hidden", kind: "soldOrCancelled" } }),
      ],
      [
        d.listingJson!,
        makeMarketRow({
          name: "d",
          verdict: { verdict: "hidden", kind: "couldNotCheck", reason: "still down" },
        }),
      ],
    ]);
    const { entries, hidden } = buildList({
      imported: [],
      pages: [makeMarketPage({ rows: [], hiddenRows: [b, c, d] })],
      rechecked,
    });
    expect(entries.map((e) => [e.kind, e.name])).toEqual([
      ["listed", "b"],
      ["unchecked", "d"],
    ]);
    expect(entries[1]).toMatchObject({ reason: "still down" });
    expect(hidden.map((h) => [h.name, h.reason.kind])).toEqual([["c", "soldOrCancelled"]]);
  });

  it("never lets a Retry's answer override a row the page itself verified", () => {
    const fresh = makeMarketRow({ name: "a", currentPrice: 100_000_000 });
    const stale = makeMarketRow({
      name: "a",
      verdict: { verdict: "hidden", kind: "couldNotCheck", reason: "old answer" },
    });
    const { entries } = buildList({
      imported: [],
      pages: [makeMarketPage({ rows: [fresh] })],
      rechecked: new Map([[fresh.listingJson, stale]]),
    });
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({ kind: "listed", row: { currentPrice: 100_000_000 } });
  });

  it("keeps a bought listing in its place when the page reads its lock coin as spent", () => {
    const row = makeMarketRow({ name: "a" });
    const page = (r: MarketRow) =>
      buildList({ imported: [], pages: [makeMarketPage({ rows: [r] })], rechecked: new Map() });
    const bought = page(row).entries[0] as ListedEntry;
    // hsd's getCoin reads a coin spent in the mempool as missing: sold.
    const { entries, hidden } = buildList({
      imported: [],
      pages: [
        makeMarketPage({
          rows: [makeMarketRow({ name: "b" })],
          hiddenRows: [
            { name: "a", reason: { kind: "soldOrCancelled" }, listingJson: row.listingJson },
          ],
        }),
      ],
      rechecked: new Map(),
      sent: new Map([[bought.key, bought]]),
    });
    expect(entries.map((e) => [e.key, e.kind])).toEqual([
      [entryKey("market", makeMarketRow({ name: "b" }).listingJson), "listed"],
      [bought.key, "listed"],
    ]);
    expect(entries[1]).toMatchObject({ row: bought.row });
    expect(hidden).toEqual([]);
  });

  it("keys a listing by where it came from and a short hash, never by its raw file", () => {
    const json = JSON.stringify({ version: 2, name: "a b", note: "with spaces" }, null, 2);
    const key = entryKey("market", json);
    expect(key).toMatch(/^market:[0-9a-f]+$/);
    expect(key.length).toBeLessThan(30);
    expect(entryKey("market", json)).toBe(key);
    expect(entryKey("file", json)).not.toBe(key);
    expect(entryKey("market", json + " ")).not.toBe(key);
    const { entries } = buildList({
      imported: [],
      pages: [makeMarketPage({ rows: [makeMarketRow({ listingJson: json })] })],
      rechecked: new Map(),
    });
    expect(entries[0]).toMatchObject({ key, row: { listingJson: json } });
  });
});

describe("stepsAhead", () => {
  const step = (price: number, validInSecs: number) => ({
    price,
    lockTime: 1_700_000_000 + validInSecs,
    validInSecs,
  });
  const at = (current: number, steps: ReturnType<typeof step>[]) =>
    stepsAhead(
      makeAuctionRow({
        steps,
        verdict: { ...makeAuctionRow().verdict, currentStep: current } as MarketRow["verdict"],
      }),
    ).map((s) => [s.price, s.validInSecs]);

  it("is the current step, then each later step that lowers the price, as hsd will see them", () => {
    expect(at(0, [step(1200, 0), step(1100, 600), step(800, 1200)])).toEqual([
      [1200, 0],
      [1100, 600],
      [800, 1200],
    ]);
  });

  it("reads a file whose steps are out of order by time and price, not by position", () => {
    // The cheapest valid step is the current one wherever it sits; an
    // expensive step at the end never comes into play.
    expect(at(1, [step(900, 1200), step(1000, 0), step(1500, 0), step(2000, 3600)])).toEqual([
      [1000, 0],
      [900, 1200],
    ]);
  });

  it("leaves out a later step no cheaper than one before it", () => {
    expect(at(0, [step(1200, 0), step(800, 600), step(1000, 1200), step(800, 1800)])).toEqual([
      [1200, 0],
      [800, 600],
    ]);
  });

  it("takes the cheapest of steps valid at the same time", () => {
    expect(at(0, [step(1200, 0), step(1100, 600), step(1000, 600)])).toEqual([
      [1200, 0],
      [1000, 600],
    ]);
  });
});

describe("visibleEntries", () => {
  const { entries } = buildList({
    imported: [{ row: makeMarketRow({ name: "zeta", currentPrice: 900_000_000 }), origin: "link" }],
    pages: [
      makeMarketPage({
        rows: [
          makeMarketRow({ name: "alpha", currentPrice: 300_000_000 }),
          makeAuctionRow({ name: "bravo", currentPrice: 500_000_000, nextValidInSecs: 7200 }),
          makeAuctionRow({ name: "charlie", currentPrice: 100_000_000, nextValidInSecs: 3600 }),
        ],
        hiddenRows: [couldNotCheck("delta")],
      }),
    ],
    rechecked: new Map(),
  });
  const names = (q: Parameters<typeof visibleEntries>[1]) =>
    visibleEntries(entries, q).map((e) => e.name);

  it("sorts by the lowest price, unchecked last", () => {
    expect(names({ query: "", filter: "all", sort: "price" })).toEqual([
      "charlie",
      "alpha",
      "bravo",
      "zeta",
      "delta",
    ]);
  });

  it("sorts by the soonest price drop, fixed prices and unchecked last", () => {
    expect(names({ query: "", filter: "all", sort: "drop" })).toEqual([
      "charlie",
      "bravo",
      "zeta",
      "alpha",
      "delta",
    ]);
  });

  it("sorts newest as imports first, then in listing order", () => {
    expect(names({ query: "", filter: "all", sort: "newest" })).toEqual([
      "zeta",
      "alpha",
      "bravo",
      "charlie",
      "delta",
    ]);
  });

  it("filters by kind, leaving out what has no known kind", () => {
    expect(names({ query: "", filter: "fixed", sort: "newest" })).toEqual(["zeta", "alpha"]);
    expect(names({ query: "", filter: "drops", sort: "newest" })).toEqual(["bravo", "charlie"]);
  });

  it("finds a name by a substring, a leading dot stripped", () => {
    expect(names({ query: ".LPH", filter: "all", sort: "newest" })).toEqual(["alpha"]);
    expect(names({ query: "  ", filter: "all", sort: "newest" })).toHaveLength(5);
  });
});

describe("hiddenToggleText", () => {
  it("counts each reason, and says how to hide them again", () => {
    const hidden = [
      { name: "a", reason: { kind: "soldOrCancelled" as const } },
      { name: "b", reason: { kind: "soldOrCancelled" as const } },
      { name: "c", reason: { kind: "soldOrCancelled" as const } },
      { name: "d", reason: { kind: "failedVerification" as const, reason: "bad sig" } },
      { name: "e", reason: { kind: "notYetValid" as const, firstValidInSecs: 18_000 } },
    ];
    expect(hiddenToggleText(hidden, false)).toBe(
      "5 unavailable listings hidden: 3 sold, 1 failed the check, 1 not valid yet",
    );
    expect(hiddenToggleText(hidden, true)).toBe("Hide 5 unavailable listings");
  });

  it("names a single listing in the singular, and the expiry reason too", () => {
    const hidden = [{ name: "a", reason: { kind: "expiresBeforeFinalize" as const } }];
    expect(hiddenToggleText(hidden, false)).toBe(
      "1 unavailable listing hidden: 1 expires too soon to finalize",
    );
    expect(hiddenToggleText(hidden, true)).toBe("Hide 1 unavailable listing");
  });

  it("puts the expiry reason's verb in the plural for more than one", () => {
    const hidden = [1, 2, 3].map((i) => ({
      name: `n${i}`,
      reason: { kind: "expiresBeforeFinalize" as const },
    }));
    expect(hiddenToggleText(hidden, false)).toBe(
      "3 unavailable listings hidden: 3 expire too soon to finalize",
    );
  });

  it("gives nothing when nothing is hidden", () => {
    expect(hiddenToggleText([], false)).toBeNull();
  });
});

describe("rowTag", () => {
  const listed = (row: MarketRow, origin: "market" | "file" | "link" = "market") =>
    buildList({
      imported: origin === "market" ? [] : [{ row, origin }],
      pages: origin === "market" ? [makeMarketPage({ rows: [row] })] : [],
      rechecked: new Map(),
    }).entries[0]!;

  it("picks Bought over Expires soon over where it came from", () => {
    const row = makeMarketRow({
      verdict: { ...makeMarketRow().verdict, warnExpiry: true } as MarketRow["verdict"],
    });
    expect(rowTag(listed(row, "file"), true)?.label).toBe("Bought");
    expect(rowTag(listed(row, "file"), false)?.label).toBe("Expires soon");
    expect(rowTag(listed(makeMarketRow(), "file"), false)?.label).toBe("From file");
    expect(rowTag(listed(makeMarketRow(), "link"), false)?.label).toBe("From link");
    expect(rowTag(listed(makeMarketRow()), false)).toBeNull();
  });
});

describe("primaryAction", () => {
  const ok: PrimaryInput = {
    name: "coffee",
    own: false,
    refusal: null,
    gated: false,
    totalDoos: 255_004_200,
    pricing: false,
    previewError: null,
    feeRateError: null,
    availableDoos: 1_000_000_000,
    drop: null,
    busy: false,
  };

  it("buys, naming the name and the total", () => {
    expect(primaryAction(ok)).toEqual({
      kind: "buy",
      label: "Buy .coffee for 255.0042 HNS",
      reason: null,
    });
  });

  it("turns buying on first when mainnet buying is off", () => {
    expect(
      primaryAction({ ...ok, gated: true, availableDoos: 0, feeRateError: "Minimum is 1,000" }),
    ).toEqual({
      kind: "fixit",
      label: "Turn on buying",
      reason:
        "Buying on mainnet is off, so this button turns it on first. Nothing is bought until you press Buy.",
    });
  });

  it("says how much is missing, and when the price drops", () => {
    const short = { ...ok, totalDoos: 300_000_000, availableDoos: 120_500_000 };
    expect(primaryAction(short)).toEqual({
      kind: "disabled",
      label: "Buy for 300 HNS",
      reason: "You have 120.5 HNS available, 179.5 HNS short of the total.",
    });
    expect(primaryAction({ ...short, drop: { price: 200_000_000, inSecs: 21_600 } }).reason).toBe(
      "You have 120.5 HNS available, 179.5 HNS short of the total. The price drops to 200 HNS in ~6 h.",
    );
  });

  it("never rounds a shortfall away: every figure on the way to Buy is exact", () => {
    expect(
      primaryAction({ ...ok, totalDoos: 1_000_004_200, availableDoos: 1_000_000_000 }),
    ).toEqual({
      kind: "disabled",
      label: "Buy for 1,000.0042 HNS",
      reason: "You have 1,000 HNS available, 0.0042 HNS short of the total.",
    });
  });

  it("names a fee rate it cannot use", () => {
    expect(primaryAction({ ...ok, feeRateError: "Whole number only" })).toEqual({
      kind: "disabled",
      label: "Buy for 255.0042 HNS",
      reason: "Whole number only",
    });
  });

  it("refuses in the backend's order: the profile or node, then the gate, then an own listing", () => {
    expect(
      primaryAction({
        ...ok,
        refusal: "Shakedex works with a recovery-phrase wallet for now",
        gated: true,
        own: true,
      }),
    ).toEqual({
      kind: "disabled",
      label: "Buy .coffee",
      reason: "Shakedex works with a recovery-phrase wallet for now.",
    });
    expect(primaryAction({ ...ok, gated: true, own: true }).kind).toBe("fixit");
    expect(primaryAction({ ...ok, own: true })).toMatchObject({ kind: "disabled" });
    expect(primaryAction({ ...ok, own: true }).reason).toMatch(/^This is your own listing/);
  });

  it("is never disabled without a reason while it prices or after a pricing failure", () => {
    const pricing = primaryAction({ ...ok, totalDoos: null, pricing: true });
    expect(pricing).toEqual({
      kind: "disabled",
      label: "Buy .coffee",
      reason: "Working out the total…",
    });
    const failed = primaryAction({
      ...ok,
      totalDoos: null,
      previewError: "the node did not answer",
    });
    expect(failed).toEqual({
      kind: "disabled",
      label: "Buy .coffee",
      reason: "the node did not answer",
    });
  });

  it("does not claim a shortfall before the balance is known", () => {
    expect(primaryAction({ ...ok, availableDoos: null }).kind).toBe("buy");
  });

  it("says it is sending while busy", () => {
    expect(primaryAction({ ...ok, busy: true })).toEqual({
      kind: "disabled",
      label: "Sending…",
      reason: null,
    });
  });
});
