/**
 * Browser QA mock of the exchange commands (spec §9), so the Exchange page
 * renders and can be clicked through without Tauri. State lives in module
 * memory for the page's life. There is no secure window here: a command that
 * would open one is taken as confirmed. A Commit moves the trade one step on;
 * Refresh moves every trade one step, so a whole trade can be walked through.
 */
import type {
  BtcBalance,
  ExchangeOverview,
  MarketOffers,
  OfferRow,
  OfferSide,
  TradeRow,
  TradeStatus,
} from "../types/exchange";

type Handler = (args?: Record<string, unknown>) => unknown;

const HOUR = 3600;
const now = () => Math.floor(Date.now() / 1000);

interface ExchangeState {
  overview: ExchangeOverview;
  myOffers: OfferRow[];
  trades: TradeRow[];
  next: number;
}

function initial(): ExchangeState {
  return {
    overview: {
      network: "regtest",
      available: true,
      refusal: null,
      enabled: false,
      relayUrl: "http://127.0.0.1:8787",
      watchtowerUrl: null,
      btcSources: ["https://mempool.space/testnet/api", "https://blockstream.info/testnet/api"],
      btcNetwork: "regtest",
      bitcoindAuthSet: false,
      nostrRelays: [],
      defaultNostrRelays: [],
      meetingPoint: "relay",
    },
    myOffers: [],
    trades: [],
    next: 1,
  };
}

const state: ExchangeState = initial();

/** Exact sats for an HNS text at a price text: floor(doos × msat / 1e9). */
function satsFor(hns: string, price: string): number {
  const toUnits = (t: string, places: number) => {
    const [w = "0", f = ""] = t.split(".");
    return (
      BigInt(w || "0") * 10n ** BigInt(places) +
      BigInt(f.padEnd(places, "0").slice(0, places) || "0")
    );
  };
  return Number((toUnits(hns, 6) * toUnits(price, 3)) / 1_000_000_000n);
}

function offer(
  id: string,
  side: OfferSide,
  price: string,
  minHns: string,
  maxHns: string,
  makerKey: string,
  own = false,
): OfferRow {
  return {
    id,
    side,
    price,
    minHns,
    maxHns,
    minSats: satsFor(minHns, price),
    maxSats: satsFor(maxHns, price),
    own,
    makerKey,
    expiresAt: now() + 180,
    live: true,
  };
}

const DEMO_OFFERS: OfferRow[] = [
  offer("demo-sell-1", "sell", "250", "100", "2500", "3f9a0c11"),
  offer("demo-sell-2", "sell", "262.5", "500", "10000", "c0ffee42"),
  offer("demo-buy-1", "buy", "238", "100", "4000", "b16b00b5"),
  offer("demo-buy-2", "buy", "230.125", "1000", "20000", "deadbe3f"),
];

function timeline(t0: number) {
  return {
    buyerFundDeadline: t0 + HOUR,
    sellerFundDeadline: t0 + 4 * HOUR,
    revealDeadline: t0 + 9 * HOUR,
    hnsRefundTime: t0 + 12 * HOUR,
    btcRefundTime: t0 + 24 * HOUR,
  };
}

/** The status a trade moves to on its next step, in the happy path. */
function step(t: TradeRow): TradeStatus {
  const tl = t.timeline ?? timeline(t.createdAt);
  switch (t.status.kind) {
    case "awaitingBuyerCommit":
      return { kind: "awaitingBtcLock", confirmations: 0, needed: 2 };
    case "awaitingBtcLock":
      return t.status.confirmations + 1 >= t.status.needed
        ? { kind: "awaitingSellerCommit", deadline: tl.sellerFundDeadline }
        : { ...t.status, confirmations: t.status.confirmations + 1 };
    case "awaitingSellerCommit":
      return { kind: "awaitingHnsLock", confirmations: 0, needed: 6 };
    case "awaitingHnsLock":
      return t.status.confirmations + 3 >= t.status.needed
        ? { kind: "revealing" }
        : { ...t.status, confirmations: t.status.confirmations + 3 };
    case "revealing":
      return { kind: "claiming", rung: 0 };
    case "claiming":
      return { kind: "completed" };
    default:
      return t.status;
  }
}

function advance(t: TradeRow): TradeRow {
  const status = step(t);
  const hex = (n: number) => n.toString(16).padStart(2, "0").repeat(32);
  return {
    ...t,
    negotiating: false,
    status,
    btcLock:
      status.kind === "awaitingBuyerCommit" ? null : (t.btcLock ?? { txid: hex(11), vout: 0 }),
    hnsLock:
      t.hnsLock ??
      (["awaitingHnsLock", "revealing", "claiming", "completed"].includes(status.kind)
        ? { txid: hex(22), vout: 0 }
        : null),
    updatedAt: now(),
  };
}

