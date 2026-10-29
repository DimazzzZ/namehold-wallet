import { SECS_PER_BLOCK } from "../../lib/auction";
import { settingToBool } from "../../lib/settingsBool";
import { MIN_FEE_RATE_DOOS_PER_KVB, parseDoosPerKvb } from "../../lib/feeRate";
import { formatDate, formatHns, formatHnsShort } from "../../lib/utils";
import type {
  MarketRow,
  ShakedexHidden,
  ShakedexListingActions,
  ShakedexListingPrices,
  ShakedexListingState,
  ShakedexListingSummary,
  ShakedexPendingSend,
  ShakedexScheduleInput,
  WalletNetwork,
  WalletProfileKind,
  WriteCapability,
} from "../../types";

/** Why Buy and Finalize are disabled for any profile but a recovery-phrase one (R16). */
export const RECOVERY_PHRASE_ONLY = "Shakedex works with a recovery-phrase wallet for now";

/** Why Buy and Finalize are disabled when the node cannot send (R6). */
export const NEEDS_SENDING_NODE =
  "Shakedex needs a local node, or a remote node with sending allowed";

/**
 * Whether a profile can buy or finalize through Shakedex: a seed-backed
 * recovery-phrase wallet on a node that can send, as the backend enforces
 * (`software_writer_ctx`). Unknown (still loading) is not.
 */
export function canUseShakedex(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
): boolean {
  return kind === "mnemonic_hot" && writeCap?.broadcasterAvailable === true;
}

/**
 * The backend's own sentence for why Shakedex is refused here, once the
 * profile or the write capability is known to refuse it; `null` otherwise.
 */
export function shakedexRefusal(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
): string | null {
  if (kind !== undefined && kind !== "mnemonic_hot") return RECOVERY_PHRASE_ONLY;
  if (writeCap != null && !writeCap.broadcasterAvailable) return NEEDS_SENDING_NODE;
  return null;
}

/** Why a market link cannot be imported off mainnet (R5). */
export const MARKET_MAINNET_ONLY = "LearnHNS Market lists mainnet names only";

/**
 * The backend's refusal of a mainnet purchase until Settings allows it
 * (R15), shown through `mapError` if it ever comes; the UI itself offers
 * "Turn on buying" first. Kept word for word with the backend's
 * (`refusal_sentences_match_the_uis_word_for_word`). Finalize is not gated.
 */
export const MAINNET_EXPERIMENTAL =
  "Shakedex purchases on mainnet are experimental: enable them in Settings";

/** R15's mainnet gate: buying is off until Settings turns it on. */
export function buyingGated(
  network: WalletNetwork | undefined,
  experimental: string | undefined,
): boolean {
  return network === "mainnet" && !settingToBool(experimental);
}

/** "~6 h" — the wait is measured against the node's median time, so it is only approximate. */
export function approxWait(secs: number): string {
  if (secs < 3600) return `~${Math.max(1, Math.round(secs / 60))} min`;
  if (secs < 48 * 3600) return `~${Math.round(secs / 3600)} h`;
  return `~${Math.round(secs / 86400)} d`;
}

/** A row the profile's chain source (SPV, Explorer) cannot check (R8). */
export const NOT_VERIFIED = "Not verified — needs a full or remote node";

/** Why a listing is not buyable, as the hidden rows and an import's error say it. */
export function hiddenReasonText(r: ShakedexHidden): string {
  switch (r.kind) {
    case "soldOrCancelled":
      return "Already sold or cancelled";
    case "failedVerification":
      return `Failed the check: ${r.reason}`;
    case "expiresBeforeFinalize":
      return "The name expires before a purchase could be finalized";
    case "notYetValid":
      return `Not valid yet: the first price becomes valid in ${approxWait(r.firstValidInSecs)}`;
    case "couldNotCheck":
      return `Couldn't check it: ${nodeReason(r.reason)}`;
    case "unverified":
      return NOT_VERIFIED;
  }
}

/**
 * "Listed until July 10, 2027 · this date does not end the listing": the
 * seller's own `expiresAt` (unix seconds). Shown, never enforced: the signed
 * price steps stay valid on chain until the lock coin is spent, whatever the
 * date says (R2), and the spec has the wallet say so wherever the date is
 * shown (§4). `null` when the value is beyond what a date can hold — it comes
 * from a foreign file as any u64.
 */
export function listedUntilText(expiresAt: number): string | null {
  const d = new Date(expiresAt * 1000);
  if (Number.isNaN(d.getTime())) return null;
  return `Listed until ${formatDate(d.toISOString().slice(0, 10))} · this date does not end the listing`;
}

/**
 * A node failure in the user's words. The backend's text names the layer
 * ("Node RPC error: …"); the user needs what happened and where to fix it.
 */
export function nodeReason(reason: string): string {
  const bare = reason.replace(/^Node RPC error:\s*/i, "").replace(/^HTTP error:\s*/i, "");
  if (/error sending request|connection refused|tcp connect|dns error/i.test(bare)) {
    return "your node did not answer. Is it running? Check Settings → Connections.";
  }
  if (/timed out|timeout/i.test(bare)) {
    return "your node took too long to answer. Try again in a moment.";
  }
  return bare;
}

/** How a listing's status reads on the Market: a tone and one sentence. */
export type StatusTone = "ok" | "warn" | "muted" | "bad";

