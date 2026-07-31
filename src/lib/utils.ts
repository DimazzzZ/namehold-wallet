import type { TxSummary } from "../types";
import { toAceName } from "./idnEncode";

export function dollarydoosToHns(dollarydoos: number): string {
  return addThousandSeparators((dollarydoos / 1_000_000).toFixed(6));
}

/**
 * Doos that actually LEAVE the wallet for a draft (excluding the fee, which is
 * shown separately). Covenant name-actions (update/renew/register/reveal/bid/
 * open/redeem/cancel) carry the name's locked value onto your OWN new coin —
 * the backend funds that output by spending the name coin itself, so nothing
 * leaves beyond the fee → 0. Only a genuine transfer to another party
 * (send_hns / transfer / finalize — the actions that set `recipientAddress`)
 * moves value out of the wallet.
 *
 * This is why a DNS UPDATE is NOT "222 HNS": that 222 is the name's locked
 * value being re-homed to your own coin, not a cost. `sendTotalDoos` (the
 * primary output value) is only a real outflow when it goes to a recipient.
 */
export function netSpendDoos(
  s: Pick<TxSummary, "sendTotalDoos" | "recipientAddress">,
): number {
  return s.recipientAddress != null ? s.sendTotalDoos : 0;
}

/**
 * Insert commas every three digits left of the decimal point.
 * "1234567.123456" → "1,234,567.123456"
 * Purely cosmetic — never touches IDs or non-numeric strings.
 */
function addThousandSeparators(nStr: string): string {
  const [intPart = "", decPart] = nStr.split(".");
  const withCommas = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return decPart != null ? `${withCommas}.${decPart}` : withCommas;
}

/**
 * Format an integer count (domains, selected items, etc.) with thousand
 * separators. NOT for IDs — those stay raw.
 */
export function formatCount(value: number): string {
  return addThousandSeparators(String(value));
}

export function hnsToDollarydoos(hns: string): number {
  return Math.round(parseFloat(hns) * 1_000_000);
}

/** The bech32 HRP+separator an HNS address starts with on a given network. */
export function hnsAddressPrefix(network: string): string {
  switch (network) {
    case "testnet":
      return "ts1";
    case "regtest":
      return "rs1";
    case "simnet":
      return "ss1";
    default:
      return "hs1"; // mainnet
  }
}

/**
 * Lightweight, network-aware format check for inline UI feedback — verifies the
 * bech32 prefix matches the network and the shape is plausible. NOT a checksum;
 * the Rust `address::decode` remains the source of truth at build time.
 */
export function isLikelyHnsAddress(addr: string, network: string): boolean {
  const a = addr.trim().toLowerCase();
  return (
    a.startsWith(hnsAddressPrefix(network)) &&
    a.length >= 40 &&
    a.length <= 90 &&
    /^[a-z0-9]+$/.test(a)
  );
}

export function formatHns(dollarydoos: number | null | undefined): string {
  if (dollarydoos == null) return "—";
  return dollarydoosToHns(dollarydoos);
}

/**
 * Format a decimal HNS amount with thousands separators and exactly 6
 * fractional digits — matching the precision used by `formatHns` (which
 * converts from dollarydoos via `.toFixed(6)`).
 *
 * Examples: 120002.4 → "120,002.400000", 1000 → "1,000.000000",
 *           0.5 → "0.500000"
 */
export function formatHnsAmount(value: number | string): string {
  const num = typeof value === "string" ? Number(value) : value;
  if (!Number.isFinite(num)) return String(value);
  return num.toLocaleString("en-US", {
    minimumFractionDigits: 6,
    maximumFractionDigits: 6,
  });
}

export function cn(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(" ");
}

// Normalize before parsing so we don't double-stamp a timezone. Inputs vary:
//   * Namebase ISO already carries a tz: "2026-06-26T00:00:00Z" / "…+02:00"
//   * SQLite naive UTC: "2026-06-26 00:00:00" (space, no tz)
//   * date-only: "2026-06-26"
// A naive value is treated as UTC; a value that already has a tz is left as-is.
function normalizeTimestamp(s: string): string {
  const hasTz = /[zZ]$/.test(s) || /[+-]\d{2}:?\d{2}$/.test(s);
  if (hasTz) return s;
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return `${s}T00:00:00Z`;
  return `${s.replace(" ", "T")}Z`;
}

/**
 * Deterministic date + time format, e.g. `"July 24, 2026 - 12:54:25"` — no
 * locale-dependent d/m/y ordering. The unified timestamp format for all
 * transaction/history rows, the tx/block info modals, and the Namebase
 * dashboard, so the same moment reads identically everywhere (readability
 * wins over compactness).
 *
 * Uses `normalizeTimestamp` for input-shape tolerance (naive-UTC SQLite
 * strings, date-only, ISO w/ tz). Returns `"—"` for null/empty and falls
 * back to the raw input on unparseable values (never `"Invalid Date"`).
 *
 * For date-only inputs (e.g. "2026-07-24"), returns just the date part
 * (e.g. "July 24, 2026") without the time suffix.
 */
export function formatDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  const s = iso.trim();
  if (!s) return "—";

  const d = new Date(normalizeTimestamp(s));
  if (Number.isNaN(d.getTime())) return s;
  const datePart = d.toLocaleDateString("en-US", {
    year: "numeric",
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  });
  // Date-only inputs (e.g. "2026-07-24") normalize to midnight UTC — showing
  // "- 00:00:00" for them would be noise, so omit the time in that case.
  const dateOnly = /^\d{4}-\d{2}-\d{2}$/.test(s);
  if (dateOnly) return datePart;
  const timePart = d.toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "UTC",
  });
  return `${datePart} - ${timePart}`;
}

