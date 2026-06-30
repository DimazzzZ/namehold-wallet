import { describe, it, expect } from "vitest";
import {
  dollarydoosToHns,
  hnsToDollarydoos,
  formatHns,
  cn,
  formatDate,
  truncate,
} from "./utils";

describe("dollarydoosToHns", () => {
  it("converts zero", () => {
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

  it("converts small amount", () => {
    expect(dollarydoosToHns(1)).toBe("0.000001");
  });
});

describe("hnsToDollarydoos", () => {
  it("converts zero", () => {
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

  it("handles large numbers", () => {
    expect(hnsToDollarydoos("1000")).toBe(1_000_000_000);
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

  it("formats zero", () => {
    expect(formatHns(0)).toBe("0.000000");
  });
});

describe("cn", () => {
  it("joins classes", () => {
    expect(cn("a", "b", "c")).toBe("a b c");
  });

  it("filters falsy values", () => {
    expect(cn("a", false, null, undefined, "b")).toBe("a b");
  });

  it("returns empty string for no args", () => {
    expect(cn()).toBe("");
  });

  it("filters empty strings", () => {
    expect(cn("a", "", "b")).toBe("a b");
  });
});

describe("formatDate", () => {
  it("returns dash for null", () => {
    expect(formatDate(null)).toBe("—");
  });

  it("returns dash for undefined", () => {
    expect(formatDate(undefined)).toBe("—");
  });

  it("returns dash for empty string", () => {
    expect(formatDate("")).toBe("—");
  });

  it("formats ISO date", () => {
    const result = formatDate("2024-01-15T10:30:00");
    expect(result).toContain("2024");
  });

  it("handles date-only strings", () => {
    const result = formatDate("2024-06-15");
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

  it("single character", () => {
    expect(truncate("a", 1)).toBe("a");
  });

  it("empty string", () => {
    expect(truncate("", 5)).toBe("");
  });
});
