import { describe, it, expect } from "vitest";
import {
  parseStringArraySetting,
  serializeStringArraySetting,
  normalizeTransaction,
  totalBalanceDoos,
} from "./providerMode";

describe("parseStringArraySetting", () => {
  it("parses a JSON array, trimming and dropping empties", () => {
    expect(parseStringArraySetting('["a"," b ",""]')).toEqual(["a", "b"]);
  });
  it("tolerates null / malformed", () => {
    expect(parseStringArraySetting(null)).toEqual([]);
    expect(parseStringArraySetting("not json")).toEqual([]);
    expect(parseStringArraySetting("{}")).toEqual([]);
  });
});

describe("serializeStringArraySetting", () => {
  it("round-trips through parse", () => {
    const raw = serializeStringArraySetting([" x ", "y", ""]);
    expect(parseStringArraySetting(raw)).toEqual(["x", "y"]);
  });
});

describe("totalBalanceDoos", () => {
  it("sums confirmed + unconfirmed; null -> 0", () => {
    expect(totalBalanceDoos(null)).toBe(0);
    expect(totalBalanceDoos(undefined)).toBe(0);
    expect(
      totalBalanceDoos({
        confirmed: 100,
        unconfirmed: 25,
        locked_confirmed: null,
        locked_unconfirmed: null,
      }),
    ).toBe(125);
  });
});

describe("normalizeTransaction", () => {
  it("classifies a flat explorer-style receive", () => {
    const row = normalizeTransaction(
      { hash: "tx1", value: 500000, direction: "receive", address: "hs1qx", confirmed: true, height: 10 },
      0,
    );
    expect(row.hash).toBe("tx1");
    expect(row.direction).toBe("receive");
    expect(row.amountDoos).toBe(500000);
    expect(row.confirmed).toBe(true);
  });

  it("falls back to a synthetic id", () => {
    const row = normalizeTransaction({}, 3);
    expect(row.hash).toBe("tx-3");
  });
});
