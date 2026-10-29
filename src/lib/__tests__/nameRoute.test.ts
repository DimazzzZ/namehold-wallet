import { describe, it, expect } from "vitest";
import { namePath, parseNameTab } from "../nameRoute";

describe("namePath", () => {
  it("encodes the raw name and leaves overview without ?tab", () => {
    expect(namePath("wallet-guy")).toBe("/names/wallet-guy");
    expect(namePath("wallet-guy", "overview")).toBe("/names/wallet-guy");
    expect(namePath("a b")).toBe("/names/a%20b");
  });

  it("adds ?tab for every other tab", () => {
    expect(namePath("xn--mnchen-3ya", "dns")).toBe("/names/xn--mnchen-3ya?tab=dns");
    expect(namePath("n", "transfer")).toBe("/names/n?tab=transfer");
    expect(namePath("n", "sign")).toBe("/names/n?tab=sign");
  });
});

describe("parseNameTab", () => {
  it("keeps a known tab", () => {
    expect(parseNameTab("dns")).toBe("dns");
    expect(parseNameTab("sign")).toBe("sign");
  });

  it("falls back to overview", () => {
    expect(parseNameTab(null)).toBe("overview");
    expect(parseNameTab("nope")).toBe("overview");
    expect(parseNameTab("")).toBe("overview");
  });
});
