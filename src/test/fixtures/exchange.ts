/**
 * Complete fixtures for the exchange's bridge types (`src/types/exchange.ts`):
 * every field set, overridable one by one.
 */
import type {
  BtcBalance,
  ExchangeOverview,
  MarketOffers,
  OfferRow,
  TradeRow,
  TradeStatus,
  TradeTimeline,
} from "../../types/exchange";

/** 2026-11-07 12:00:00 UTC. */
export const T0 = 1_794_052_800;
const H = 3600;

export function makeOverview(over: Partial<ExchangeOverview> = {}): ExchangeOverview {
  return {
    network: "regtest",
    available: true,
    refusal: null,
    enabled: true,
    relayUrl: "http://127.0.0.1:8787",
    watchtowerUrl: null,
    btcSources: ["https://mempool.example/api", "https://blockstream.example/api"],
    btcNetwork: "regtest",
    bitcoindAuthSet: false,
    ...over,
  };
}

export function makeBtcBalance(over: Partial<BtcBalance> = {}): BtcBalance {
  return {
    confirmedSats: 1_250_000,
    unconfirmedSats: 20_000,
    receiveAddress: "bcrt1qexchangereceive0000000000000000000000",
    lockedInTradesSats: 300_000,
    checkedAt: T0,
    error: null,
    ...over,
  };
}

export function makeOffer(over: Partial<OfferRow> = {}): OfferRow {
  return {
    id: "offer-1",
    side: "sell",
    price: "250",
    minHns: "100",
    maxHns: "2500",
    minSats: 25_000,
    maxSats: 625_000,
    own: false,
    makerKey: "a1b2c3d4",
    expiresAt: T0 + 180,
    live: true,
    ...over,
  };
}

export function makeMarketOffers(over: Partial<MarketOffers> = {}): MarketOffers {
  return {
    offers: [
      makeOffer({ id: "sell-1", side: "sell", makerKey: "aaaa1111" }),
      makeOffer({ id: "sell-own", side: "sell", own: true, makerKey: "0wn0wn00", price: "260" }),
      makeOffer({ id: "buy-1", side: "buy", makerKey: "bbbb2222", price: "240.5" }),
    ],
    fetchedAt: T0,
    error: null,
    ...over,
  };
}

export function makeTimeline(over: Partial<TradeTimeline> = {}): TradeTimeline {
  return {
    buyerFundDeadline: T0 + 1 * H,
    sellerFundDeadline: T0 + 4 * H,
    revealDeadline: T0 + 9 * H,
    hnsRefundTime: T0 + 12 * H,
    btcRefundTime: T0 + 24 * H,
    ...over,
  };
}

export function makeTrade(over: Partial<TradeRow> = {}): TradeRow {
  return {
    tradeId: "trade-1",
    termsId: "ab".repeat(32),
    role: "buyer",
    maker: false,
    offerId: "sell-1",
    hnsAmount: "1000",
    btcSats: 250_000,
    price: "250",
    status: { kind: "awaitingBuyerCommit", deadline: T0 + 1 * H },
    negotiating: false,
    declined: null,
    timeline: makeTimeline(),
    hnsLock: null,
    btcLock: null,
    watchtower: "none",
    lastError: null,
    createdAt: T0,
    updatedAt: T0,
    ...over,
  };
}

/** One value of every `TradeStatus` variant (every chain and reason too). */
export const EVERY_STATUS: TradeStatus[] = [
  { kind: "awaitingBuyerCommit", deadline: T0 + H },
  { kind: "awaitingBtcLock", confirmations: 1, needed: 2 },
  { kind: "awaitingSellerCommit", deadline: T0 + 4 * H },
  { kind: "awaitingHnsLock", confirmations: 3, needed: 6 },
  { kind: "revealing" },
  { kind: "awaitingReveal", refundOpensAt: T0 + 12 * H },
  { kind: "claiming", rung: 2 },
  { kind: "completed" },
  { kind: "awaitingRefund", chain: "hns", opensAt: T0 + 12 * H },
  { kind: "awaitingRefund", chain: "btc", opensAt: T0 + 24 * H },
  { kind: "refunding", chain: "hns" },
  { kind: "refunding", chain: "btc" },
  { kind: "refunded", chain: "hns" },
  { kind: "refunded", chain: "btc" },
  { kind: "expired" },
  { kind: "cannotCheck", chain: "hns" },
  { kind: "cannotCheck", chain: "btc" },
  { kind: "needsAttention", reason: "sellerLockedWrongAmount", chain: "hns" },
  { kind: "needsAttention", reason: "buyerLockedWrongAmount", chain: "btc" },
  { kind: "needsAttention", reason: "btcRefundedAfterReveal", chain: "btc" },
  { kind: "needsAttention", reason: "hnsRefundedAfterReveal", chain: "hns" },
  { kind: "needsAttention", reason: "missingPresigned", chain: "hns" },
  { kind: "needsAttention", reason: "missingPresigned", chain: "btc" },
];