/**
 * Classify a history/activity row's amount for coloring and sign:
 * - `"income"`  → money entered the wallet (green, leading `+`).
 * - `"spend"`   → money genuinely left the wallet (red, leading `-`).
 * - `"neutral"` → self-homed name actions with `valueDoos === 0`, internal
 *   moves, or any zero-magnitude row (gray, no sign).
 *
 * The rule mirrors `netSpendDoos`: a DNS UPDATE / BID / REGISTER / RENEW /
 * REVEAL / REDEEM re-homes the name's own locked value onto our new coin
 * (`valueDoos === 0`) — nothing was lost, so it MUST NOT paint red.
 */
export function amountTone(row: {
  direction: string;
  valueDoos: number;
}): "income" | "spend" | "neutral" {
  if (row.direction === "receive" && row.valueDoos > 0) return "income";
  if (row.direction === "send" && row.valueDoos < 0) return "spend";
  return "neutral";
}

/**
 * The more recent of two nullable timestamps (same SQLite/ISO formats
 * `formatDate` understands), or whichever one is present when only one is,
 * or `null` when neither is. Used to show a single "Last successful sync"
 * line that reflects whichever sync path — node-RPC (`lastSyncedAt`) or
 * explorer (`lastExplorerSyncAt`) — most recently completed, since only ONE
 * of the two advances in a given sync mode (Task 11 review, Finding 2).
 */
export function latestTimestamp(
  a: string | null | undefined,
  b: string | null | undefined
): string | null {
  const at = a?.trim();
  const bt = b?.trim();
  if (!at) return bt || null;
  if (!bt) return at;
  const da = new Date(normalizeTimestamp(at));
  const db = new Date(normalizeTimestamp(bt));
  if (Number.isNaN(da.getTime())) return bt;
  if (Number.isNaN(db.getTime())) return at;
  return db > da ? bt : at;
}

export function truncate(str: string, len: number): string {
  if (str.length <= len) return str;
  return str.slice(0, len) + "...";
}

/**
 * Middle-truncate a long opaque string (xpub, txid, profile id) for display:
 * keeps the first `head` and last `tail` characters with an ellipsis between,
 * e.g. "xpub6CUG…FDVmz". Strings short enough to show in full are returned
 * unchanged, so the ellipsis never appears when it wouldn't save space.
 */
export function truncateMiddle(s: string, head = 8, tail = 6): string {
  if (!s || s.length <= head + tail + 1) return s;
  return s.slice(0, head) + "…" + s.slice(-tail);
}

/**
 * Normalize a user-typed name into a form suitable for hsd name lookups.
 *
 * - trims whitespace
 * - lowercases
 * - strips a leading `.` (users sometimes type `.example`)
 * - strips a trailing `.hsd` suffix (common TLD confusion)
 * - collapses repeated dots
 *
 * Returns an empty string when the input is blank after normalization.
 */
export function normalizeNameInput(raw: string): string {
  let name = raw.trim().toLowerCase();
  if (!name) return "";
  // Strip leading dot: ".example" → "example"
  name = name.replace(/^\.+/, "");
  // Strip trailing .hsd suffix: "example.hsd" → "example"
  name = name.replace(/\.hsd$/i, "");
  // Keep only valid Handshake name characters: a-z, 0-9, hyphens, dots
  name = name.replace(/[^a-z0-9.\-]/g, "");
  // Collapse repeated dots: "a..b" → "a.b"
  name = name.replace(/\.{2,}/g, ".");
  // Strip any remaining leading/trailing dots after collapse
  name = name.replace(/^\.+|\.+$/g, "");
  return name;
}

/**
 * Normalize a user-typed name for the Auctions "Get a TLD" input, with
 * Unicode → ACE (Punycode) encoding support.
 *
 * Like `normalizeNameInput`, this:
 * - lowercases
 * - strips a leading `.` (users sometimes type `.example`)
 * - strips a trailing `.hsd` suffix (common TLD confusion)
 * - collapses repeated dots
 *
 * UNLIKE `normalizeNameInput`, this preserves and encodes Unicode characters
 * to their ACE form (e.g. "сбер" → "xn--90ai7ab") instead of stripping them.
 * The result is safe to send to backend commands.
 *
 * Returns an empty string when the input is blank after normalization or
 * cannot be encoded.
 */
export function normalizeNameInputAce(raw: string): string {
  let name = raw.trim().toLowerCase();
  if (!name) return "";
  // Strip leading dot: ".example" → "example"
  name = name.replace(/^\.+/, "");
  // Strip trailing .hsd suffix: "example.hsd" → "example"
  name = name.replace(/\.hsd$/i, "");
  // Strip characters that are invalid in Handshake names: keep Unicode letters,
  // digits, hyphens, underscores, and dots (for multi-label names). This
  // preserves Cyrillic/CJK/etc. for encoding, while stripping spaces, punctuation, etc.
  name = name.replace(/[^\p{L}\p{N}._\-]/gu, "");
  // Collapse repeated dots: "a..b" → "a.b"
  name = name.replace(/\.{2,}/g, ".");
  // Strip any remaining leading/trailing dots after collapse
  name = name.replace(/^\.+|\.+$/g, "");

  // Encode to ACE form (tr46 handles dotted names natively).
  // This preserves Unicode input and converts it to the on-chain punycode form.
  return toAceName(name);
}
