import { describe, it, expect } from "vitest";
import { needsYouItems, needsYouWhen, type NeedsYouInput } from "../needsYou";
import { allowed, makeCapabilities } from "../../test/fixtures/capabilities";
import type { HsdName, RenewalRow } from "../../types";

const NOW = Date.UTC(2026, 9, 28);

const input = (over: Partial<NeedsYouInput> = {}): NeedsYouInput => ({
  isWatchOnly: false,
  network: "mainnet",
  names: [],
  caps: [],
  renewals: [],
  finalizeSent: new Set(),
  ...over,
});

const renewal = (over: Partial<RenewalRow> = {}): RenewalRow => ({
  name: "oldtimer",
  state: "CLOSED",
  renewalHeight: 1,
  expiresAtHeight: 2,
  blocksUntilExpire: 1450,
  daysUntilExpire: 10,
  source: "chain",
  expiringSoon: true,
  ...over,
});

const purchase = (over: Partial<NonNullable<HsdName["shakedex"]>> = {}): HsdName => ({
  name: "pixelart",
  state: "TRANSFER",
  height: 1,
  renewal: 1,
  owner: null,
  stats: null,
  shakedex: {
    state: "awaitingFinalize",
    blocksRemaining: 0,
    purchaseId: "pur-1",
    purchaseDraftId: "d-1",
    priceDoos: 1,
    lockupBlocks: 288,
    wallClockTiming: true,
    ...over,
  },
});

describe("needsYouItems", () => {
  it("builds each kind from its source, with its copy", () => {
    const items = needsYouItems(
      input({
        caps: [
          makeCapabilities({
            name: "cryptohq",
            taskState: "readyToReveal",
            lockupValueDoos: 24_000_000,
            countdownBlocks: 5,
            countdownHours: 0.83,
          }),
          makeCapabilities({
            name: "handywallet",
            taskState: "transferPendingFinalize",
            canFinalize: allowed,
          }),
          makeCapabilities({ name: "satoshi", taskState: "wonNeedsRegister" }),
          makeCapabilities({
            name: "oldbid",
            taskState: "lostNeedsRedeem",
            redeemableValueDoos: 11_000_000,
          }),
        ],
        renewals: [renewal()],
        names: [purchase()],
      }),
      NOW,
    );
    expect(items.map((i) => [i.kind, i.name, i.actionLabel, i.tone])).toEqual([
      ["reveal", "cryptohq", "Reveal", "warn"],
      ["renew", "oldtimer", "Renew", "bad"],
      ["finalizeTransfer", "handywallet", "Finalize", "ok"],
      ["finalizePurchase", "pixelart", "Finalize", "ok"],
      ["register", "satoshi", "Register", "ok"],
      ["redeem", "oldbid", "Redeem", "muted"],
    ]);
    expect(items[0]!.text).toBe("Reveal your bid, or the 24 HNS lockup can't be reclaimed");
    expect(items[0]!.when).toBe("~50m · 5 blocks");
    expect(items[1]!.text).toBe("Renew before it expires — an expired name is lost for good");
    expect(items[1]!.when).toBe("~10 days · 1450 blocks");
    expect(items[3]!.purchaseId).toBe("pur-1");
    expect(items[3]!.when).toBe("Ready now");
    expect(items[4]!.when).toBe("No rush");
    expect(items[5]!.text).toBe("You lost — reclaim the 11 HNS lockup");
  });

  it("words a redeem on an owned name as reclaiming our other bids, and an unknown value plainly", () => {
    const items = needsYouItems(
      input({
        caps: [
          makeCapabilities({
            name: "mine",
            taskState: "lostNeedsRedeem",
            ownsName: true,
            redeemableValueDoos: 7_000_000,
          }),
          makeCapabilities({ name: "lost", taskState: "lostNeedsRedeem", redeemableValueDoos: 0 }),
        ],
      }),
    );
    expect(items.map((i) => i.text)).toEqual([
      "Reclaim the 7 HNS locked in your other bids",
      "You lost — reclaim your lockup",
    ]);
  });

  it("words a reveal with no known lockup without a figure", () => {
    const [item] = needsYouItems(
      input({
        caps: [
          makeCapabilities({ name: "x", taskState: "readyToReveal", lockupValueDoos: null }),
        ],
      }),
    );
    expect(item!.text).toBe("Reveal your bid, or its lockup can't be reclaimed");
    expect(item!.when).toBe("soon");
  });

  it("never fires a renewal from CSV-imported expiry", () => {
    expect(needsYouItems(input({ renewals: [renewal({ source: "csv-import" })] }))).toEqual([]);
  });

  it("dedupes by name + kind, the renewals row winning over caps expiringSoon", () => {
    const items = needsYouItems(
      input({
        renewals: [renewal()],
        caps: [
          makeCapabilities({
            name: "oldtimer",
            taskState: "expiringSoon",
            countdownBlocks: 9,
            countdownHours: null,
          }),
        ],
      }),
      NOW,
    );
    expect(items).toHaveLength(1);
    expect(items[0]!.blocks).toBe(1450);
  });

  it("leaves out a transfer still in its lockup", () => {
    expect(
      needsYouItems(
        input({ caps: [makeCapabilities({ name: "t", taskState: "transferPendingFinalize" })] }),
      ),
    ).toEqual([]);
  });

  it("leaves out a purchase that isn't ready, or whose finalize was already sent", () => {
    expect(needsYouItems(input({ names: [purchase({ blocksRemaining: 3 })] }))).toEqual([]);
    expect(
      needsYouItems(input({ names: [purchase()], finalizeSent: new Set(["pur-1"]) })),
    ).toEqual([]);
  });

  it("sorts within a kind by fewer blocks first, unknown last", () => {
    const reveal = (name: string, countdownBlocks: number | null) =>
      makeCapabilities({ name, taskState: "readyToReveal", countdownBlocks });
    const items = needsYouItems(
      input({ caps: [reveal("c", null), reveal("b", 20), reveal("a", 3)] }),
    );
    expect(items.map((i) => i.name)).toEqual(["a", "b", "c"]);
  });

  it("is empty for a watch-only wallet", () => {
    expect(
      needsYouItems(input({ isWatchOnly: true, renewals: [renewal()], names: [purchase()] })),
    ).toEqual([]);
  });
});

describe("needsYouWhen", () => {
  it("shows backend hours plus blocks on a wall-clock network", () => {
    expect(needsYouWhen(5, 0.83, "mainnet", NOW)).toBe("~50m · 5 blocks");
    expect(needsYouWhen(30, 5, "testnet", NOW)).toBe("~5h · 30 blocks");
    expect(needsYouWhen(432, 72, "mainnet", NOW)).toBe("~3 days · 432 blocks");
    expect(needsYouWhen(20000, 24 * 140, "mainnet", NOW)).toBe("Mar 2027 · 20000 blocks");
  });

  it("shows blocks alone on regtest, or when the backend gave no hours", () => {
    expect(needsYouWhen(5, 0.83, "regtest", NOW)).toBe("5 blocks");
    expect(needsYouWhen(1, null, "mainnet", NOW)).toBe("1 block");
  });
});
