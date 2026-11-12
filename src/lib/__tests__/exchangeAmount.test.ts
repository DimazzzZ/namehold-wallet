import { describe, it, expect } from "vitest";
import {
  PRICE_DECIMALS,
  PRICE_NOT_POSITIVE,
  SATS_ABOVE_SUPPLY,
  SATS_NOT_POSITIVE,
  btcSatsFor,
  btcText,
  hnsText,
  msatToPriceDigits,
  offerHnsDoos,
  parsePriceMsat,
  parseSatsAmount,
  priceText,
  satsText,
} from "../exchangeAmount";

const U64_MAX = 18_446_744_073_709_551_615;

describe("parsePriceMsat", () => {
  it.each([
    ["250", 250_000n],
    ["250.5", 250_500n],
    ["0.001", 1n],
    [".5", 500n],
    ["7.", 7000n],
    [" 12.345 ", 12_345n],
    ["18446744073709551615.999", 18_446_744_073_709_551_615_999n],
  ])("reads %j exactly", (text, msat) => {
    expect(parsePriceMsat(text)).toEqual({ msat });
  });

  it.each(["", "0", "0.000", "-1", "1e3", "1,000", "1.2.3", "abc", "."])("refuses %j", (text) => {
    expect(parsePriceMsat(text)).toEqual({ error: PRICE_NOT_POSITIVE });
  });

  it("refuses a 4th decimal, never rounding", () => {
    expect(parsePriceMsat("1.0001")).toEqual({ error: PRICE_DECIMALS });
  });

  it("round-trips through the price digits", () => {
    for (const t of ["250", "250.5", "0.001", "1000000.123"]) {
      const read = parsePriceMsat(t);
      expect("msat" in read && msatToPriceDigits(read.msat)).toBe(t);
    }
  });
});

describe("btcSatsFor", () => {
  it("is floor(doos × msat / 1e9)", () => {
    // 1000 HNS at 250 sats/HNS = 250,000 sats.
    expect(btcSatsFor(1_000_000_000n, 250_000n)).toBe(250_000n);
    // 1.5 HNS at 0.001 sats/HNS = 0.0015 sats → 0.
    expect(btcSatsFor(1_500_000n, 1n)).toBe(0n);
    // 0.333333 HNS at 3 sats/HNS = 0.999999 sats → 0; 0.333334 → 1.000002 → 1.
    expect(btcSatsFor(333_333n, 3000n)).toBe(0n);
    expect(btcSatsFor(333_334n, 3000n)).toBe(1n);
  });

  it("computes at u64-sized inputs without losing a digit", () => {
    const doos = 18_446_744_073_709_551_615n;
    const msat = 18_446_744_073_709_551_615_999n;
    expect(btcSatsFor(doos, msat)).toBe((doos * msat) / 1_000_000_000n);
  });
});

describe("display helpers", () => {
  it("formats sats and BTC from numbers and BigInts", () => {
    expect(satsText(1_250_000)).toBe("1,250,000 sats");
    expect(satsText(1)).toBe("1 sat");
    expect(btcText(1_250_000)).toBe("0.01250000 BTC");
    expect(btcText(2_100_000_000_000_000n)).toBe("21,000,000.00000000 BTC");
    expect(satsText(123_456_789_012_345_678_901n)).toBe("123,456,789,012,345,678,901 sats");
  });

  it("returns null for a value that cannot be shown exactly (u64 safety)", () => {
    expect(satsText(U64_MAX)).toBeNull();
    expect(btcText(U64_MAX)).toBeNull();
    expect(satsText(-1)).toBeNull();
    expect(satsText(1.5)).toBeNull();
    expect(btcText(Number.NaN)).toBeNull();
    expect(satsText(-1n)).toBeNull();
  });

  it("formats an offer's price and HNS, or null for text that is not one", () => {
    expect(priceText("250.5")).toBe("250.5 sats/HNS");
    expect(priceText("1234567")).toBe("1,234,567 sats/HNS");
    expect(priceText("abc")).toBeNull();
    expect(hnsText("1250.500000")).toBe("1,250.5 HNS");
    expect(hnsText("1.2345678")).toBeNull();
    expect(offerHnsDoos("18446744073709.551615")).toBe(18_446_744_073_709_551_615n);
  });
});

describe("parseSatsAmount", () => {
  it("reads whole sats", () => {
    expect(parseSatsAmount(" 25000 ")).toEqual({ sats: 25_000 });
    expect(parseSatsAmount("2100000000000000")).toEqual({ sats: 2_100_000_000_000_000 });
  });

  it.each(["", "0", "1.5", "-3", "1e5", "abc"])("refuses %j", (t) => {
    expect(parseSatsAmount(t)).toEqual({ error: SATS_NOT_POSITIVE });
  });

  it("refuses more than 21 million BTC", () => {
    expect(parseSatsAmount("2100000000000001")).toEqual({ error: SATS_ABOVE_SUPPLY });
    expect(parseSatsAmount("18446744073709551615")).toEqual({ error: SATS_ABOVE_SUPPLY });
  });
});
