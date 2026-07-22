import { describe, it, expect } from "vitest";
import { rowToRecord, rowsToRecords } from "./dnsRecords";

describe("rowToRecord", () => {
  it("serializes TXT to {type,txt:[value]}", () => {
    expect(rowToRecord({ type: "TXT", value: "hello" })).toEqual({ type: "TXT", txt: ["hello"] });
  });

  it("serializes NS to {type,ns}", () => {
    expect(rowToRecord({ type: "NS", value: "ns1.example." })).toEqual({
      type: "NS",
      ns: "ns1.example.",
    });
  });

  it("serializes SYNTH4/SYNTH6 to {type,address}", () => {
    expect(rowToRecord({ type: "SYNTH4", value: "1.2.3.4" })).toEqual({
      type: "SYNTH4",
      address: "1.2.3.4",
    });
    expect(rowToRecord({ type: "SYNTH6", value: "2001:db8::1" })).toEqual({
      type: "SYNTH6",
      address: "2001:db8::1",
    });
  });

  it("serializes GLUE4 to {type,ns,address}", () => {
    expect(rowToRecord({ type: "GLUE4", ns: "ns1.example.", address: "1.2.3.4" })).toEqual({
      type: "GLUE4",
      ns: "ns1.example.",
      address: "1.2.3.4",
    });
  });

  it("serializes GLUE6 to {type,ns,address}", () => {
    expect(rowToRecord({ type: "GLUE6", ns: "ns1.example.", address: "2001:db8::1" })).toEqual({
      type: "GLUE6",
      ns: "ns1.example.",
      address: "2001:db8::1",
    });
  });

  it("serializes DS with numeric keyTag/algorithm/digestType and hex digest string", () => {
    const rec = rowToRecord({
      type: "DS",
      keyTag: "12345",
      algorithm: "8",
      digestType: "2",
      digest: "ABCDEF0123456789",
    });
    expect(rec).toEqual({
      type: "DS",
      keyTag: 12345,
      algorithm: 8,
      digestType: 2,
      digest: "ABCDEF0123456789",
    });
    expect(typeof (rec as { keyTag: unknown }).keyTag).toBe("number");
    expect(typeof (rec as { algorithm: unknown }).algorithm).toBe("number");
    expect(typeof (rec as { digestType: unknown }).digestType).toBe("number");
    expect(typeof (rec as { digest: unknown }).digest).toBe("string");
  });

  it("drops a DS row missing any of its 4 fields", () => {
    expect(rowToRecord({ type: "DS", keyTag: "1", algorithm: "8", digestType: "2" })).toBeNull();
    expect(
      rowToRecord({ type: "DS", keyTag: "1", algorithm: "8", digestType: "2", digest: "" }),
    ).toBeNull();
    expect(rowToRecord({ type: "DS" })).toBeNull();
  });

  it("drops a GLUE row missing ns or address", () => {
    expect(rowToRecord({ type: "GLUE4", ns: "ns1.example." })).toBeNull();
    expect(rowToRecord({ type: "GLUE4", address: "1.2.3.4" })).toBeNull();
  });

  it("trims and drops blank single-value rows", () => {
    expect(rowToRecord({ type: "TXT", value: "  hi  " })).toEqual({ type: "TXT", txt: ["hi"] });
    expect(rowToRecord({ type: "NS", value: "   " })).toBeNull();
    expect(rowToRecord({ type: "TXT" })).toBeNull();
  });
});

describe("rowsToRecords", () => {
  it("round-trips a mixed set, skipping incomplete rows", () => {
    const records = rowsToRecords([
      { type: "TXT", value: "cua-agent-verified" },
      { type: "GLUE4", ns: "", address: "" },
      { type: "NS", value: "ns1.example." },
      {
        type: "DS",
        keyTag: "12345",
        algorithm: "8",
        digestType: "2",
        digest: "ABCDEF01",
      },
    ]);
    expect(records).toEqual([
      { type: "TXT", txt: ["cua-agent-verified"] },
      { type: "NS", ns: "ns1.example." },
      { type: "DS", keyTag: 12345, algorithm: 8, digestType: 2, digest: "ABCDEF01" },
    ]);
  });

  it("returns null when every row is blank (→ EMPTY resource)", () => {
    expect(rowsToRecords([{ type: "TXT", value: "" }])).toBeNull();
    expect(rowsToRecords([])).toBeNull();
  });
});
