import { describe, it, expect } from "vitest";
import { dollarydoosToHns, hnsToDollarydoos, formatHns, cn, formatDate, truncate } from "./utils";

describe("dollarydoosToHns", () => {
  it("converts 0", () => {
    expect(dollarydoosToHns(0)).toBe("0.000000");
  });
  it("converts 1 HNS", () => {
    expect(dollarydoosToHns(1_000_000)).toBe("1.000000");
  });
  it("converts fractional", () => {
    expect(dollarydoosToHns(500_000)).toBe("0.500000");
  });
  it("converts large amount", () => {
    expect(dollarydoosToHns(1_234_567_890)).toBe("1234.567890");
  });
});

describe("hnsToDollarydoos", () => {
  it("converts 0", () => {
    expect(hnsToDollarydoos("0")).toBe(0);
  });
  it("converts 1 HNS", () => {
    expect(hnsToDollarydoos("1")).toBe(1_000_000);
  });
  it("converts fractional", () => {
    expect(hnsToDollarydoos("0.5")).toBe(500_000);
  });
  it("rounds correctly", () => {
    expect(hnsToDollarydoos("1.23456789")).toBe(1_234_568);
  });
});

describe("formatHns", () => {
  it("returns dash for null", () => {
    expect(formatHns(null)).toBe("—");
  });
  it("returns dash for undefined", () => {
    expect(formatHns(undefined)).toBe("—");
  });
  it("formats value", () => {
    expect(formatHns(1_000_000)).toBe("1.000000");
  });
});

describe("cn", () => {
  it("joins classes", () => {
    expect(cn("a", "b", "c")).toBe("a b c");
  });
  it("filters falsy", () => {
    expect(cn("a", false, null, undefined, "b")).toBe("a b");
  });
  it("returns empty for no args", () => {
    expect(cn()).toBe("");
  });
});

describe("formatDate", () => {
  it("returns dash for null", () => {
    expect(formatDate(null)).toBe("—");
  });
  it("returns dash for empty", () => {
    expect(formatDate("")).toBe("—");
  });
  it("formats ISO date", () => {
    const result = formatDate("2024-01-15T10:30:00");
    expect(result).toContain("2024");
  });
});

describe("truncate", () => {
  it("returns short strings unchanged", () => {
    expect(truncate("hello", 10)).toBe("hello");
  });
  it("truncates long strings", () => {
    expect(truncate("hello world", 5)).toBe("hello...");
  });
  it("exact length unchanged", () => {
    expect(truncate("hello", 5)).toBe("hello");
  });
});
