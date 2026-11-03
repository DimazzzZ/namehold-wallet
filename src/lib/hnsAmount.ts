/**
 * HNS amounts typed by the user, read exactly.
 *
 * A doo (dollarydoo) is the chain's smallest unit and 1 HNS is 1,000,000 doos,
 * so an amount has at most 6 decimals. Every field where the user types an
 * amount of HNS that the wallet moves or locks (Send, Bid and Lockup, Batch
 * bid, Recover bid, the Namebase withdrawal) reads it through
 * `parseHnsAmount`, and the field's check and the conversion sent to the
 * backend are the same call, so they cannot disagree. Nothing here goes
 * through a float: the digits become a BigInt and an amount that does not fit
 * the rules is refused with the sentence its field shows, never rounded.
 */

/** Doos in one HNS. */
export const DOOS_PER_HNS = 1_000_000n;

/**
 * `purchase.rs::MAX_MONEY`: every HNS that will ever exist (2.04 billion), in
 * doos. It is below `Number.MAX_SAFE_INTEGER` (about 9.007e15), so an amount
 * at or under it converts to a JS number exactly.
 */
export const MAX_MONEY_DOOS = 2_040_000_000_000_000n;

/** The digits of a decimal HNS amount, as doos, or why they are not one. */
export type HnsDigits =
  { doos: bigint } | { error: "not a number of HNS" | "HNS has at most 6 decimals" };

/**
 * The core both the money fields and the Market's price rule
 * (`parseStepPrice`, the twin of `sell.rs::parse_step_price`) read digits
 * with. `t` is already trimmed. Only ASCII digits and at most one `.` count:
 * no sign, exponent, thousands separator or other notation, and at least one
 * digit on one side of the point (".5" and "5." are 0.5 and 5 HNS). Never
 * rounds; an amount past the 6th decimal is refused.
 */
export function hnsDigitsToDoos(t: string): HnsDigits {
  // No '.' is a whole number of HNS; a second '.' lands in `frac` and is not a digit.
  const dot = t.indexOf(".");
  const whole = dot < 0 ? t : t.slice(0, dot);
  const frac = dot < 0 ? "" : t.slice(dot + 1);
  const digits = (s: string) => /^[0-9]*$/.test(s);
  if ((whole === "" && frac === "") || !digits(whole) || !digits(frac)) {
    return { error: "not a number of HNS" };
  }
  if (frac.length > 6) return { error: "HNS has at most 6 decimals" };
  return { doos: BigInt(whole === "" ? "0" : whole) * DOOS_PER_HNS + BigInt(frac.padEnd(6, "0")) };
}

/** Shown under a money field whose text is not an amount of HNS, or is 0. */
export const HNS_AMOUNT_NOT_POSITIVE = "Enter an amount greater than 0, in digits like 12.5";
/** Shown under a money field with a 7th decimal. */
export const HNS_AMOUNT_DECIMALS = "HNS has at most 6 decimals";
/** Shown under a money field above the money supply. */
export const HNS_AMOUNT_ABOVE_SUPPLY =
  "That is more than the 2.04 billion HNS that will ever exist";

/** A money field's text, read: the doos, or the sentence the field shows. */
export type HnsAmount = { doos: number } | { error: string };

/**
 * Read an amount of HNS the user typed into a money field: the exact doos,
 * as a safe integer, or the sentence the field shows under it. Refused: an
 * empty field, anything that is not plain digits with at most one point,
 * more than 6 decimals, 0, and more than the money supply.
 */
export function parseHnsAmount(text: string): HnsAmount {
  const read = hnsDigitsToDoos(text.trim());
  if ("error" in read) {
    return {
      error:
        read.error === "HNS has at most 6 decimals" ? HNS_AMOUNT_DECIMALS : HNS_AMOUNT_NOT_POSITIVE,
    };
  }
  if (read.doos === 0n) return { error: HNS_AMOUNT_NOT_POSITIVE };
  if (read.doos > MAX_MONEY_DOOS) return { error: HNS_AMOUNT_ABOVE_SUPPLY };
  return { doos: Number(read.doos) };
}

/**
 * Doos as plain HNS digits for a money field to hold ("2043.66", "0.000001",
 * "3"): no thousands separators, no trailing zeros, and `parseHnsAmount` reads
 * it back to the same doos. Throws on a value that is not a whole number of
 * doos from 0 up, which no balance or draft the backend returns can be.
 */
export function doosToHnsDigits(doos: number): string {
  if (!Number.isSafeInteger(doos) || doos < 0) {
    throw new RangeError(`not a whole number of doos: ${doos}`);
  }
  const d = BigInt(doos);
  const whole = (d / DOOS_PER_HNS).toString();
  const frac = (d % DOOS_PER_HNS).toString().padStart(6, "0").replace(/0+$/, "");
  return frac === "" ? whole : `${whole}.${frac}`;
}