function tradeById(args?: Record<string, unknown>): TradeRow {
  const t = state.trades.find((x) => x.tradeId === args?.tradeId);
  if (!t) throw new Error("No such trade");
  return t;
}

function replace(t: TradeRow): TradeRow {
  state.trades = state.trades.map((x) => (x.tradeId === t.tradeId ? t : x));
  return t;
}

export const exchangeHandlers: Record<string, Handler> = {
  exchange_overview: (): ExchangeOverview => ({ ...state.overview }),
  exchange_enable: (): ExchangeOverview => {
    state.overview = { ...state.overview, enabled: true };
    return { ...state.overview };
  },
  exchange_update_settings: (args): ExchangeOverview => {
    const auth = args?.bitcoindAuth as string | null | undefined;
    state.overview = {
      ...state.overview,
      relayUrl: String(args?.relayUrl ?? ""),
      watchtowerUrl: String(args?.watchtowerUrl ?? "") || null,
      btcSources: (args?.btcSources as string[] | undefined) ?? [],
      bitcoindAuthSet: auth == null ? state.overview.bitcoindAuthSet : auth !== "",
      nostrRelays: (args?.nostrRelays as string[] | undefined) ?? [],
    };
    const nostr = state.overview.nostrRelays.length
      ? state.overview.nostrRelays
      : state.overview.defaultNostrRelays;
    state.overview.meetingPoint = state.overview.relayUrl
      ? "relay"
      : nostr.length
        ? "nostr"
        : "none";
    return { ...state.overview };
  },
  exchange_btc_balance: (): BtcBalance => ({
    confirmedSats: 1_250_000,
    unconfirmedSats: 0,
    receiveAddress: "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080",
    lockedInTradesSats: state.trades
      .filter((t) => t.role === "buyer" && t.btcLock && t.status.kind !== "completed")
      .reduce((n, t) => n + t.btcSats, 0),
    checkedAt: now(),
    error: null,
  }),
  exchange_btc_withdraw: () => ({ txid: "c4".repeat(32) }),
  exchange_list_offers: (): MarketOffers => ({
    offers:
      state.overview.meetingPoint !== "none"
        ? [...DEMO_OFFERS, ...state.myOffers.map((o) => ({ ...o, own: true }))]
        : [],
    fetchedAt: now(),
    error: null,
  }),
  exchange_my_offers: (): OfferRow[] => state.myOffers,
  exchange_create_offer: (args): OfferRow => {
    const o = offer(
      `mine-${state.next++}`,
      args?.side === "buy" ? "buy" : "sell",
      String(args?.price),
      String(args?.minHns),
      String(args?.maxHns),
      "0e0e0e0e",
      true,
    );
    state.myOffers = [o, ...state.myOffers];
    return o;
  },
  exchange_withdraw_offer: (args) => {
    state.myOffers = state.myOffers.filter((o) => o.id !== args?.offerId);
    return null;
  },
  exchange_take_offer: (args): TradeRow => {
    const o = DEMO_OFFERS.find((x) => x.id === args?.offerId);
    if (!o) throw new Error("That offer is no longer on the relay");
    const t0 = now();
    const hnsAmount = String(args?.hnsAmount);
    const t: TradeRow = {
      tradeId: `trade-${state.next++}`,
      termsId: "7e".repeat(32),
      role: o.side === "sell" ? "buyer" : "seller",
      maker: false,
      offerId: o.id,
      hnsAmount,
      btcSats: satsFor(hnsAmount, o.price),
      price: o.price,
      status: { kind: "awaitingBuyerCommit", deadline: t0 + HOUR },
      negotiating: false,
      declined: null,
      timeline: timeline(t0),
      hnsLock: null,
      btcLock: null,
      watchtower: state.overview.watchtowerUrl ? "registered" : "none",
      lastError: null,
      createdAt: t0,
      updatedAt: t0,
    };
    state.trades = [t, ...state.trades];
    return t;
  },
  exchange_list_trades: (): TradeRow[] => state.trades,
  exchange_commit: (args): TradeRow => replace(advance(tradeById(args))),
  exchange_cancel_trade: (args): TradeRow =>
    replace({ ...tradeById(args), status: { kind: "expired" }, updatedAt: now() }),
  exchange_refresh: (): TradeRow[] => {
    // The user's own Commit is theirs to make: Refresh never makes it for them.
    // The counterparty's Commit is simulated as made.
    const mine = (t: TradeRow) =>
      (t.status.kind === "awaitingBuyerCommit" && t.role === "buyer") ||
      (t.status.kind === "awaitingSellerCommit" && t.role === "seller");
    state.trades = state.trades.map((t) => (mine(t) ? t : advance(t)));
    return state.trades;
  },
};

/** Test-only: back to a fresh exchange. */
export function __resetExchangeForTests(): void {
  Object.assign(state, initial());
}
