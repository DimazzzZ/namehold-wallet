/**
 * Fee-rate unit adapter for the UI.
 *
 * The Rust backend uses two related-but-different units:
 *   - The persistent setting `fee_rate_doos_per_kvb` is stored in
 *     **doos per kvB** (doos per 1000 vbytes), matching hsd's `rate`
 *     convention. The backend divides by 1000 to get sats/byte.
 *   - Every draft-builder command accepts an optional `fee_rate` argument
 *     already in **sats/byte** (dollarydoos per vbyte).
 *
 * To keep one mental model for users, the UI **always** takes input in
 * doos/kvB. This module is the seam that converts to sats/byte right before
 * calling a draft-builder.
 *
 * Floor: 1000 doos/kvB == 1 sat/byte, matching Rust
 * `noncustodial::send::MIN_FEE_RATE_PER_BYTE`.
 */

/** Minimum accepted fee rate in doos/kvB (== 1 sat/byte in the Rust layer). */
export const MIN_FEE_RATE_DOOS_PER_KVB = 1000;

/** Default fee rate in doos/kvB when nothing is configured (matches Rust default of 1 sat/byte). */
export const DEFAULT_FEE_RATE_DOOS_PER_KVB = 1000;

/**
 * Parse a user-typed doos/kvB string. Returns null on empty/invalid, otherwise
 * the integer value floored at the minimum. Callers decide whether "empty"
 * means "fall back to setting default" (per-tx override) or "reject".
 */
export function parseDoosPerKvb(raw: string): number | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n) || !Number.isInteger(n) || n <= 0) return null;
  return Math.max(n, MIN_FEE_RATE_DOOS_PER_KVB);
}

/**
 * Convert a doos/kvB value into the sats/byte unit expected by every
 * `build_*_draft` command's optional `fee_rate` parameter. Returns null when
 * the input is null, so callers can pass the result straight through to
 * `feeRate: doosPerKvbToSatsPerByte(...)` without extra branching.
 */
export function doosPerKvbToSatsPerByte(doosPerKvb: number | null): number | null {
  if (doosPerKvb == null) return null;
  const satsPerByte = Math.floor(doosPerKvb / 1000);
  // Backend also floors to MIN_FEE_RATE_PER_BYTE = 1; mirror that here so
  // the UI-displayed effective rate never disagrees with what the backend uses.
  return Math.max(satsPerByte, 1);
}

/**
 * Format an integer doos/kvB value for display, with thousands separators.
 * The unit label is deliberately NOT included so callers can vary it (input
 * suffix, tooltip, etc.).
 */
export function formatDoosPerKvb(doosPerKvb: number): string {
  return doosPerKvb.toLocaleString();
}
