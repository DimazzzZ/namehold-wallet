import { describe, it, expect } from "vitest";
import { buildNameRows, chipCounts, wallClock, type NameRowsInput } from "../nameRows";
import { formatSpanDays } from "../auction";
import { allowed, makeCapabilities } from "../../test/fixtures/capabilities";
import type { NeedsYouItem } from "../needsYou";
import type { HsdName, NameActionCapabilities } from "../../types";

const name = (n: string, over: Partial<HsdName> = {}): HsdName => ({
  name: n,
  state: "CLOSED",
  height: 100,
  renewal: 200,
  owner: { hash: "t", index: 0 },
  stats: { daysUntilExpire: 300, blocksUntilExpire: 43200 },
  value: 5_000_000,
  ...over,
});

const need = (n: string, over: Partial<NeedsYouItem> = {}): NeedsYouItem => ({
  kind: "renew",
  name: n,
  text: "Renew before it expires — an expired name is lost for good",
  when: "~3 days · 432 blocks",
  tone: "bad",
  actionLabel: "Renew",
  blocks: 432,
  ...over,
});

const caps = (list: NameActionCapabilities[]) => new Map(list.map((c) => [c.name, c]));

const input = (over: Partial<NameRowsInput> = {}): NameRowsInput => ({
  owned: [],
  positions: [],
  capsByName: new Map(),
  watched: [],
  watchInfo: {},
  watchStatus: [],
  needs: [],
  finalizeSent: new Set(),
  wallClock: true,
  ...over,
});

const one = (over: Partial<NameRowsInput>) => {
  const rows = buildNameRows(input(over));
  expect(rows).toHaveLength(1);
  return rows[0]!;
};

describe("buildNameRows — status per task state", () => {
  const cases: [string, Partial<NameActionCapabilities>, string, string | null, string | null][] = [
    [
      "readyToReveal",
      {
        taskState: "readyToReveal",
        phase: "REVEAL",
        bidValueDoos: 18_250_000,
        countdownBlocks: 5,
        countdownHours: 0.8,
      },
      "Reveal your bid",
      "5 blocks (~48m) left",
      "Reveal",
    ],
    [
      "revealBroadcastPending",
      { taskState: "revealBroadcastPending", phase: "REVEAL" },
      "Reveal sent",
      "waiting to confirm",
      null,
    ],
    [
      "wonNeedsRegister",
      { taskState: "wonNeedsRegister" },
      "You won",
      "register it to make it yours",
      "Register",
    ],
    [
      "lostNeedsRedeem owned",
      { taskState: "lostNeedsRedeem", ownsName: true, redeemableValueDoos: 7_000_000 },
      "Owned",
      "reclaim 7 HNS from your other bids",
      "Redeem",
    ],
    [
      "lostNeedsRedeem",
      { taskState: "lostNeedsRedeem", redeemableValueDoos: 0 },
      "You lost",
      "get your lockup back",
      "Redeem",
    ],
    [
      "lostNeedsRedeem revoked",
      { taskState: "lostNeedsRedeem", phase: "REVOKED", redeemableValueDoos: 2_000_000 },
      "Revoked",
      "reclaim your 2 HNS lockup",
      "Redeem",
    ],
    [
      "lostUnrevealed",
      { taskState: "lostUnrevealed" },
      "Lost",
      "bid not revealed in time — the lockup can't be reclaimed",
      null,
    ],
    [
      "expiringSoon",
      { taskState: "expiringSoon" },
      "Renew it or lose it",
      "expires in 10mo",
      "Renew",
    ],
    [
      "ownedNoUrgentAction",
      { taskState: "ownedNoUrgentAction" },
      "Owned",
      "renew within 10mo",
      null,
    ],
    [
      "transfer not ready",
      { taskState: "transferPendingFinalize" },
      "Transferring",
      "finalize once the transfer lockup ends",
      null,
    ],
    [
      "transfer ready",
      { taskState: "transferPendingFinalize", canFinalize: allowed },
      "Transferring",
      "ready to finalize",
      "Finalize",
    ],
    ["bidding, no bid", { taskState: "readyToBid", phase: "BIDDING" }, "Bidding", null, "Bid"],
    [
      "opening",
      { taskState: "waitingForBidding", phase: "OPENING" },
      "Opening",
      "bidding opens soon",
      null,
    ],
  ];
  it.each(cases)("%s", (_, c, status, detail, action) => {
    const r = one({
      owned: [name("x")],
      capsByName: caps([makeCapabilities({ name: "x", ...c })]),
    });
    expect(r.status).toBe(status);
    expect(r.detail).toBe(detail);
    expect(r.action?.label ?? null).toBe(action);
  });

  it("hides the action and shows a pending badge while our transaction is in flight", () => {
    const r = one({
      owned: [name("x")],
      capsByName: caps([
        makeCapabilities({
          name: "x",
          taskState: "readyToReveal",
          pendingBroadcastAction: "reveal",
        }),
      ]),
    });
    expect(r.pending).toMatch(/waiting for a block/);
    expect(r.action).toBeNull();
  });

  it("falls back to the chain phase without capabilities", () => {
    const r = one({ owned: [name("x", { state: "BIDDING" })] });
    expect(r.status).toBe("Bidding");
    expect(r.tone).toBe("muted");
  });
});

