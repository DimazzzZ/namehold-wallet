/**
 * Unicode → ACE (Punycode) encoding for Handshake TLD names.
 *
 * This is the ENCODING counterpart to `idn.ts` (which is decode/display-only).
 * The output of `toAceName()` is intended to be sent to backend commands
 * (`read_name_info`, `build_open_draft`, etc.) — it produces the on-chain ACE
 * form (e.g. `xn--e1adigm`) that the Handshake protocol requires.
 *
 * Uses `tr46` (UTS-46 / IDNA compatibility processing) for correct Unicode
 * normalization, case folding, and punycode encoding.
 */

import tr46 from "tr46";

/**
 * Convert a user-typed name (possibly Unicode) to its ACE form for backend use.
 *
 * - ASCII-only input passes through unchanged (e.g. "example" → "example").
 * - Unicode input is encoded via UTS-46 (e.g. "козел" → "xn--e1adigm").
 * - Already-encoded ACE input (`xn--…`) passes through unchanged (idempotent).
 * - Returns `""` for empty input or any input that cannot be validly encoded.
 *
 * This function is intentionally permissive: it does not enforce DNS length
 * limits (the backend's `verify_name` handles that). It never throws.
 */
export function toAceName(raw: string): string {
  const trimmed = raw.trim().toLowerCase();
  if (!trimmed) return "";

  const result = tr46.toASCII(trimmed, {
    useSTD3ASCIIRules: false,
    verifyDNSLength: false,
  });

  // tr46.toASCII returns null on encoding failure — fall back to empty (silent).
  return result ?? "";
}
