import { describe, it, expect } from "vitest";
import { toAceName } from "../idnEncode";
import { punycodeDecode } from "../idn";
import { normalizeNameInputAce } from "../utils";

// Reference ACE values verified via Node:
//   node -e "console.log(require('tr46').toASCII(<input>, {useSTD3ASCIIRules:false, verifyDNSLength:false}))"
// These also round-trip through the existing dependency-free RFC 3492 decoder
// in idn.ts (see idn.test.ts for the decode direction).

describe("toAceName", () => {
  it("encodes a Cyrillic label (козел → xn--e1adigm)", () => {
    expect(toAceName("козел")).toBe("xn--e1adigm");
  });

  it("encodes a German umlaut label (münchen → xn--mnchen-3ya)", () => {
    expect(toAceName("münchen")).toBe("xn--mnchen-3ya");
  });

  it("encodes a CJK label (中文 → xn--fiq228c)", () => {
    expect(toAceName("中文")).toBe("xn--fiq228c");
  });

  it("encodes another Cyrillic label (сбер → xn--90ai7ab)", () => {
    expect(toAceName("сбер")).toBe("xn--90ai7ab");
  });

  it("passes ASCII input through unchanged", () => {
    expect(toAceName("example")).toBe("example");
  });

  it("is idempotent on already-encoded ACE names", () => {
    expect(toAceName("xn--e1adigm")).toBe("xn--e1adigm");
    expect(toAceName("xn--90ai7ab")).toBe("xn--90ai7ab");
  });

  it("returns empty string for empty / whitespace input", () => {
    expect(toAceName("")).toBe("");
    expect(toAceName("   ")).toBe("");
  });

  it("lowercases mixed-case input", () => {
    expect(toAceName("Example")).toBe("example");
    expect(toAceName("КОЗЕЛ")).toBe("xn--e1adigm");
  });

  it("round-trips through the existing punycodeDecode", () => {
    for (const original of ["козел", "münchen", "中文", "сбер"]) {
      const ace = toAceName(original);
      expect(ace.startsWith("xn--")).toBe(true);
      const decoded = punycodeDecode(ace.slice("xn--".length));
      expect(decoded).toBe(original);
    }
  });
});

describe("normalizeNameInputAce", () => {
  it("encodes a Cyrillic input to ACE form", () => {
    expect(normalizeNameInputAce("сбер")).toBe("xn--90ai7ab");
  });

  it("strips a leading dot before encoding", () => {
    expect(normalizeNameInputAce(".сбер")).toBe("xn--90ai7ab");
  });

  it("strips a trailing .hsrd suffix before encoding", () => {
    expect(normalizeNameInputAce("сбер.hsrd")).toBe("xn--90ai7ab");
  });

  it("collapses repeated dots and trims", () => {
    // Trailing whitespace/dots stripped; .hsrd suffix removed.
    expect(normalizeNameInputAce("  сбер.hsrd  ")).toBe("xn--90ai7ab");
  });

  it("passes ASCII input through (unchanged, lowercased)", () => {
    expect(normalizeNameInputAce("Example")).toBe("example");
    expect(normalizeNameInputAce("example.hsrd")).toBe("example");
  });

  it("returns empty string for blank input", () => {
    expect(normalizeNameInputAce("")).toBe("");
    expect(normalizeNameInputAce("   ")).toBe("");
    expect(normalizeNameInputAce(".")).toBe("");
  });

  it("does NOT strip non-ASCII characters (the regression this fix prevents)", () => {
    // The old normalizeNameInput would return "" here (Cyrillic stripped).
    // The new function must produce the ACE form instead.
    expect(normalizeNameInputAce("козел")).not.toBe("");
    expect(normalizeNameInputAce("козел")).toBe("xn--e1adigm");
  });
});