describe("buildNameRows — purchases, listings, positions, watched", () => {
  const purchase = (over: Partial<NonNullable<HsdName["shakedex"]>>) =>
    name("buy", {
      shakedex: {
        state: "awaitingFinalize",
        blocksRemaining: 0,
        purchaseId: "p1",
        purchaseDraftId: "d1",
        priceDoos: 410_200_000,
        lockupBlocks: 288,
        wallClockTiming: true,
        ...over,
      },
    });

  it("a ready purchase finalizes; one not ready is viewed; neither is selectable", () => {
    const ready = one({ owned: [purchase({})] });
    expect(ready).toMatchObject({
      kind: "purchase",
      status: "Bought",
      detail: "ready to finalize",
      value: "410.2 paid",
      selectable: false,
    });
    expect(ready.action?.kind).toBe("finalizePurchase");
    const waiting = one({ owned: [purchase({ blocksRemaining: 12 })] });
    expect(waiting.detail).toBe("awaiting finalize · 12 blocks left");
    expect(waiting.action?.kind).toBe("viewPurchase");
  });

  it("a sent finalize shows its pending badge and no button", () => {
    const r = one({ owned: [purchase({})], finalizeSent: new Set(["p1"]) });
    expect(r.pending).toBe("Finalize · waiting for a block");
    expect(r.action).toBeNull();
  });

  it("a listing opens My listings and stays selectable", () => {
    const r = one({
      owned: [name("l", { listing: { state: "listed", blocksUntilFinalize: null } as never })],
    });
    expect(r).toMatchObject({ kind: "listing", status: "Listed for sale", selectable: true });
    expect(r.action?.kind).toBe("myListings");
  });

  it("keeps a position only while its caps are in an active auction state, deduped against owned", () => {
    const rows = buildNameRows(
      input({
        owned: [name("own")],
        positions: ["own", "live", "settled"],
        capsByName: caps([
          makeCapabilities({ name: "live", taskState: "readyToBid", phase: "BIDDING" }),
          makeCapabilities({ name: "settled", taskState: "unavailableOther" }),
        ]),
      }),
    );
    expect(rows.map((r) => r.name).sort()).toEqual(["live", "own"]);
    expect(rows.find((r) => r.name === "live")!.kind).toBe("auction");
  });

  it("a watched name we own is one row with the watch line", () => {
    const rows = buildNameRows(
      input({
        owned: [name("both")],
        watched: [{ name: "both", addedAt: "2026-07-01", notes: "", tags: "x" }],
      }),
    );
    expect(rows).toHaveLength(1);
    expect(rows[0]!.watch).toEqual({ tags: "x", addedAt: "2026-07-01" });
    expect(rows[0]!.cats).toContain("watching");
  });

  it("a watched name of someone else's reads its phase, highest bid and Bid while bidding", () => {
    const r = one({
      watched: [{ name: "w", addedAt: "", notes: "", tags: "" }],
      watchInfo: { w: name("w", { state: "BIDDING", highest: 3_000_000, stats: null }) },
    });
    expect(r).toMatchObject({ kind: "watch", status: "Bidding", value: "3 high", tone: "info" });
    expect(r.action?.label).toBe("Bid");
    const loading = one({ watched: [{ name: "w", addedAt: "", notes: "", tags: "" }] });
    expect(loading.status).toBe("Loading…");
  });
});

describe("categories, counts and sort", () => {
  it("counts each chip over all rows and sorts needs first, in their order", () => {
    const rows = buildNameRows(
      input({
        owned: [name("a"), name("b"), name("c", { state: "BIDDING" })],
        capsByName: caps([
          makeCapabilities({ name: "a", taskState: "ownedNoUrgentAction" }),
          makeCapabilities({ name: "b", taskState: "expiringSoon" }),
          makeCapabilities({ name: "c", taskState: "readyToBid", phase: "BIDDING" }),
        ]),
        watched: [{ name: "z", addedAt: "", notes: "", tags: "" }],
        needs: [need("b")],
      }),
    );
    expect(rows.map((r) => r.name)).toEqual(["b", "c", "a", "z"]);
    expect(chipCounts(rows)).toEqual({ all: 4, needs: 1, auction: 1, owned: 2, watching: 1 });
  });
});