/** The colour of a status's dot, on the Market and in My listings. */
export const TONE_DOT: Record<StatusTone, string> = {
  ok: "bg-green-500",
  warn: "bg-amber-500",
  muted: "bg-gray-300",
  bad: "bg-red-500",
};

/** Names-page dots: the Market tones plus a blue "in progress". */
export const NAME_TONE_DOT: Record<StatusTone | "info", string> = {
  ...TONE_DOT,
  info: "bg-blue-500",
};

/** `commands/shakedex.rs::OWN_LISTING` (R33): a row this device locked, above its disabled Buy. */
export const OWN_LISTING =
  "this is your own listing: cancel it instead (My listings), buying it would only cost you network fees";

// --- Buying: the Market page ---

/** A backend fragment as a sentence of its own: capitalized, with a full stop. */
export function sentence(text: string): string {
  const t = text.trim();
  if (t === "") return t;
  const capital = t.charAt(0).toUpperCase() + t.slice(1);
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`;
}

/** The gate banner's lead and text (R15). */
export const BUYING_OFF_LEAD = "Buying on mainnet is off.";
export const BUYING_OFF_TEXT =
  "Shakedex swaps on mainnet are still experimental. You can browse and check listings; turn buying on when you're ready. Finalizing a name you already bought always works.";

/** The reason above the detail panel's Turn on buying. */
export const BUYING_OFF_REASON =
  "Buying on mainnet is off, so this button turns it on first. Nothing is bought until you press Buy.";

export const BUYING_ON_TOAST = "Mainnet buying turned on · also in Settings › Shakedex";

/** The detail panel's text for a listing the node could not answer for. */
export function uncheckedText(reason: string): string {
  return `${sentence(nodeReason(reason))} The listing may be fine; Namehold just won't show a price or let you buy until it's verified.`;
}

/** "1,100 HNS": at most two decimals, for prices read at a glance. */
export const hns = (doos: number) => `${formatHnsShort(doos)} HNS`;

/** "1,000.0042 HNS": every doo, trailing zeros trimmed, for what is paid or missing. */
export const hnsExact = (doos: number) => `${formatHns(doos).replace(/\.?0+$/, "")} HNS`;

/** The Next price column: "↓ 1,100 HNS in ~6 h", "Fixed price", or a reverse auction at its floor. */
export function nextPriceText(row: MarketRow): string {
  if (row.kind !== "reverseAuction") return "Fixed price";
  if (row.nextPrice == null || row.nextValidInSecs == null) return "At its floor";
  return `↓ ${hns(row.nextPrice)} in ${approxWait(row.nextValidInSecs)}`;
}

/**
 * A reverse auction's way down: "Drops to 1,100 HNS in ~6 h, down to a floor
 * of 400 HNS in ~3 d." The floor's wait is its step's, as the backend gives it.
 */
export function priceDropText(row: MarketRow): string {
  if (row.nextPrice == null || row.nextValidInSecs == null) {
    return `At its floor of ${hns(row.floorPrice)}; the price drops no further.`;
  }
  const next = `Drops to ${hns(row.nextPrice)} in ${approxWait(row.nextValidInSecs)}`;
  if (row.nextPrice === row.floorPrice) return `${next}, its floor.`;
  const floor = row.steps.find((st) => st.price === row.floorPrice);
  const when = floor?.validInSecs != null ? ` in ${approxWait(floor.validInSecs)}` : "";
  return `${next}, down to a floor of ${hns(row.floorPrice)}${when}.`;
}

/**
 * A fixed price's caption. The seller's `expiresAt` is shown, never
 * enforced: the signed price stays valid on chain until the lock coin is
 * spent (R2), so the caption says so beside the date. A date no `Date` can
 * hold (a u64 from a foreign file) is left out.
 */
export function fixedPriceCaption(expiresAt: number | null): string {
  const until = "stays at this price until it sells or the seller cancels.";
  const d = expiresAt == null ? null : new Date(expiresAt * 1000);
  if (d == null || Number.isNaN(d.getTime())) return sentence(until);
  return `Listed until ${formatDate(d.toISOString().slice(0, 10))}, but it ${until}`;
}

/**
 * "July 2027": the month the name's registration runs out, from its expiry
 * height and the node's tip, ten minutes a block. Null for a height no date
 * can hold.
 */
export function registeredUntilMonth(
  expiryEnd: number,
  tip: number,
  nowMs: number = Date.now(),
): string | null {
  const d = new Date(nowMs + (expiryEnd - tip) * SECS_PER_BLOCK * 1000);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString("en-US", { month: "long", year: "numeric", timeZone: "UTC" });
}

/** How it works' wait before the preview says the network's lockup. */
export const WAIT_UNKNOWN = "The swap settles on-chain. Track it in Your purchases.";

/**
 * How it works' wait step for a network's lockup: "Wait ~2 days" where blocks
 * keep time (`wallClockTiming`, R10), "Wait 10 blocks" where they are mined
 * on demand (regtest, simnet).
 */
export function waitStep(blocks: number, wallClockTiming: boolean): [string, string] {
  const track = "Track it in Your purchases.";
  if (!wallClockTiming) {
    return [
      `Wait ${blocks} blocks`,
      `The swap settles once ${blocks} more blocks are mined. ${track}`,
    ];
  }
  const mins = (blocks * SECS_PER_BLOCK) / 60;
  const unit = (n: number, one: string) => `~${n} ${one}${n === 1 ? "" : "s"}`;
  const time =
    mins < 60
      ? unit(Math.max(1, Math.round(mins)), "minute")
      : mins < 48 * 60
        ? unit(Math.round(mins / 60), "hour")
        : unit(Math.round(mins / 1440), "day");
  return [`Wait ${time}`, `${blocks} blocks while the swap settles. ${track}`];
}

/**
 * "~1 d 0 h left": the rest of a purchase's lockup, ten minutes a block,
 * where blocks keep time (`wallClockTiming`, R10). Regtest and simnet mine on
 * demand, so there it is "4 blocks left".
 */
export function lockupLeftText(blocks: number, wallClockTiming: boolean): string {
  if (!wallClockTiming) return `${blocks} ${blocks === 1 ? "block" : "blocks"} left`;
  const secs = blocks * SECS_PER_BLOCK;
  if (secs < 3600) return `~${Math.max(1, Math.round(secs / 60))} min left`;
  const hours = Math.round(secs / 3600);
  if (hours < 24) return `~${hours} h left`;
  return `~${Math.floor(hours / 24)} d ${hours % 24} h left`;
}

/**
 * R35: a Listed or Restored row's warning that buyers stop seeing it, as
 * the name gets too close to expiring for a purchase to be finalized (R9):
 * shown once `blocks` (`blocksUntilUnbuyable`) falls within `warnBlocks`
 * (R9's 30 days, at most half the renewal window), "in ~12 days" where
 * blocks keep time (`wallClockTiming`, R10) and "in 42 blocks" where they
 * are mined on demand. A name in a lock cannot be renewed, so the way out is Cancel.
 * `null`: nothing to say, or not known.
 */
export function unbuyableText(
  blocks: number | null,
  warnBlocks: number,
  wallClockTiming: boolean,
): string | null {
  if (blocks === null || blocks > warnBlocks) return null;
  const why = "the name is too close to expiring to finalize a purchase";
  const home = "Cancel it and bring the name home to renew it";
  if (blocks <= 0) return `Buyers no longer see this listing: ${why}. ${home}.`;
  let when: string;
  if (!wallClockTiming) {
    when = `in ${blocks} ${blocks === 1 ? "block" : "blocks"}`;
  } else {
    // Whole days down, as `lockupLeftText` counts them: never a day more than is left.
    const days = Math.floor((blocks * SECS_PER_BLOCK) / 86_400);
    when = days < 1 ? "in less than a day" : `in ~${days} ${days === 1 ? "day" : "days"}`;
  }
  return `Buyers stop seeing this listing ${when}, when the name gets too close to expiring to finalize a purchase. ${home}, then list it again.`;
}

/**
 * Why a typed fee rate cannot be used, or null (empty is the default). It
 * takes what `parseDoosPerKvb` (and so `FeeRateOverride`) takes, except a
 * rate below the minimum: the parser would raise it, and the panel refuses
 * it, rather than pay a rate nobody typed. The rate sent is still whole
 * sat/byte, rounded down (`parseFeeRateArg`): 1,500 doos/kvB pays 1 sat/byte.
 */
export function feeRateError(raw: string): string | null {
  const t = raw.trim();
  if (t === "") return null;
  if (!Number.isInteger(Number(t))) return "Whole number only";
  if (parseDoosPerKvb(t) === null || Number(t) < MIN_FEE_RATE_DOOS_PER_KVB) {
    return `Minimum is ${MIN_FEE_RATE_DOOS_PER_KVB.toLocaleString("en-US")}`;
  }
  return null;
}

/** The start of every link `market/learnhns.rs::name_from_listing_link` takes. */
const LISTING_LINK_PREFIX = "https://market.learnhns.com/listing/";

export const IMPORT_BAD_LINK =
  "That isn't a LearnHNS listing link. Expected market.learnhns.com/listing/<name>.";
export const IMPORT_BAD_FILE =
  "That isn't a Shakedex listing file. Expected JSON with version, name, lockingTxHash…";

/** A pasted `http(s)://` text is a market link, not a listing file. */
export function isListingLink(text: string): boolean {
  return /^https?:\/\//i.test(text.trim());
}

/**
 * What is plainly wrong with a pasted import before the node is asked: a
 * link off LearnHNS's listing pages, or text that is not JSON. The backend
 * judges the rest (the name in the link, the listing file's fields).
 */
export function importProblem(text: string): string | null {
  const t = text.trim();
  if (isListingLink(t)) return t.startsWith(LISTING_LINK_PREFIX) ? null : IMPORT_BAD_LINK;
  try {
    JSON.parse(t);
    return null;
  } catch {
    return IMPORT_BAD_FILE;
  }
}

// --- Selling (T7) ---

/** `noncustodial/shakedex/mod.rs::MAINNET_SELLING_EXPERIMENTAL` (R15, R29). */
export const MAINNET_SELLING_EXPERIMENTAL =
  "Selling names through Shakedex on mainnet is experimental: enable it in Settings";

/** `sell.rs::LOCK_COSTS` (R27). */
export const LOCK_COSTS =
  "Once the name is finalized into the lock, its DNS records cannot be changed and it cannot be renewed; the listing lasts at most one renewal window from that finalize, which renews the name.";

/** `commands/shakedex.rs::REVERSE_NEEDS_SCHEDULE` (R24, carry 8). */
export const REVERSE_NEEDS_SCHEDULE =
  "a reverse auction needs its schedule: a start price, a floor, the number of steps and the time between them";

/** `sell.rs::STEP_SIGNATURE_PERMANENCE` (R20). */
export const STEP_SIGNATURE_PERMANENCE =
  "Each signature lets anyone buy the name at its price until the listing is cancelled and the cancel is mined. A price can be lowered later, never raised.";

/** `sell.rs::LOWER_PRICE_PERMANENCE` (R26). */
export const LOWER_PRICE_PERMANENCE =
  "The earlier, dearer signatures stay valid until the lock coin is spent, by a purchase or by a cancel once it is mined, so anyone holding the listing file can still use them. A price can be lowered later, never raised.";

/** `commands/shakedex.rs::LOWER_DURING_CANCEL` (R26). */
export const LOWER_DURING_CANCEL =
  "a cancel of this listing is not mined yet: its price cannot be lowered";

/** `commands/shakedex.rs::LOWER_MANAGED_ELSEWHERE` (R40): Lower price waits for a take-over. */
export const LOWER_MANAGED_ELSEWHERE =
  "another device or profile with your recovery phrase manages this listing: lower its price there, or manage it from this device first";

/** `commands/shakedex.rs::TAKE_OVER_OTHER_ADDRESS` (R44): the other device's price pays another address. */
export const TAKE_OVER_OTHER_ADDRESS =
  "the price set on the other device pays an address this listing does not know: import that device's listing file, or cancel the listing and list it again";

/** R42: a listing managed elsewhere, in place of its market status. */
export const MANAGED_ELSEWHERE =
  "Managed from another device: LearnHNS Market shows a price this device never signed, so this device no longer uploads the listing. It still follows the chain.";

/** R42: the other device's price, under {@link MANAGED_ELSEWHERE}. */
export function managedElsewherePriceText(price: number): string {
  return `Price set on the other device: ${formatHns(price)} HNS. Anyone can buy at it until the listing is cancelled.`;
}

/** R42: this device's own prices of a listing managed elsewhere are labelled so. */
export const ON_THIS_DEVICE = "on this device";

/** R40: the export of a listing managed elsewhere holds only this device's prices. */
export const EXPORT_THIS_DEVICE_ONLY =
  "The listing file holds only this device's prices, not the one set on the other device.";

/** R40: the cancel of a listing managed elsewhere: the other device's price stays buyable until it is mined. */
export function cancelElsewhereText(price: number): string {
  return `The price set on the other device (${formatHns(price)} HNS) can be bought too until this cancel is mined.`;
}

/** R44: the take-over's button. */
export const MANAGE_HERE_LABEL = "Manage from this device";

/** R44: the take-over's confirmation, with the other device's price. */
export function MANAGE_HERE_TEXT(price: number): string {
  return `This device will upload the listing to LearnHNS Market again, at the price set on the other device (${formatHns(price)} HNS), and keep it there. If the other device keeps publishing it too, the two now upload the same price and stop overwriting each other. To manage it only from the other device instead, leave this as it is.`;
}

/** R43: a sale at a price none of this device's steps names. */
export const SOLD_ELSEWHERE = "Sold at a price set on another device";

/** R43: a mined cancel this device has no draft of. */
export const CANCELLED_ELSEWHERE =
  "Cancelled from another device or profile with your recovery phrase";

/** `commands/shakedex.rs::CANCEL_LOCKUP_NOT_COUNTED` (R28): Finalize home before any sync has counted the cancel's lockup. */
export const CANCEL_LOCKUP_NOT_COUNTED =
  "the cancel is mined, but its transfer lockup is not counted yet: sync, then finalize it";

/** `cancel.rs::CANCEL_STILL_BUYABLE` (R28). */
export const CANCEL_STILL_BUYABLE =
  "Anyone holding the listing file can still buy the name at its current price until this cancel is mined.";

/** `cancel.rs::CANCEL_STILL_BUYABLE_AS_IT_FALLS` (R28). */
export const CANCEL_STILL_BUYABLE_AS_IT_FALLS =
  "Anyone holding the listing file can still buy the name at any price step that is valid by then, a cheaper price step too once it is valid, until this cancel is mined.";

/** `cancel.rs::CANCEL_MEMPOOL_PURCHASE` (R28). */
export const CANCEL_MEMPOOL_PURCHASE =
  "If a purchase of the name is already in your node's mempool, the node does not take this cancel and it is never mined, although the node still answers it with a transaction id.";

/** `cancel.rs::CANCEL_THEN_FINALIZE` (R28). */
export const CANCEL_THEN_FINALIZE =
  "Once the cancel is mined and the transfer lockup is over, finalize it to bring the name home.";

/** `cancel.rs::CANCEL_MARKET_TOLD` (R28). */
export const CANCEL_MARKET_TOLD =
  "Told the listing is cancelled at the next sync after this cancel is mined (and later again while the market does not answer), if it was told about the listing; until then it can still show it. LearnHNS Market offers no way to withdraw a pending listing.";

/** R23's help for the publish box. */
export const PUBLISH_PRIVACY =
  "Publishing makes the name, its price and its lock public on LearnHNS Market, and the market sees your IP address every time the wallet checks or uploads the listing.";

/** R23, R29: off mainnet a listing is shared as its file only (no publish box, no market status). */
export const SHARED_AS_FILE_ONLY = `Shared as a file only: ${MARKET_MAINNET_ONLY}`;

/** R32: a lock restored by name, without its listing file. */
export const RESTORED_LABEL = "Locked — listing details unknown";

/** R34: the search's button and the lines around it. */
export const FIND_LOCKS_LABEL = "Find my locks";
export const FIND_LOCKS_BUSY = "Searching…";
export const NO_LOCKS_FOUND = "No locked names found";
/** R34: a forced search that adopted nothing but could not read or judge everything. */
export const LOCK_SEARCH_PARTIAL =
  "Some addresses or names could not be read from your node. Try again later.";
/** R34: the toast after a search restored `n` names. */
export const foundLocksText = (n: number) =>
  `Found ${n} ${n === 1 ? "name" : "names"} locked for sale · restored in My listings`;
/** R34: candidates wait for the seed while the signer is locked. */
export const LOCKS_WAIT_FOR_UNLOCK =
  "Names this recovery phrase may have locked for sale were found. Unlock the wallet to restore them.";
/** R34: the backend's `LOCK_SEARCH_NEEDS_INDEX`, sentence-cased with a full stop. */
export const LOCK_SEARCH_NEEDS_INDEX =
  "Finding your locks needs a node run with --index-tx and --index-address; Restore a lock by name works without them.";

/** Carry 6: a schedule longer than a listing file's year (`sell.rs::LISTING_LIFETIME_SECS`). */
export const SCHEDULE_PAST_A_YEAR =
  "The last price comes more than a year after the first, past the listing file's expiry date.";

/** `sell.rs::LISTING_LIFETIME_SECS` (365 days), pinned by `refusal_sentences_match_the_uis_word_for_word`. */
const LISTING_LIFETIME_SECS = 31536000;

/** `noncustodial/send.rs::DUST_THRESHOLD` and `purchase.rs::MAX_MONEY`, in doos. */
const DUST_DOOS = 1000n;
const MAX_MONEY_DOOS = 2_040_000_000_000_000n;

/** `sell.rs::{REVERSE_MIN_STEPS, REVERSE_MAX_STEPS, REVERSE_MIN_INTERVAL_SECS}` (R24). */
const REVERSE_MIN_STEPS = 2;
const REVERSE_MAX_STEPS = 1000;
const REVERSE_MIN_INTERVAL_SECS = 900;

/** `template.rs::encode_lock_time`'s bound (R3): a lock time is below 2^40 seconds. */
const LOCK_TIME_LIMIT = 2 ** 40;
/** `sell.rs::LOCK_TIME_UNIT_SECS`: R3 encodes a lock time in 512 s units. */
const LOCK_TIME_UNIT_SECS = 512;

/** "0.001000 HNS", as the backend's `doos_to_hns_string` writes it. */
function doosText(doos: bigint): string {
  return `${doos / 1_000_000n}.${(doos % 1_000_000n).toString().padStart(6, "0")} HNS`;
}

/**
 * Rust's `str::trim`: it strips Unicode White_Space (`char::is_whitespace`),
 * which differs from JS `trim()`: U+0085 is whitespace in Rust and not in
 * JS; U+FEFF is whitespace in JS and not in Rust.
 */
const RUST_WHITESPACE = "\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000";
const RUST_TRIM = new RegExp(`^[${RUST_WHITESPACE}]+|[${RUST_WHITESPACE}]+$`, "g");

/**
 * A character Rust's `{:?}` escapes as `\u{..}`: not printable (control,
 * format, unassigned, private use, surrogate, line and paragraph separators,
 * spaces but U+0020) or a grapheme extender. Unicode tables follow the JS
 * engine, so a character whose category changed between Unicode versions
 * may differ.
 */
const RUST_DEBUG_HEX =
  /[\p{Cc}\p{Cf}\p{Cn}\p{Co}\p{Cs}\p{Zl}\p{Zp}\p{Grapheme_Extend}]|(?! )\p{Zs}/u;

/** A string as Rust's `{:?}` writes it (the backend quotes the typed price so). */
function rustDebug(text: string): string {
  let out = "";
  for (const c of text) {
    if (c === "\\" || c === '"') out += `\\${c}`;
    else if (c === "\t") out += "\\t";
    else if (c === "\n") out += "\\n";
    else if (c === "\r") out += "\\r";
    else if (c === "\0") out += "\\0";
    else if (RUST_DEBUG_HEX.test(c)) out += `\\u{${(c.codePointAt(0) ?? 0).toString(16)}}`;
    else out += c;
  }
  return `"${out}"`;
}

/**
 * R19's price rule, the twin of `sell.rs::parse_step_price`: the doos, or
 * the backend's own reason. Never rounds. Pinned against the backend by
 * `src/test/fixtures/sell-price-cases.json`.
 */
export function parseStepPrice(text: string): { doos: bigint } | { error: string } {
  const bad = (why: string) => ({ error: `price ${rustDebug(text)}: ${why}` });
  const t = text.replace(RUST_TRIM, "");
  if (t === "") return bad("enter a price in HNS");
  // No '.' is a whole number of HNS; a second '.' lands in `frac` and is not a digit.
  const dot = t.indexOf(".");
  const whole = dot < 0 ? t : t.slice(0, dot);
  const frac = dot < 0 ? "" : t.slice(dot + 1);
  const digits = (s: string) => /^[0-9]*$/.test(s);
  if ((whole === "" && frac === "") || !digits(whole) || !digits(frac)) {
    return bad("not a number of HNS");
  }
  if (frac.length > 6) return bad("HNS has at most 6 decimals");
  const doos = BigInt(whole === "" ? "0" : whole) * 1_000_000n + BigInt(frac.padEnd(6, "0"));
  if (doos === 0n) return bad("a price must be above 0");
  if (doos < DUST_DOOS) return bad(`below the dust limit of ${doosText(DUST_DOOS)}`);
  if (doos > MAX_MONEY_DOOS) return bad("above the money supply");
  return { doos };
}

/**
 * R24, the twin of `sell.rs::parse_schedule`: the backend's reason, or null.
 * The start price is read first, then the floor, then the schedule's rules
 * in `reverse_schedule`'s order, the steps' lock times counted from 0: the
 * last one, `(steps − 1) · interval` rounded up to R3's 512 s unit as every
 * step after the first is, must fit R3's 40 bits. Finalize & sign
 * builds the lock times again from the node's median time, which the
 * renderer does not know, so the backend may still refuse a schedule whose
 * last step fits from 0 but not from that time.
 */
export function scheduleProblem(s: ShakedexScheduleInput): string | null {
  const bad = (why: string) => `reverse auction: ${why}`;
  const stepCount = () =>
    bad(`${s.steps} steps: a schedule has ${REVERSE_MIN_STEPS} to ${REVERSE_MAX_STEPS} steps`);
  // The backend takes a u32 and a u64: Tauri refuses anything else before
  // the command reads a price, so these come first.
  if (!Number.isInteger(s.steps)) return stepCount();
  if (!Number.isInteger(s.intervalSecs) || s.intervalSecs < 0) {
    return bad("the time between steps must be a whole number of seconds");
  }
  const start = parseStepPrice(s.startPrice);
  if ("error" in start) return start.error;
  const floor = parseStepPrice(s.floorPrice);
  if ("error" in floor) return floor.error;
  if (!(s.steps >= REVERSE_MIN_STEPS && s.steps <= REVERSE_MAX_STEPS)) return stepCount();
  if (s.intervalSecs < REVERSE_MIN_INTERVAL_SECS) {
    return bad(`steps ${s.intervalSecs} s apart: steps must be at least 15 minutes apart`);
  }
  if (floor.doos >= start.doos) {
    return bad(`the floor ${doosText(floor.doos)} is not below the start ${doosText(start.doos)}`);
  }
  const afterFirst = BigInt(s.steps - 1);
  if (start.doos - floor.doos < afterFirst) {
    return bad(
      `prices would not be strictly falling: ${s.steps} steps need the start at least ${afterFirst} doos above the floor`,
    );
  }
  // `template.rs::encode_lock_time`: a lock time is below 2^40 s, and the
  // last step's is rounded up to 512 s (`sell.rs::reverse_schedule`); 2^40
  // is a multiple of 512, so it fits exactly when it is at most 2^40 − 512.
  // Below 2^53 the product is exact, and rounding a larger one cannot bring
  // it under that, so this refuses exactly what the backend refuses.
  if ((s.steps - 1) * s.intervalSecs > LOCK_TIME_LIMIT - LOCK_TIME_UNIT_SECS) {
    return bad("the last step's time does not fit a lock time");
  }
  return null;
}

/**
 * R26, the twin of `sell::check_lower_price` after R19's price rule: the
 * backend's reason for a new price that cannot be signed, or null. The
 * current price is the node's (`ListingPrices.at`); without one only the
 * price rule is checked here and the backend compares at the median time.
 */
export function lowerPriceProblem(text: string, currentPrice: number | null): string | null {
  const p = parseStepPrice(text);
  if ("error" in p) return p.error;
  if (currentPrice != null && p.doos >= BigInt(currentPrice)) {
    return `${doosText(p.doos)} is not below the current price ${doosText(BigInt(currentPrice))}: a price can only be lowered; raising it needs a cancel`;
  }
  return null;
}

/**
 * A listing's transaction that failed to broadcast and that its My listings
 * row still offers (`pending_send`): never sent, so it can be discarded too
 * (`discardable`), or held as `broadcast_pending`, which only a new send
 * resolves.
 */
export const UNSENT_KEPT =
  "The signed transaction stays in My listings, where it can be sent or discarded.";
export const SEND_AGAIN_KEPT =
  "The signed transaction stays in My listings, where it can be sent again.";

/**
 * A broadcast failure's reason, followed by where its draft is kept when a
 * fresh read of My listings shows a row offering it (`kept`); the reason
 * alone otherwise (the node refused it, or R31's recheck deleted a lock with
 * its listing: the backend's error says why).
 */
export function withKept(why: string, kept: ShakedexPendingSend | null): string {
  if (!kept) return why;
  return `${why.replace(/[.\s]+$/, "")}. ${kept.discardable ? UNSENT_KEPT : SEND_AGAIN_KEPT}`;
}

/**
 * Where a listing goes once its unsent draft is discarded, as
 * `queries::delete_tx_draft` moves it: a Locking listing goes with its lock,
 * an unsent FINALIZE into the lock takes its steps back to Ready to
 * Finalize & sign, an unsent cancel leaves the listing as it was (Listed, or
 * Restored without a file), an unsent cancel FINALIZE awaits it again.
 */
export function discardedText(state: ShakedexListingState): string {
  switch (state) {
    case "locking":
      return "Lock discarded. Nothing was sent: the name stays yours.";
    case "finalizing":
      return `Finalize discarded. The listing is back to ${listingStateLabel("readyToFinalize", null)}.`;
    case "cancelling":
      return "Cancel discarded. The listing is back as it was before the cancel.";
    case "cancelFinalizing":
      return `Finalize discarded. The listing is back to ${listingStateLabel("cancelAwaitingFinalize", null)}.`;
    default:
      return "Discarded";
  }
}

/**
 * R28's first sentence for a cancel, the twin of the backend's
 * `cheaper_ahead` (`commands/shakedex.rs`, `cancel::still_buyable`) over the
 * row's prices: a cheaper step ahead of the current one (`nextPrice`, which
 * `template::next_step_index` gives only below it); with no step valid yet,
 * any step ahead, for a reverse auction only; never without the node's
 * median time. The secure window shows the backend's own at the cancel.
 */
export function cancelStillBuyable(l: ShakedexListingSummary, p: ShakedexListingPrices): string {
  const cheaperAhead =
    p.kind === "at" &&
    p.nextPrice != null &&
    (p.currentPrice != null || l.mode === "reverseAuction");
  return cheaperAhead ? CANCEL_STILL_BUYABLE_AS_IT_FALLS : CANCEL_STILL_BUYABLE;
}

/** Carry 6: the warning for a schedule whose last step comes more than a year after its first. */
export function scheduleWarning(steps: number, intervalSecs: number): string | null {
  return (steps - 1) * intervalSecs > LISTING_LIFETIME_SECS ? SCHEDULE_PAST_A_YEAR : null;
}

/**
 * Why a new listing (Sell / Lock) is refused here, in the order
 * `shakedex_build_lock_draft` runs its gates: profile (R16), node (R6), then
 * the mainnet selling flag (R15, R29), which only `"true"` enables. `null`
 * when nothing known refuses it.
 */
export function sellRefusal(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
  network: WalletNetwork | undefined,
  experimental: string | undefined,
): string | null {
  const refusal = shakedexRefusal(kind, writeCap);
  if (refusal) return refusal;
  if (network === "mainnet" && !settingToBool(experimental)) return MAINNET_SELLING_EXPERIMENTAL;
  return null;
}

/** Whether a new listing can be made: `canUseShakedex` plus the mainnet selling flag. */
export function canSellShakedex(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
  network: WalletNetwork | undefined,
  experimental: string | undefined,
): boolean {
  return (
    canUseShakedex(kind, writeCap) &&
    network !== undefined &&
    sellRefusal(kind, writeCap, network, experimental) === null
  );
}

const STATE_LABEL: Record<ShakedexListingState, string> = {
  locking: "Locking",
  readyToFinalize: "Ready to Finalize & sign",
  finalizing: "Finalizing into the lock",
  listed: "Listed for sale",
  salePending: "Sale pending",
  sold: "Sold",
  cancelling: "Cancelling",
  cancelAwaitingFinalize: "Cancelled · finalize to bring it home",
  cancelFinalizing: "Coming home",
  cancelled: "Cancelled",
  aborted: "Lock aborted",
  restored: RESTORED_LABEL,
  expired: "Expired in the lock",
};

/** A listing's state in a few words, the same on Owned Names and My listings. */
export function listingStateLabel(
  state: ShakedexListingState,
  blocksUntilFinalize: number | null,
): string {
  if (state === "locking" && blocksUntilFinalize != null && blocksUntilFinalize > 0) {
    return `Locking · ${blocksUntilFinalize} ${blocksUntilFinalize === 1 ? "block" : "blocks"}`;
  }
  return STATE_LABEL[state];
}

/** A Restored lock's mode is not known (carry 7): no mode is shown for it. */
export function modeText(l: ShakedexListingSummary): string | null {
  if (l.state === "restored") return null;
  return l.mode === "reverseAuction" ? "Reverse auction" : "Buy now";
}

/**
 * A My listings row's prices (R12) in one line, or null when there is none
 * to show: no steps here, or any state but Listed. Only a Listed listing's
 * prices are ones anyone can buy at; every other state's stored steps still
 * give prices (an ended or cancelled listing, a sale pending, a lock not yet
 * finalized), so none is shown.
 */
export function listingPricesText(
  l: ShakedexListingSummary,
  p: ShakedexListingPrices,
): string | null {
  if (l.state !== "listed") return null;
  switch (p.kind) {
    case "noSteps":
      return null;
    case "unverified":
      return "Current price not checked — needs a full or remote node";
    case "couldNotCheck":
      return `Current price could not be checked: ${nodeReason(p.reason)}`;
    case "unreadable":
      return `The stored prices cannot be read: ${p.reason}`;
    case "at": {
      const now =
        p.currentPrice == null ? "No price step valid yet" : `Now ${formatHns(p.currentPrice)} HNS`;
      if (l.mode !== "reverseAuction") {
        return p.currentPrice == null ? now : `Price ${formatHns(p.currentPrice)} HNS`;
      }
      const parts = [now];
      if (p.nextPrice != null) {
        const when = p.nextValidInSecs != null ? ` in ${approxWait(p.nextValidInSecs)}` : "";
        parts.push(`next ${formatHns(p.nextPrice)} HNS${when}`);
      }
      parts.push(`floor ${formatHns(p.floorPrice)} HNS`);
      return parts.join(" · ");
    }
  }
}

/** "at 14:05" from an RFC 3339 time; null for one no date can hold. */
function clockText(iso: string | null): string | null {
  if (iso == null) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d.toLocaleString();
}

/**
 * The states no sync puts on the market: an ended listing
 * (`ListingState::TERMINAL`) and a Restored lock, which has no listing file.
 */
const NOT_ON_MARKET: ShakedexListingState[] = [
  "sold",
  "cancelled",
  "aborted",
  "expired",
  "restored",
];

/** The states before the listing is in its lock (Listed). */
const BEFORE_LISTED: ShakedexListingState[] = ["locking", "readyToFinalize", "finalizing"];

/** Where a listing stands on LearnHNS Market (R23, R25, R28), in one sentence. */
export function marketStatusText(
  l: ShakedexListingSummary,
  network: WalletNetwork | undefined,
): { tone: StatusTone; text: string } {
  if (!l.publish) {
    return network === "mainnet"
      ? { tone: "muted", text: "Not published on LearnHNS Market — shared as a file only" }
      : { tone: "muted", text: SHARED_AS_FILE_ONLY };
  }
  const why = l.marketError ? `: “${l.marketError}”` : "";
  switch (l.marketStatus) {
    case null:
      if (NOT_ON_MARKET.includes(l.state)) return { tone: "muted", text: "Not on LearnHNS Market" };
      return BEFORE_LISTED.includes(l.state)
        ? { tone: "muted", text: "Announced on LearnHNS Market once the lock transfer is sent" }
        : { tone: "muted", text: "Goes on LearnHNS Market at the next sync" };
    case "pending":
      return { tone: "ok", text: "On LearnHNS Market as a pending listing" };
    case "listed":
      return { tone: "ok", text: "On LearnHNS Market" };
    case "replacedReuploaded":
      return { tone: "warn", text: "Replaced on the market by someone else's copy — re-uploaded" };
    case "retrying": {
      const at = clockText(l.marketRetryAt);
      return { tone: "warn", text: `Not on market — retrying${at ? ` at ${at}` : ""}${why}` };
    }
    case "refused":
      return BEFORE_LISTED.includes(l.state)
        ? {
            tone: "warn",
            text: `LearnHNS Market refused the pending listing${why}. The listing is uploaded once the finalize into the lock is mined.`,
          }
        : {
            tone: "bad",
            text: `LearnHNS Market refused the listing${why}. It is sent again only when its price or expiry changes.`,
          };
    case "stepsUnverified":
      return {
        tone: "bad",
        text: "Not on market: a signed price does not verify over the lock coin your node reports, so nothing was uploaded",
      };
    case "reported":
      return l.marketError
        ? {
            tone: "muted",
            text: `LearnHNS Market refused the report of the end of this listing${why}`,
          }
        : { tone: "muted", text: "LearnHNS Market was told this listing ended" };
  }
}

/** When a stored step becomes valid, as a date; null for a lock time no date can hold (u64 from a file). */
export function stepTimeText(lockTime: number): string | null {
  const d = new Date(lockTime * 1000);
  return Number.isNaN(d.getTime()) ? null : d.toLocaleString();
}

/** A button of My listings: an action of `ShakedexListingActions`. */
export type ListingButton = keyof ShakedexListingActions;

/** The Market page's tab that holds the seller's listings. */
export const MY_LISTINGS_TAB = "My listings";

/** Each My listings button's label, as other screens name it too. */
export const LISTING_BUTTON_LABEL: Record<ListingButton, string> = {
  finalizeAndSign: "Finalize & sign",
  lowerPrice: "Lower price",
  cancel: "Cancel listing",
  finalizeCancel: "Finalize home",
  export: "Export listing file",
  publish: "Publish on LearnHNS Market",
  importFile: "Import its listing file",
  manageHere: MANAGE_HERE_LABEL,
};

/**
 * R44, R45: the buttons a Listed row shows besides its own while another
 * device or profile of the phrase manages it: take it over, or import that
 * device's listing file.
 */
export const MANAGED_ELSEWHERE_BUTTONS: ListingButton[] = ["manageHere", "importFile"];

/** The buttons each state shows in My listings; whether each is enabled is the backend's `actions`. */
export const LISTING_BUTTONS: Record<ShakedexListingState, ListingButton[]> = {
  locking: ["finalizeAndSign"],
  readyToFinalize: ["finalizeAndSign"],
  finalizing: [],
  listed: ["lowerPrice", "cancel", "export", "publish"],
  salePending: ["export"],
  cancelling: ["lowerPrice", "export"],
  cancelAwaitingFinalize: ["finalizeCancel", "export"],
  cancelFinalizing: ["export"],
  sold: [],
  cancelled: [],
  aborted: [],
  expired: [],
  restored: ["cancel", "importFile"],
};
