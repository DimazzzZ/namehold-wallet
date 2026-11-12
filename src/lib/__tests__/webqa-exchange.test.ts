import { beforeEach, describe, expect, it } from "vitest";
import { mockInvoke, __resetChainForTests } from "../webqa-mock";
import type { ExchangeOverview, MarketOffers, TradeRow } from "../../types/exchange";

describe("browser QA mock: exchange", () => {
  beforeEach(() => __resetChainForTests());

  it("starts off, enables, and serves an order book of both sides", () => {
    expect(mockInvoke<ExchangeOverview>("exchange_overview").enabled).toBe(false);
    expect(mockInvoke<ExchangeOverview>("exchange_enable").enabled).toBe(true);
    const book = mockInvoke<MarketOffers>("exchange_list_offers");
    expect(new Set(book.offers.map((o) => o.side))).toEqual(new Set(["sell", "buy"]));
  });

  it("walks a taken trade to completion through Commit and Refresh", () => {
    mockInvoke("exchange_enable");
    const t = mockInvoke<TradeRow>("exchange_take_offer", {
      offerId: "demo-sell-1",
      hnsAmount: "1000",
    });
    expect(t.role).toBe("buyer");
    expect(t.btcSats).toBe(250_000);
    mockInvoke("exchange_commit", { tradeId: t.tradeId });
    let kind = "";
    for (let i = 0; i < 10 && kind !== "awaitingSellerCommit"; i++) {
      kind = mockInvoke<TradeRow[]>("exchange_refresh")[0]!.status.kind;
    }
    // The seller's Commit is the counterparty's: Refresh simulates it.
    for (let i = 0; i < 10 && kind !== "completed"; i++) {
      kind = mockInvoke<TradeRow[]>("exchange_refresh")[0]!.status.kind;
    }
    expect(kind).toBe("completed");
  });

  it("stores settings, keeping or clearing the bitcoind login", () => {
    const set = (bitcoindAuth: string | null) =>
      mockInvoke<ExchangeOverview>("exchange_update_settings", {
        relayUrl: "",
        watchtowerUrl: "",
        btcSources: ["https://mempool.space/api"],
        bitcoindAuth,
      });
    expect(set("u:p").bitcoindAuthSet).toBe(true);
    expect(set(null).bitcoindAuthSet).toBe(true);
    expect(set("").bitcoindAuthSet).toBe(false);
    expect(mockInvoke<MarketOffers>("exchange_list_offers").offers).toEqual([]);
  });
});
