/**
 * The exchange's numbers, read and shown exactly.
 *
 * Prices are sats per HNS with at most 3 decimals, so the exact unit of a
 * price is the millisat (1 sat = 1000 msat). HNS amounts are read with
 * `hnsAmount.ts` (doos). The BTC a trade moves is
 * `floor(doos × msatPerHns / 1e9)` sats, computed in BigInt so no amount or
 * price, however large, goes through a float or throws.
 *
 * Every number the backend or the relay sends is a u64 that may not fit a JS
 * number: the `…Text` helpers return null for one they cannot show exactly.
 */
import { DOOS_PER_HNS, hnsDigitsToDoos } from "./hnsAmount";

/** Millisats in one sat. */
export const MSAT_PER_SAT = 1000n;
/** Sats in one BTC. */
export const SATS_PER_BTC = 100_000_000n;
/** Every BTC that will ever exist, in sats (21 million BTC). */
export const MAX_BTC_SATS = 2_100_000_000_000_000n;

/** Shown under a price field that is not a price, or is 0. */
export const PRICE_NOT_POSITIVE = "Enter a price greater than 0, in sats per HNS like 250 or 250.5";
/** Shown under a price field with a 4th decimal. */
export const PRICE_DECIMALS = "A price has at most 3 decimals of a sat";

/** A price's text, read: millisats per HNS, or the sentence its field shows. */
export type PriceRead = { msat: bigint } | { error: string };

/**
 * Read a price in sats per HNS: plain ASCII digits with at most one point and
 * at most 3 decimals, greater than 0. No sign, exponent or separator; never
 * rounds. The same rule reads a price the user types and a price an offer
 * carries.
 */
export function parsePriceMsat(text: string): PriceRead {
  const t = text.trim();
  const dot = t.indexOf(".");
  const whole = dot < 0 ? t : t.slice(0, dot);
  const frac = dot < 0 ? "" : t.slice(dot + 1);
  const digits = (s: string) => /^[0-9]*$/.test(s);
  if ((whole === "" && frac === "") || !digits(whole) || !digits(frac)) {
    return { error: PRICE_NOT_POSITIVE };
  }
  if (frac.length > 3) return { error: PRICE_DECIMALS };
  const msat = BigInt(whole === "" ? "0" : whole) * MSAT_PER_SAT + BigInt(frac.padEnd(3, "0"));
  if (msat === 0n) return { error: PRICE_NOT_POSITIVE };
  return { msat };
}

/** Millisats per HNS as plain price digits ("250", "250.5", "0.001"), no trailing zeros. */
export function msatToPriceDigits(msat: bigint): string {
  const whole = (msat / MSAT_PER_SAT).toString();
  const frac = (msat % MSAT_PER_SAT).toString().padStart(3, "0").replace(/0+$/, "");
  return frac === "" ? whole : `${whole}.${frac}`;
}

/** The sats `doos` of HNS cost at `msatPerHns`: `floor(doos × msat / 1e9)`. */
export function btcSatsFor(doos: bigint, msatPerHns: bigint): bigint {
  return (doos * msatPerHns) / (DOOS_PER_HNS * MSAT_PER_SAT);
}

/** An HNS amount an offer carries (`minHns`, a trade's `hnsAmount`), as doos, or null. */
export function offerHnsDoos(text: string): bigint | null {
  const read = hnsDigitsToDoos(text.trim());
  return "doos" in read ? read.doos : null;
}

/** An integer with thousands separators, from a BigInt ("1,234,567"). */
function grouped(n: bigint): string {
  return n.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/**
 * A whole number of sats the backend sent, as a BigInt: null for anything that
 * is not a safe non-negative integer (a u64 past 2^53 has already lost its
 * last digits on the way into JS, so it cannot be shown exactly).
 */
export function satsValue(sats: number): bigint | null {
  if (typeof sats !== "number" || !Number.isSafeInteger(sats) || sats < 0) return null;
  return BigInt(sats);
}

/** "1,234,567 sats", or null for a value that is not a safe whole number of sats. */
export function satsText(sats: number | bigint): string | null {
  const v = typeof sats === "bigint" ? (sats < 0n ? null : sats) : satsValue(sats);
  if (v === null) return null;
  return `${grouped(v)} ${v === 1n ? "sat" : "sats"}`;
}

/** "0.01234567 BTC" (always 8 decimals), or null for a value that cannot be shown. */
export function btcText(sats: number | bigint): string | null {
  const v = typeof sats === "bigint" ? (sats < 0n ? null : sats) : satsValue(sats);
  if (v === null) return null;
  const frac = (v % SATS_PER_BTC).toString().padStart(8, "0");
  return `${grouped(v / SATS_PER_BTC)}.${frac} BTC`;
}

/** What a field shows for a number it cannot show exactly. */
export const UNREADABLE = "—";

/** A price the backend or the relay sent, for display ("250.5 sats/HNS"), or null if it is not a price. */
export function priceText(price: string): string | null {
  const read = parsePriceMsat(price);
  if ("error" in read) return null;
  const digits = msatToPriceDigits(read.msat);
  const dot = digits.indexOf(".");
  const whole = dot < 0 ? digits : digits.slice(0, dot);
  const frac = dot < 0 ? "" : digits.slice(dot);
  return `${grouped(BigInt(whole))}${frac} sats/HNS`;
}

/** An HNS amount the backend or the relay sent, for display ("1,250.5 HNS"), or null if it is not one. */
export function hnsText(hns: string): string | null {
  const doos = offerHnsDoos(hns);
  if (doos === null) return null;
  const frac = (doos % DOOS_PER_HNS).toString().padStart(6, "0").replace(/0+$/, "");
  return `${grouped(doos / DOOS_PER_HNS)}${frac === "" ? "" : `.${frac}`} HNS`;
}

/** A sats amount the user typed (Withdraw BTC), read: the sats, or the sentence the field shows. */
export type SatsRead = { sats: number } | { error: string };

export const SATS_NOT_POSITIVE = "Enter a whole number of sats greater than 0, like 25000";
export const SATS_ABOVE_SUPPLY = "That is more than the 21 million BTC that will ever exist";

/** Read a whole number of sats: digits only, greater than 0, at most 21 million BTC. */
export function parseSatsAmount(text: string): SatsRead {
  const t = text.trim();
  if (!/^[0-9]+$/.test(t)) return { error: SATS_NOT_POSITIVE };
  const v = BigInt(t);
  if (v === 0n) return { error: SATS_NOT_POSITIVE };
  if (v > MAX_BTC_SATS) return { error: SATS_ABOVE_SUPPLY };
  // At most 2.1e15: a safe integer.
  return { sats: Number(v) };
}
