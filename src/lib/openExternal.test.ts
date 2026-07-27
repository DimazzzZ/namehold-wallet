import { describe, expect, it } from "vitest";

import {
  SHAKESHIFT_BASE,
  EXPLORER_TX_BASE,
  explorerAddressUrl,
  explorerBlockUrl,
  explorerNameUrl,
  explorerTxUrl,
} from "./openExternal";

// The URL builders are the single source of truth for every user-facing
// explorer link; regressions here mean broken "View on explorer" affordances.
describe("openExternal URL builders", () => {
  it("SHAKESHIFT_BASE points at the canonical explorer host", () => {
    expect(SHAKESHIFT_BASE).toBe("https://shakeshift.com");
  });

  it("keeps the legacy EXPLORER_TX_BASE alias in sync with SHAKESHIFT_BASE", () => {
    expect(EXPLORER_TX_BASE).toBe(`${SHAKESHIFT_BASE}/transaction`);
  });

  it("explorerTxUrl builds /transaction/<txid>", () => {
    const txid =
      "3c91c37f649146dd159357f955f464d4d94a6d44a75e0c6f506d7a527af8ec38";
    expect(explorerTxUrl(txid)).toBe(
      `https://shakeshift.com/transaction/${txid}`,
    );
  });

  it("explorerNameUrl builds /name/<name> for ASCII", () => {
    expect(explorerNameUrl("namehold")).toBe(
      "https://shakeshift.com/name/namehold",
    );
  });

  it("explorerNameUrl percent-encodes non-ASCII (emoji / unicode) names", () => {
    // Shakeshift accepts both raw and percent-encoded forms; we pick the
    // safe percent-encoded form so the URL is transport-clean everywhere.
    const url = explorerNameUrl("🐨");
    expect(url.startsWith("https://shakeshift.com/name/")).toBe(true);
    expect(url).not.toContain("🐨");
    // Round-trip check.
    const encoded = url.slice("https://shakeshift.com/name/".length);
    expect(decodeURIComponent(encoded)).toBe("🐨");
  });

  it("explorerNameUrl leaves an already-punycode name untouched", () => {
    // `xn--wo8h` = 🐨 in IDNA punycode. Only `-` in the ASCII-safe set, so
    // encodeURIComponent is a no-op.
    expect(explorerNameUrl("xn--wo8h")).toBe(
      "https://shakeshift.com/name/xn--wo8h",
    );
  });

  it("explorerAddressUrl builds /address/<hs1…>", () => {
    const addr = "hs1q7p94h09nqshcjuc5hpq06pz7mf40gpmjg6k6yk";
    expect(explorerAddressUrl(addr)).toBe(
      `https://shakeshift.com/address/${addr}`,
    );
  });

  it("explorerBlockUrl builds /block/<height>", () => {
    expect(explorerBlockUrl(340052)).toBe(
      "https://shakeshift.com/block/340052",
    );
  });
});
