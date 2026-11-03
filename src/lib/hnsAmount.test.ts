import { describe, it, expect } from "vitest";
import {
  doosToHnsDigits,
  hnsDigitsToDoos,
  HNS_AMOUNT_ABOVE_SUPPLY,
  HNS_AMOUNT_DECIMALS,
  HNS_AMOUNT_NOT_POSITIVE,
  MAX_MONEY_DOOS,
  parseHnsAmount,
} from "./hnsAmount";

describe("parseHnsAmount — an HNS amount typed into a money field", () => {
  it("reads 1234.567891 HNS as exactly 1,234,567,891 doos", () => {
    expect(parseHnsAmount("1234.567891")).toEqual({ doos: 1_234_567_891 });
  });

  it("reads trailing zeros and whole amounts exactly", () => {
    expect(parseHnsAmount("2043.660000")).toEqual({ doos: 2_043_660_000 });
    expect(parseHnsAmount("1")).toEqual({ doos: 1_000_000 });
    expect(parseHnsAmount("0.000001")).toEqual({ doos: 1 });
    expect(parseHnsAmount("007.5")).toEqual({ doos: 7_500_000 });
  });

  it("reads the digits without a float in between", () => {
    // parseFloat("279126647.329015") * 1e6 is 279126647329015.03, not whole doos.
    expect(parseHnsAmount("279126647.329015")).toEqual({ doos: 279_126_647_329_015 });
    expect(parseHnsAmount("123.456789")).toEqual({ doos: 123_456_789 });
  });

  it("accepts a point with digits on one side only", () => {
    expect(parseHnsAmount(".5")).toEqual({ doos: 500_000 });
    expect(parseHnsAmount("5.")).toEqual({ doos: 5_000_000 });
  });

  it("ignores spaces around the amount", () => {
    expect(parseHnsAmount("  12.5 ")).toEqual({ doos: 12_500_000 });
  });

  it.each(["10.0000005", "1.23456789", "0.0000004", "1234.5678912"])(
    "refuses a 7th decimal instead of rounding it (%s)",
    (text) => {
      expect(parseHnsAmount(text)).toEqual({ error: HNS_AMOUNT_DECIMALS });
    },
  );

  it.each(["0", "0.0", "0.000000", ".0"])("refuses zero (%s)", (text) => {
    expect(parseHnsAmount(text)).toEqual({ error: HNS_AMOUNT_NOT_POSITIVE });
  });

  it.each([
    "",
    "   ",
    ".",
    "abc",
    "1.5abc",
    "0x10",
    "0b11",
    "1e3",
    "1e-7",
    "-5",
    "+5",
    "Infinity",
    "NaN",
    "1,000",
    "1.2.3",
    "1 000",
    "١٢",
  ])("refuses what is not plain digits and one point (%j)", (text) => {
    expect(parseHnsAmount(text)).toEqual({ error: HNS_AMOUNT_NOT_POSITIVE });
  });

  it("accepts the whole money supply and converts it exactly", () => {
    const read = parseHnsAmount("2040000000");
    expect(read).toEqual({ doos: 2_040_000_000_000_000 });
    expect("doos" in read && Number.isSafeInteger(read.doos)).toBe(true);
  });

  it("refuses one doo above the money supply", () => {
    expect(parseHnsAmount("2040000000.000001")).toEqual({ error: HNS_AMOUNT_ABOVE_SUPPLY });
  });

  it("refuses an amount whose doos are past Number.MAX_SAFE_INTEGER", () => {
    // 9007199254.740993 HNS = 2^53 + 1 doos, which a JS number cannot hold.
    expect(parseHnsAmount("9007199254.740993")).toEqual({ error: HNS_AMOUNT_ABOVE_SUPPLY });
    expect(parseHnsAmount("99999999999999999999")).toEqual({ error: HNS_AMOUNT_ABOVE_SUPPLY });
  });
});

describe("hnsDigitsToDoos — the digits core shared with the Market price", () => {
  it("returns a bigint and never rounds", () => {
    expect(hnsDigitsToDoos("1234.567891")).toEqual({ doos: 1_234_567_891n });
    expect(hnsDigitsToDoos("1.0000001")).toEqual({ error: "HNS has at most 6 decimals" });
    expect(hnsDigitsToDoos("1e3")).toEqual({ error: "not a number of HNS" });
  });

  it("needs a digit on at least one side of the point, as `sell.rs::parse_step_price` does", () => {
    expect(hnsDigitsToDoos(".")).toEqual({ error: "not a number of HNS" });
    expect(hnsDigitsToDoos("")).toEqual({ error: "not a number of HNS" });
  });

  it("leaves 0 and the money supply to its callers", () => {
    expect(hnsDigitsToDoos("0")).toEqual({ doos: 0n });
    expect(hnsDigitsToDoos("3000000000")).toEqual({ doos: 3_000_000_000_000_000n });
    expect(MAX_MONEY_DOOS).toBe(2_040_000_000_000_000n);
  });
});

describe("doosToHnsDigits — doos as plain digits for a money field", () => {
  it("writes no thousands separators and no trailing zeros", () => {
    expect(doosToHnsDigits(2_043_660_000)).toBe("2043.66");
    expect(doosToHnsDigits(3_000_000)).toBe("3");
    expect(doosToHnsDigits(1)).toBe("0.000001");
    expect(doosToHnsDigits(0)).toBe("0");
  });

  it.each([1, 999_999, 1_000_000, 1_234_567_891, 2_043_660_000, 2_040_000_000_000_000])(
    "reads back to the same doos (%i)",
    (doos) => {
      expect(parseHnsAmount(doosToHnsDigits(doos))).toEqual({ doos });
    },
  );

  it.each([-1, 0.5, Number.NaN, 2 ** 53])("throws on what is not whole doos (%d)", (doos) => {
    expect(() => doosToHnsDigits(doos)).toThrow(RangeError);
  });
});
