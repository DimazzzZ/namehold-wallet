/**
 * The HNS ⇄ BTC exchange's bridge types, field for field the structs of
 * `docs/specs/2026-11-07-hns-btc-exchange.md` §9 (camelCase on both sides).
 *
 * Amounts of HNS cross as strings of whole HNS with up to 6 decimals, BTC as
 * integer satoshis, prices as strings of sats per HNS with up to 3 decimals,
 * times as Unix seconds. Every number here comes from the backend's u64s
 * (and, for offers, from the relay), so it can exceed what a JS number or a
 * `Date` holds: render them through the helpers in `lib/exchangeAmount.ts`
 * and `lib/exchangeStatus.ts`, which return null for a value they cannot show.
 */

/** A chain a trade's status talks about. */
export type ExchangeChain = "hns" | "btc";

/** The maker's side of an offer: `sell` = the maker sells HNS, `buy` = the maker buys HNS. */
export type OfferSide = "sell" | "buy";

/** This party's role in a trade. */
export type TradeRole = "buyer" | "seller";

export type ExchangeOverview = {
  /** "main" | "testnet" | "regtest" | "simnet" */
  network: string;
  /** false: `refusal` says why (R17). */
  available: boolean;
  refusal: string | null;
  /** The BTC account and swap keys exist. */
  enabled: boolean;
  relayUrl: string;
  watchtowerUrl: string | null;
  btcSources: string[];
  /** "bitcoin" | "testnet" | "regtest" */
  btcNetwork: string;
  /** A bitcoind RPC `user:password` is stored (its value never crosses the bridge). */
  bitcoindAuthSet: boolean;
  /** The Nostr relays as set; empty = `defaultNostrRelays`. */
  nostrRelays: string[];
  defaultNostrRelays: string[];
  /** Where offers and messages go: the custom relay, Nostr, or nowhere. */
  meetingPoint: "relay" | "nostr" | "none";
};

export type BtcBalance = {
  confirmedSats: number;
  unconfirmedSats: number;
  receiveAddress: string;
  /** BTC in this profile's BTC locks. */
  lockedInTradesSats: number;
  checkedAt: number;
  /** The source could not be read. */
  error: string | null;
};

export type OfferRow = {
  id: string;
  /** The maker's side. */
  side: OfferSide;
  /** Sats per HNS. */
  price: string;
  minHns: string;
  maxHns: string;
  minSats: number;
  maxSats: number;
  own: boolean;
  /** First 8 hex of the maker's relay key. */
  makerKey: string;
  expiresAt: number;
  /** Own offers: published and heartbeating. */
  live: boolean;
};

export type MarketOffers = { offers: OfferRow[]; fetchedAt: number; error: string | null };

export type TradeTimeline = {
  buyerFundDeadline: number;
  sellerFundDeadline: number;
  revealDeadline: number;
  hnsRefundTime: number;
  btcRefundTime: number;
};

export type TradeOutpoint = { txid: string; vout: number };

export type TradeRow = {
  tradeId: string;
  /** null while negotiating. */
  termsId: string | null;
  role: TradeRole;
  maker: boolean;
  offerId: string;
  hnsAmount: string;
  btcSats: number;
  price: string;
  /** engine::Status, tagged by `kind`. */
  status: TradeStatus;
  /** Terms not agreed yet. */
  negotiating: boolean;
  /** The counterparty's reason. */
  declined: string | null;
  timeline: TradeTimeline | null;
  hnsLock: TradeOutpoint | null;
  btcLock: TradeOutpoint | null;
  watchtower: "none" | "registered" | "failed";
  lastError: string | null;
  createdAt: number;
  updatedAt: number;
};

export type NeedsAttentionReason =
  | "sellerLockedWrongAmount"
  | "buyerLockedWrongAmount"
  | "btcRefundedAfterReveal"
  | "hnsRefundedAfterReveal"
  | "missingPresigned";

export type TradeStatus =
  | { kind: "awaitingBuyerCommit"; deadline: number }
  | { kind: "awaitingBtcLock"; confirmations: number; needed: number }
  | { kind: "awaitingSellerCommit"; deadline: number }
  | { kind: "awaitingHnsLock"; confirmations: number; needed: number }
  | { kind: "revealing" }
  | { kind: "awaitingReveal"; refundOpensAt: number }
  | { kind: "claiming"; rung: number }
  | { kind: "completed" }
  | { kind: "awaitingRefund"; chain: ExchangeChain; opensAt: number }
  | { kind: "refunding"; chain: ExchangeChain }
  | { kind: "refunded"; chain: ExchangeChain }
  | { kind: "expired" }
  | { kind: "cannotCheck"; chain: ExchangeChain }
  | { kind: "needsAttention"; reason: NeedsAttentionReason; chain: ExchangeChain };

export type TradeStatusKind = TradeStatus["kind"];

/** `exchange_btc_withdraw`'s answer. */
export type BtcWithdrawResult = { txid: string };
