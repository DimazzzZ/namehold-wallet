import { describe, expect, it } from "vitest";
import {
  DEFAULT_FEE_RATE_DOOS_PER_KVB,
  MIN_FEE_RATE_DOOS_PER_KVB,
  doosPerKvbToSatsPerByte,
  formatDoosPerKvb,
  parseDoosPerKvb,
  parseFeeRateArg,
} from "./feeRate";

describe("parseDoosPerKvb", () => {
  it("returns null for empty / whitespace", () => {
    expect(parseDoosPerKvb("")).toBeNull();
    expect(parseDoosPerKvb("   ")).toBeNull();
  });

  it("returns null for non-integer / non-finite / non-positive input", () => {
    expect(parseDoosPerKvb("abc")).toBeNull();
    expect(parseDoosPerKvb("1.5")).toBeNull();
    expect(parseDoosPerKvb("-100")).toBeNull();
    expect(parseDoosPerKvb("0")).toBeNull();
  });

  it("floors small values to the minimum", () => {
    expect(parseDoosPerKvb("1")).toBe(MIN_FEE_RATE_DOOS_PER_KVB);
    expect(parseDoosPerKvb("500")).toBe(MIN_FEE_RATE_DOOS_PER_KVB);
    expect(parseDoosPerKvb("999")).toBe(MIN_FEE_RATE_DOOS_PER_KVB);
  });

  it("passes through values at or above the minimum", () => {
    expect(parseDoosPerKvb("1000")).toBe(1000);
    expect(parseDoosPerKvb("5000")).toBe(5000);
    expect(parseDoosPerKvb("42000")).toBe(42000);
  });
});

describe("doosPerKvbToSatsPerByte", () => {
  it("returns null when the input is null (fall-through case)", () => {
    expect(doosPerKvbToSatsPerByte(null)).toBeNull();
  });

  it("divides by 1000, flooring at 1 sat/byte", () => {
    expect(doosPerKvbToSatsPerByte(1000)).toBe(1);
    expect(doosPerKvbToSatsPerByte(1999)).toBe(1);
    expect(doosPerKvbToSatsPerByte(2000)).toBe(2);
    expect(doosPerKvbToSatsPerByte(42000)).toBe(42);
  });

  it("floors sub-1000 values at 1 sat/byte (mirrors backend)", () => {
    // parseDoosPerKvb already prevents this, but be defensive at the seam.
    expect(doosPerKvbToSatsPerByte(500)).toBe(1);
  });
});

describe("formatDoosPerKvb", () => {
  it("adds thousand separators via toLocaleString", () => {
    // Guard against locale drift by matching the digits only.
    expect(formatDoosPerKvb(1000).replace(/\D/g, "")).toBe("1000");
    expect(formatDoosPerKvb(42000).replace(/\D/g, "")).toBe("42000");
  });
});

describe("defaults match the backend", () => {
  it("DEFAULT_FEE_RATE_DOOS_PER_KVB converts to Rust's DEFAULT_FEE_RATE_PER_BYTE (1)", () => {
    expect(doosPerKvbToSatsPerByte(DEFAULT_FEE_RATE_DOOS_PER_KVB)).toBe(1);
  });
});

describe("parseFeeRateArg", () => {
  it("returns null for empty input (no override)", () => {
    expect(parseFeeRateArg("")).toBeNull();
    expect(parseFeeRateArg("  ")).toBeNull();
  });

  it("parses and converts valid doos/kvB to sats/byte in one step", () => {
    expect(parseFeeRateArg("1000")).toBe(1);
    expect(parseFeeRateArg("2000")).toBe(2);
    expect(parseFeeRateArg("42000")).toBe(42);
  });

  it("floors sub-1000 values to 1 sat/byte", () => {
    expect(parseFeeRateArg("500")).toBe(1);
    expect(parseFeeRateArg("999")).toBe(1);
  });

  it("returns null for invalid input", () => {
    expect(parseFeeRateArg("abc")).toBeNull();
    expect(parseFeeRateArg("1.5")).toBeNull();
    expect(parseFeeRateArg("-100")).toBeNull();
  });
});
