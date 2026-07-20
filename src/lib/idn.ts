/**
 * Punycode → Unicode DISPLAY-ONLY decoding for Handshake TLDs.
 *
 * Handshake names are frequently registered as IDNs, which show up
 * everywhere on-chain and in our backend as their ACE (ASCII-Compatible
 * Encoding) form, e.g. `xn--e1adigm`. This module decodes that form to the
 * human-readable Unicode label for RENDERING ONLY.
 *
 * HARD CONSTRAINT: nothing here may be used to produce a value sent to a
 * backend command, mutation, or query key. Every such value must stay the
 * raw `xn--…` string the chain actually knows about. Callers must only ever
 * feed the *output* of `displayName()` into JSX text content.
 *
 * `punycodeDecode` is a small, dependency-free RFC 3492 bootstring decoder
 * (decode direction only — we never need to encode). It never throws: any
 * malformed input (bad digits, overflow, non-ASCII, truncated sequences)
 * yields `null`, and `displayName` treats that as "leave this label as-is".
 */

const BASE = 36;
const T_MIN = 1;
const T_MAX = 26;
const SKEW = 38;
const DAMP = 700;
const INITIAL_BIAS = 72;
const INITIAL_N = 128;
// RFC 3492 uses a 32-bit unsigned overflow bound; using this (rather than
// Number.MAX_SAFE_INTEGER) matches the reference algorithm and still comfortably
// bounds anything a real domain label could produce.
const MAX_INT = 0x7fffffff;

/** Bias adaptation function from RFC 3492 §6.1. */
function adapt(delta: number, numPoints: number, firstTime: boolean): number {
  let d = firstTime ? Math.floor(delta / DAMP) : delta >> 1;
  d += Math.floor(d / numPoints);
  let k = 0;
  while (d > ((BASE - T_MIN) * T_MAX) >> 1) {
    d = Math.floor(d / (BASE - T_MIN));
    k += BASE;
  }
  return k + Math.floor(((BASE - T_MIN + 1) * d) / (d + SKEW));
}

/** Maps an ASCII code point to its base-36 digit value, or BASE if invalid. */
function basicToDigit(codePoint: number): number {
  if (codePoint >= 0x30 && codePoint <= 0x39) return codePoint - 0x16; // '0'-'9' -> 26-35
  if (codePoint >= 0x41 && codePoint <= 0x5a) return codePoint - 0x41; // 'A'-'Z' -> 0-25
  if (codePoint >= 0x61 && codePoint <= 0x7a) return codePoint - 0x61; // 'a'-'z' -> 0-25
  return BASE;
}

/**
 * Decodes the punycode BODY of a label (i.e. everything after the `xn--`
 * ACE prefix has already been stripped by the caller) into a Unicode
 * string. Returns `null` on any malformed input instead of throwing.
 */
export function punycodeDecode(input: string): string | null {
  try {
    for (let idx = 0; idx < input.length; idx++) {
      if (input.charCodeAt(idx) > 0x7f) return null; // punycode input must be pure ASCII
    }

    const output: number[] = [];

    // Everything before the last '-' is copied verbatim as basic code
    // points; the '-' itself is a delimiter, not part of either half.
    let basicEnd = input.lastIndexOf("-");
    if (basicEnd < 0) basicEnd = 0;
    for (let j = 0; j < basicEnd; j++) {
      output.push(input.charCodeAt(j));
    }

    let n = INITIAL_N;
    let i = 0;
    let bias = INITIAL_BIAS;
    const inputLength = input.length;

    for (let index = basicEnd > 0 ? basicEnd + 1 : 0; index < inputLength; ) {
      const oldI = i;
      let w = 1;
      for (let k = BASE; ; k += BASE) {
        if (index >= inputLength) return null; // truncated digit sequence
        const digit = basicToDigit(input.charCodeAt(index++));
        if (digit >= BASE) return null; // not a valid punycode digit
        if (digit > Math.floor((MAX_INT - i) / w)) return null; // overflow
        i += digit * w;
        const t = k <= bias ? T_MIN : k >= bias + T_MAX ? T_MAX : k - bias;
        if (digit < t) break;
        const baseMinusT = BASE - t;
        if (w > Math.floor(MAX_INT / baseMinusT)) return null; // overflow
        w *= baseMinusT;
      }

      const outLen = output.length + 1;
      bias = adapt(i - oldI, outLen, oldI === 0);

      if (Math.floor(i / outLen) > MAX_INT - n) return null; // overflow
      n += Math.floor(i / outLen);
      i %= outLen;

      // Reject surrogate halves and anything outside the valid Unicode range —
      // a well-formed encoder never produces these, so seeing one means the
      // input is corrupt.
      if (n < 0 || n > 0x10ffff || (n >= 0xd800 && n <= 0xdfff)) return null;

      output.splice(i, 0, n);
      i++;
    }

    return output.map((cp) => String.fromCodePoint(cp)).join("");
  } catch {
    return null;
  }
}

/**
 * Decodes a (possibly dotted) name for DISPLAY. Each label starting with
 * the `xn--` ACE prefix (case-insensitive) is decoded independently; a
 * label that fails to decode is left exactly as-is. Non-punycode labels are
 * never touched. Never throws.
 *
 * This is display-only — never feed the result into a backend command,
 * mutation, or query key. Always pass the original raw name for those.
 */
export function displayName(name: string): string {
  if (!name.toLowerCase().includes("xn--")) return name; // fast path: nothing to decode
  return name
    .split(".")
    .map((label) => {
      if (!label.toLowerCase().startsWith("xn--")) return label;
      const decoded = punycodeDecode(label.slice(4));
      return decoded === null ? label : decoded;
    })
    .join(".");
}