describe("expiry", () => {
  it("reads days on a wall-clock network and blocks otherwise", () => {
    const n = name("x", { stats: { daysUntilExpire: 20, blocksUntilExpire: 2880 } });
    const c = caps([makeCapabilities({ name: "x", taskState: "ownedNoUrgentAction" })]);
    expect(one({ owned: [n], capsByName: c }).expires).toEqual({
      text: "in 20d",
      className: "text-red-600",
    });
    expect(one({ owned: [n], capsByName: c, wallClock: false }).expires).toEqual({
      text: "in 2,880 blocks",
      className: "text-gray-500",
    });
  });

  it("wallClock uses the wallet's network values, not hsd's 'main'", () => {
    expect(wallClock("mainnet")).toBe(true);
    expect(wallClock("testnet")).toBe(true);
    expect(wallClock("regtest")).toBe(false);
    expect(wallClock("main")).toBe(false);
  });

  it("formatSpanDays", () => {
    expect(formatSpanDays(0.5)).toBe("<1d");
    expect(formatSpanDays(45)).toBe("45d");
    expect(formatSpanDays(250)).toBe("8mo");
    expect(formatSpanDays(365)).toBe("1y");
    expect(formatSpanDays(610)).toBe("1y 8mo");
  });
});

describe("needs-you items without a row", () => {
  it("get a minimal row so the Needs chip equals the nav count", () => {
    const rows = buildNameRows(
      input({
        owned: [name("a")],
        capsByName: caps([makeCapabilities({ name: "a", taskState: "ownedNoUrgentAction" })]),
        // A chain renewal for a name read_names doesn't list.
        needs: [need("ghost")],
        renewals: [
          {
            name: "ghost",
            state: "CLOSED",
            renewalHeight: 1,
            expiresAtHeight: 2,
            blocksUntilExpire: 432,
            daysUntilExpire: 3,
            source: "chain",
            expiringSoon: true,
          },
        ],
      }),
    );
    const ghost = rows.find((r) => r.name === "ghost")!;
    expect(rows[0]).toBe(ghost);
    expect(ghost).toMatchObject({
      kind: "owned",
      cats: ["needs", "owned"],
      status: "Renew before it expires — an expired name is lost for good",
      detail: "~3 days · 432 blocks",
      tone: "bad",
      action: { label: "Renew", kind: "open", signs: true },
      selectable: false,
    });
    expect(ghost.expires?.text).toBe(`in ${formatSpanDays(3)}`);
    // Sortable by Expires like any other row.
    expect(ghost).toMatchObject({ expiresDays: 3, expiresBlocks: 432, valueDoos: null });
    expect(chipCounts(rows)).toMatchObject({ needs: 1, owned: 2 });
  });

  it("stay out of Owned unless the task is a renew", () => {
    const rows = buildNameRows(
      input({ needs: [need("x", { kind: "reveal", actionLabel: "Reveal", tone: "warn" })] }),
    );
    expect(rows[0]!.cats).toEqual(["needs"]);
    expect(rows[0]!.expires).toBeNull();
  });

  it("don't duplicate a name that already has a row", () => {
    const rows = buildNameRows(
      input({
        owned: [name("b")],
        capsByName: caps([makeCapabilities({ name: "b", taskState: "expiringSoon" })]),
        needs: [need("b")],
      }),
    );
    expect(rows).toHaveLength(1);
    expect(rows[0]!.status).toBe("Renew it or lose it");
  });
});

describe("Owned chip", () => {
  it("keeps a name we own with our own outbid reveals to reclaim; a lost one is In auction", () => {
    const rows = buildNameRows(
      input({
        owned: [name("mine"), name("lost", { state: "CLOSED" })],
        capsByName: caps([
          makeCapabilities({ name: "mine", taskState: "lostNeedsRedeem", ownsName: true }),
          makeCapabilities({ name: "lost", taskState: "lostNeedsRedeem", ownsName: false }),
        ]),
      }),
    );
    const by = (n: string) => rows.find((r) => r.name === n)!;
    expect(by("mine").cats).toContain("owned");
    expect(by("mine").expires).not.toBeNull();
    expect(by("lost").cats).not.toContain("owned");
  });
});
