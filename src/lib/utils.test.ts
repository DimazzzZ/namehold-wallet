import { describe, it, expect } from "vitest";
import {
  dollarydoosToHns,
  hnsToDollarydoos,
  formatHns,
  formatCount,
  formatHnsAmount,
  cn,
  formatDate,
  latestTimestamp,
  truncate,
  netSpendDoos,
  truncateMiddle,
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

  it("converts large amount with thousand separators", () => {
    expect(dollarydoosToHns(1_234_567_890)).toBe("1,234.567890");
  });

  it("adds thousand separators for millions", () => {
    expect(dollarydoosToHns(1_000_000_000_000)).toBe("1,000,000.000000");
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

  it("formats large value with thousand separators", () => {
    expect(formatHns(123_456_789_000_000)).toBe("123,456,789.000000");
  });
});

describe("formatCount", () => {
  it("formats small number without separators", () => {
    expect(formatCount(42)).toBe("42");
  });

  it("adds thousand separators", () => {
    expect(formatCount(120002)).toBe("120,002");
  });

  it("formats millions", () => {
    expect(formatCount(1000000)).toBe("1,000,000");
  });

  it("handles zero", () => {
    expect(formatCount(0)).toBe("0");
  });
});

describe("formatHnsAmount", () => {
  it("formats integer with thousand separators and 6 decimal places", () => {
    expect(formatHnsAmount(120002)).toBe("120,002.000000");
  });

  it("formats decimal with thousand separators and 6 decimal places", () => {
    expect(formatHnsAmount(120002.4)).toBe("120,002.400000");
  });

  it("formats large decimal with 6 decimal places", () => {
    expect(formatHnsAmount(120002.400000)).toBe("120,002.400000");
  });

  it("formats small number with 6 decimal places", () => {
    expect(formatHnsAmount(0.5)).toBe("0.500000");
  });

  it("formats zero with 6 decimal places", () => {
    expect(formatHnsAmount(0)).toBe("0.000000");
  });

  it("formats millions with 6 decimal places", () => {
    expect(formatHnsAmount(1000000)).toBe("1,000,000.000000");
  });

  it("formats string input with 6 decimal places", () => {
    expect(formatHnsAmount("120002.4")).toBe("120,002.400000");
  });

  it("normalizes trailing zeros from string input to 6 decimal places", () => {
    expect(formatHnsAmount("1000.50")).toBe("1,000.500000");
  });

  it("returns raw string for non-finite values", () => {
    expect(formatHnsAmount("not-a-number")).toBe("not-a-number");
  });

  it("does NOT format IDs (IDs should not use this function)", () => {
    // IDs are never passed to formatHnsAmount — this test documents the contract
    const id = 123456789;
    // IDs stay raw as numbers; formatHnsAmount is only for HNS amounts
    expect(formatHnsAmount(id)).toBe("123,456,789.000000");
  });

  it("always uses exactly 6 decimal places for consistency with formatHns", () => {
    expect(formatHnsAmount(1)).toBe("1.000000");
    expect(formatHnsAmount(0.123456)).toBe("0.123456");
    expect(formatHnsAmount(100000.123456)).toBe("100,000.123456");
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

  it("parses a Namebase ISO timestamp that already carries Z (no double-Z → never Invalid Date)", () => {
    const result = formatDate("2026-06-26T00:00:00Z");
    expect(result).not.toMatch(/Invalid Date/);
    expect(result).toContain("2026");
  });

  it("parses an ISO timestamp with millis + Z", () => {
    const result = formatDate("2026-06-26T13:23:32.000Z");
    expect(result).not.toMatch(/Invalid Date/);
    expect(result).toContain("2026");
  });

  it("parses an ISO timestamp with a numeric offset", () => {
    const result = formatDate("2026-06-26T13:23:32+02:00");
    expect(result).not.toMatch(/Invalid Date/);
    expect(result).toContain("2026");
  });

  it("parses a SQLite naive 'YYYY-MM-DD HH:MM:SS' timestamp as UTC", () => {
    const result = formatDate("2026-06-26 00:00:00");
    expect(result).not.toMatch(/Invalid Date/);
    expect(result).toContain("2026");
  });

  it("returns the raw string for an unparseable value (never 'Invalid Date')", () => {
    expect(formatDate("not-a-date")).toBe("not-a-date");
  });
});

describe("latestTimestamp (Task 11 review, Finding 2)", () => {
  it("returns null when both are null", () => {
    expect(latestTimestamp(null, null)).toBeNull();
  });

  it("returns the non-null one when only one is set", () => {
    expect(latestTimestamp("2026-07-10 12:00:00", null)).toBe("2026-07-10 12:00:00");
    expect(latestTimestamp(null, "2026-07-10 12:00:00")).toBe("2026-07-10 12:00:00");
  });

  it("returns the newer of two SQLite naive-UTC timestamps", () => {
    const older = "2026-01-01 00:00:00";
    const newer = "2026-07-14 09:00:00";
    expect(latestTimestamp(older, newer)).toBe(newer);
    expect(latestTimestamp(newer, older)).toBe(newer);
  });

  it("compares across mixed formats (SQLite naive vs. ISO with Z)", () => {
    const older = "2026-01-01 00:00:00";
    const newer = "2026-07-14T09:00:00Z";
    expect(latestTimestamp(older, newer)).toBe(newer);
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

describe("netSpendDoos", () => {
  it("returns 0 for a covenant name-action (value carried to your own coin)", () => {
    // e.g. a DNS UPDATE: sendTotalDoos is the name's locked value (222 HNS),
    // re-homed to your own new coin — nothing leaves the wallet beyond the fee.
    expect(
      netSpendDoos({ sendTotalDoos: 222_000_000, recipientAddress: null }),
    ).toBe(0);
  });

  it("returns the full amount for a real transfer to a recipient (send/transfer/finalize)", () => {
    expect(
      netSpendDoos({ sendTotalDoos: 1_000_000, recipientAddress: "hs1qexample" }),
    ).toBe(1_000_000);
  });
});

describe("truncateMiddle", () => {
  it("returns short strings unchanged (no ellipsis when it wouldn't save space)", () => {
    expect(truncateMiddle("short")).toBe("short");
    expect(truncateMiddle("", 8, 6)).toBe("");
    // Exactly at the boundary — head+tail+1 chars long, no truncation.
    expect(truncateMiddle("abcdefghijklmno", 8, 6)).toBe("abcdefghijklmno");
  });

  it("keeps `head` and `tail` chars around a middle ellipsis for long strings", () => {
    const xpub =
      "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
    const out = truncateMiddle(xpub);
    expect(out.startsWith("xpub6CUG")).toBe(true);
    expect(out.endsWith("Au3fDVmz".slice(-6))).toBe(true);
    expect(out).toContain("…");
    expect(out.length).toBeLessThan(xpub.length);
  });

  it("honors custom head/tail lengths", () => {
    expect(truncateMiddle("abcdefghijklmnop", 3, 3)).toBe("abc…nop");
  });
});
