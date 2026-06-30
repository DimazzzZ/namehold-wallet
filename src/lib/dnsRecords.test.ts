import { describe, it, expect } from "vitest";
import { rowToRecord, rowsToRecords } from "./dnsRecords";

describe("rowToRecord", () => {
  it("serializes each supported type to its hsd shape", () => {
    expect(rowToRecord({ type: "A", value: "1.2.3.4" })).toEqual({ type: "A", address: "1.2.3.4" });
    expect(rowToRecord({ type: "AAAA", value: "::1" })).toEqual({ type: "AAAA", address: "::1" });
    expect(rowToRecord({ type: "CNAME", value: "t.example." })).toEqual({
      type: "CNAME",
      target: "t.example.",
    });
    expect(rowToRecord({ type: "NS", value: "ns1.example." })).toEqual({
      type: "NS",
      ns: "ns1.example.",
    });
    expect(rowToRecord({ type: "TXT", value: "hello" })).toEqual({ type: "TXT", txt: ["hello"] });
  });

  it("trims and drops blank values", () => {
    expect(rowToRecord({ type: "TXT", value: "  hi  " })).toEqual({ type: "TXT", txt: ["hi"] });
    expect(rowToRecord({ type: "A", value: "   " })).toBeNull();
  });
});

describe("rowsToRecords", () => {
  it("round-trips a mixed set, skipping blank rows", () => {
    const records = rowsToRecords([
      { type: "TXT", value: "cua-agent-verified" },
      { type: "A", value: "" },
      { type: "CNAME", value: "x.example." },
    ]);
    expect(records).toEqual([
      { type: "TXT", txt: ["cua-agent-verified"] },
      { type: "CNAME", target: "x.example." },
    ]);
  });

  it("returns null when every row is blank (→ EMPTY resource)", () => {
    expect(rowsToRecords([{ type: "TXT", value: "" }])).toBeNull();
    expect(rowsToRecords([])).toBeNull();
  });
});
