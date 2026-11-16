/**
 * The showcase wallet's side of the HNS ⇄ BTC exchange: enabled on mainnet,
 * offers and messages over the default Nostr relays, a few offers in the
 * order book, one of our own, and three trades in different states.
 *
 * Typed against `src/types/exchange.ts`. Amounts respect mainnet's policy
 * (`noncustodial/exchange/policy.rs`: 50 000 to 10 000 000 sats a trade) and
 * the times follow its windows (1 h for the Buyer's Commit, 4 h more for the
 * Seller's, the HNS refund at 12 h, the reveal 3 h before it, the BTC refund
 * 12 h after it). The Bitcoin address is derived by `derive-keys.py` from a
 * fixed string, so it belongs to nobody.
 */
import type {
  BtcBalance,
  ExchangeOverview,
  MarketOffers,
  OfferRow,
  OfferSide,
  TradeRow,
  TradeTimeline,
} from "../../src/types/exchange";

const HOUR = 3600;
const MINUTE = 60;

/** `env::DEFAULT_NOSTR_RELAYS`. */
const DEFAULT_NOSTR_RELAYS = [
  "wss://relay.primal.net",
  "wss://offchain.pub",
  "wss://relay.snort.social",
  "wss://relay.nostr.net",
  "wss://nostr.oxtr.dev",
  "wss://relay.mostro.network",
];

const BTC_RECEIVE = "bc1qxzez3mhy90fqdzcl5mt5r6gf3dgnre83g0atj5";

const secs = (now: Date) => Math.floor(now.getTime() / 1000);

/** Exact sats for whole HNS at a price of sats per HNS with up to 3 decimals. */
function sats(hns: number, price: string): number {
  const [w = "0", f = ""] = price.split(".");
  const milli = BigInt(w) * 1000n + BigInt(f.padEnd(3, "0").slice(0, 3) || "0");
  return Number((BigInt(hns) * milli) / 1000n);
}

function offer(
  now: Date,
  id: string,
  side: OfferSide,
  price: string,
  minHns: number,
  maxHns: number,
  makerKey: string,
  own = false,
): OfferRow {
  return {
    id: id.repeat(64).slice(0, 64),
    side,
    price,
    minHns: String(minHns),
    maxHns: String(maxHns),
    minSats: sats(minHns, price),
    maxSats: sats(maxHns, price),
    own,
    makerKey,
    // A Nostr offer event lives 5 minutes and is renewed every minute.
    expiresAt: secs(now) + 4 * MINUTE,
    live: true,
  };
}

const ownOffer = (now: Date) => offer(now, "a4", "sell", "10.4", 5_000, 50_000, "7c21e9d0", true);

export function exchangeOverview(): ExchangeOverview {
  return {
    network: "main",
    available: true,
    refusal: null,
    enabled: true,
    relayUrl: "",
    watchtowerUrl: null,
    btcSources: ["https://mempool.space/api", "https://blockstream.info/api"],
    btcNetwork: "bitcoin",
    bitcoindAuthSet: false,
    nostrRelays: [],
    defaultNostrRelays: DEFAULT_NOSTR_RELAYS,
    meetingPoint: "nostr",
  };
}

export function marketOffers(now: Date): MarketOffers {
  return {
    offers: [
      offer(now, "e1", "sell", "10.2", 5_000, 40_000, "3f9a0c11"),
      ownOffer(now),
      offer(now, "e2", "sell", "10.75", 10_000, 120_000, "c0ffee42"),
      offer(now, "b1", "buy", "9.8", 5_500, 30_000, "b16b00b5"),
      offer(now, "b2", "buy", "9.45", 6_000, 80_000, "d2e8a4f7"),
    ],
    fetchedAt: secs(now) - 20,
    error: null,
  };
}

export function myOffers(now: Date): OfferRow[] {
  return [ownOffer(now)];
}

function timeline(t0: number): TradeTimeline {
  return {
    buyerFundDeadline: t0 + HOUR,
    sellerFundDeadline: t0 + 5 * HOUR,
    revealDeadline: t0 + 9 * HOUR,
    hnsRefundTime: t0 + 12 * HOUR,
    btcRefundTime: t0 + 24 * HOUR,
  };
}

/** A txid that looks like one: the sha256 of a fixed string, so it is nobody's transaction. */
const TXIDS: Record<string, string> = {
  "9b": "14112cca250804ffec7360c8126415d8d6ebd8ca6cee7f299b1b1fb058139c87",
  "4c": "c34d5897e1bbe56410ddf46945b4a664d2da07a5ea2809c0fee26f1792921759",
  a2: "a19cf7fc486d09e60b10321bca92776ebcb1adcbd90f223471d2b8e9147a50da",
  "6f": "566d4537631976bdda05e5a40033527e9d6e3b5d55562b63493d90de368a0a08",
};
const outpoint = (tag: string, vout = 0) => ({ txid: TXIDS[tag]!, vout });

/** Newest first, as the backend lists them. */
export function trades(now: Date): TradeRow[] {
  const t = secs(now);
  // Someone took our sell offer and locked their BTC: our confirmation is due.
  const sold = t - 25 * MINUTE;
  // We took a sell offer and locked our BTC; the seller has not confirmed yet.
  const bought = t - 40 * MINUTE;
  // Yesterday's trade, settled on both chains.
  const done = t - 26 * HOUR;
  return [
    {
      tradeId: "5e11".repeat(8),
      termsId: "71e5".repeat(16),
      role: "seller",
      maker: true,
      offerId: ownOffer(now).id,
      hnsAmount: "20000",
      btcSats: sats(20_000, "10.4"),
      price: "10.4",
      status: { kind: "awaitingSellerCommit", deadline: timeline(sold).sellerFundDeadline },
      negotiating: false,
      declined: null,
      timeline: timeline(sold),
      hnsLock: null,
      btcLock: outpoint("9b"),
      watchtower: "none",
      lastError: null,
      createdAt: sold,
      updatedAt: t - 2 * MINUTE,
    },
    {
      tradeId: "b0b5".repeat(8),
      termsId: "7e2a".repeat(16),
      role: "buyer",
      maker: false,
      offerId: "e1".repeat(32),
      hnsAmount: "12000",
      btcSats: sats(12_000, "10.2"),
      price: "10.2",
      status: { kind: "awaitingSellerCommit", deadline: timeline(bought).sellerFundDeadline },
      negotiating: false,
      declined: null,
      timeline: timeline(bought),
      hnsLock: null,
      btcLock: outpoint("4c"),
      watchtower: "none",
      lastError: null,
      createdAt: bought,
      updatedAt: t - 6 * MINUTE,
    },
    {
      tradeId: "d0e1".repeat(8),
      termsId: "3c7d".repeat(16),
      role: "buyer",
      maker: false,
      offerId: "e2".repeat(32),
      hnsAmount: "8000",
      btcSats: sats(8_000, "10.1"),
      price: "10.1",
      status: { kind: "completed" },
      negotiating: false,
      declined: null,
      timeline: timeline(done),
      hnsLock: outpoint("a2", 0),
      btcLock: outpoint("6f"),
      watchtower: "none",
      lastError: null,
      createdAt: done,
      updatedAt: done + 50 * MINUTE,
    },
  ];
}

export function btcBalance(now: Date): BtcBalance {
  return {
    confirmedSats: 2_163_480,
    unconfirmedSats: 0,
    receiveAddress: BTC_RECEIVE,
    // Our BTC in the lock of the trade we are buying in.
    lockedInTradesSats: sats(12_000, "10.2"),
    checkedAt: secs(now) - 40,
    error: null,
  };
}
