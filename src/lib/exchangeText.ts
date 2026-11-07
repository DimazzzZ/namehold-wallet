/**
 * The Exchange page's shared sentences, and the one predicate every button
 * on it asks whether it may act (R17).
 *
 * The backend decides whether the exchange can be used on the active profile
 * (`exchange_overview.available`) and says why not in `refusal`; that sentence
 * is shown verbatim, as the banner and as the hint of every disabled button,
 * so the page never words a refusal of its own that could drift from the
 * backend's.
 */
import type { ExchangeOverview, OfferRow, OfferSide, TradeRole } from "../types/exchange";

export const EXCHANGE_TITLE = "Exchange";
export const EXCHANGE_SUBTITLE =
  "Trade HNS for BTC with other people. Each trade is an atomic swap: both sides get paid, or both get their coins back.";

/** While `exchange_overview` has not answered (or failed): nothing may act yet. */
export const EXCHANGE_CHECKING =
  "Namehold has not finished checking whether the exchange can be used with this wallet.";
/** `available` is false but the backend sent no sentence: never leave a disabled button unexplained. */
export const EXCHANGE_UNAVAILABLE =
  "The exchange cannot be used with this wallet. Namehold did not say why.";
/** Before `exchange_enable`. */
export const EXCHANGE_NOT_ENABLED = "Enable the exchange first.";
/** The enable call to action. */
export const ENABLE_LEAD = "The exchange is off for this wallet.";
export const ENABLE_TEXT =
  "Enabling it stores the public keys of a Bitcoin account made from this wallet's recovery phrase (any BIP 84 wallet restores it from the same phrase) and of the keys your trades use. It needs the wallet unlocked and makes no transaction.";
export const ENABLED_TOAST = "Exchange enabled";

/**
 * Why nothing on the page may act, or null when it may: the backend's own
 * sentence when the exchange is unavailable here. The Enable button asks this.
 */
export function exchangeRefusal(overview: ExchangeOverview | null | undefined): string | null {
  if (overview == null) return EXCHANGE_CHECKING;
  if (!overview.available) return overview.refusal ?? EXCHANGE_UNAVAILABLE;
  return null;
}

/**
 * Why a trading action (take, create, withdraw, commit, cancel, refresh) may
 * not run, or null: the exchange's refusal, else not enabled yet.
 */
export function exchangeActionRefusal(
  overview: ExchangeOverview | null | undefined,
): string | null {
  const refusal = exchangeRefusal(overview);
  if (refusal !== null) return refusal;
  if (!overview?.enabled) return EXCHANGE_NOT_ENABLED;
  return null;
}

// --- The relay and the settings ---

/** No relay URL is stored: the order book cannot be read, and no offer can be taken, made or withdrawn. */
export const NO_RELAY = "No relay is set. Set one in Exchange settings to see and publish offers.";

/** The relay is configured. */
export function hasRelay(overview: ExchangeOverview | null | undefined): boolean {
  return (overview?.relayUrl ?? "").trim() !== "";
}

/** Why an offer action (take, make, withdraw) may not run, or null: the gate, else no relay. */
export function offerActionRefusal(overview: ExchangeOverview | null | undefined): string | null {
  return exchangeActionRefusal(overview) ?? (hasRelay(overview) ? null : NO_RELAY);
}

export const SOURCES_HINT =
  "Bitcoin sources are not trusted: Namehold checks every lock by proof of work and needs every source to agree.";
export const SETTINGS_SAVED_TOAST = "Exchange settings saved";
export const URL_INVALID = "Enter a URL that starts with http:// or https://";
export const SOURCE_INVALID = (line: string) =>
  `"${line}" is not a Bitcoin source: use an Esplora URL (https://…) or bitcoind+http://host:port`;
export const SOURCES_EMPTY = "Enter at least one Bitcoin source";
export const AUTH_INVALID = "Enter the RPC login as user:password";
export const AUTH_SET = "A user:password is set. Leave the field empty to keep it.";
export const AUTH_CLEARED = "The stored user:password is removed when you save.";

// --- The order book ---

/** Section titles: `sell` offers are people selling HNS (the user buys), `buy` the reverse. */
export const SIDE_TITLE: Record<OfferSide, string> = { sell: "Sell HNS", buy: "Buy HNS" };
export const SIDE_SUBTITLE: Record<OfferSide, string> = {
  sell: "People selling HNS for BTC. Take one to buy HNS.",
  buy: "People buying HNS with BTC. Take one to sell HNS.",
};
export const SIDE_EMPTY: Record<OfferSide, string> = {
  sell: "Nobody is selling HNS right now.",
  buy: "Nobody is buying HNS right now.",
};
/** The user's role when taking an offer of this side. */
export function takerRole(side: OfferSide): TradeRole {
  return side === "sell" ? "buyer" : "seller";
}
/** The user's role in an offer they make. */
export function makerRole(side: OfferSide): TradeRole {
  return side === "sell" ? "seller" : "buyer";
}

export const OWN_OFFER = "This is your own offer.";
export const OFFER_UNREADABLE =
  "This offer's price or limits cannot be read, so it cannot be taken.";

/** Why an offer's Take button is disabled, or null. */
export function takeRefusal(
  overview: ExchangeOverview | null | undefined,
  offer: OfferRow,
): string | null {
  return offerActionRefusal(overview) ?? (offer.own ? OWN_OFFER : null);
}

export const TAKE_BELOW_MIN = (min: string) => `This offer takes at least ${min} HNS`;
export const TAKE_ABOVE_MAX = (max: string) => `This offer takes at most ${max} HNS`;
export const TAKE_ZERO_SATS = "At this price that amount is worth less than 1 sat";
export const TAKEN_TOAST = "Trade started · it is under My trades";

// --- Creating an offer ---

export const MIN_ABOVE_MAX = "The smallest trade cannot be larger than the largest";
export const OFFER_CREATED_TOAST = "Offer published";
export const OFFER_WITHDRAWN_TOAST = "Offer withdrawn";
export const SECURE_WINDOW_WAIT = "Confirm in the secure window…";

// --- Trades ---

export const TERMS_NOT_AGREED = "The terms are not agreed yet.";
export const COMMITTED_TOAST = "Confirmed and signed";
export const CANCELLED_TOAST = "Trade cancelled";
export const NO_TRADES = "No trades yet. Take an offer in the order book, or make your own.";
export const NO_OFFERS = "You have no offers. Make one to let others trade with you.";

// --- The BTC account ---

export const BTC_NOT_ENABLED_TEXT =
  "Enable the exchange to get a Bitcoin account made from this wallet's recovery phrase.";
export const WITHDRAW_NO_ADDRESS = "Enter a Bitcoin address";
export const WITHDRAW_ABOVE_BALANCE = (balance: string) =>
  `That is more than your confirmed balance of ${balance}`;
export const BTC_SENT_TOAST = "BTC sent";
