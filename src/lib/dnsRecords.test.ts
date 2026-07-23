import { describe, it, expect } from "vitest";
import { rowToRecord, rowsToRecords, recordToRow, recordsToRows } from "./dnsRecords";

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

describe("recordToRow — inverse mapper (Manage DNS prefill)", () => {
  it("converts TXT {type,txt:[...]} to row {type,value:joined}", () => {
    expect(recordToRow({ type: "TXT", txt: ["hello"] })).toEqual({ type: "TXT", value: "hello" });
    expect(recordToRow({ type: "TXT", txt: ["a", "b"] })).toEqual({ type: "TXT", value: "a b" });
  });

  it("converts NS {type,ns} to row {type,value}", () => {
    expect(recordToRow({ type: "NS", ns: "ns1.example." })).toEqual({
      type: "NS",
      value: "ns1.example.",
    });
  });

  it("converts SYNTH4/SYNTH6 {type,address} to row {type,value}", () => {
    expect(recordToRow({ type: "SYNTH4", address: "1.2.3.4" })).toEqual({
      type: "SYNTH4",
      value: "1.2.3.4",
    });
    expect(recordToRow({ type: "SYNTH6", address: "2001:db8::1" })).toEqual({
      type: "SYNTH6",
      value: "2001:db8::1",
    });
  });

  it("converts GLUE4/GLUE6 {type,ns,address} to row {type,ns,address}", () => {
    expect(recordToRow({ type: "GLUE4", ns: "ns1.example.", address: "1.2.3.4" })).toEqual({
      type: "GLUE4",
      ns: "ns1.example.",
      address: "1.2.3.4",
    });
    expect(recordToRow({ type: "GLUE6", ns: "ns1.example.", address: "2001:db8::1" })).toEqual({
      type: "GLUE6",
      ns: "ns1.example.",
      address: "2001:db8::1",
    });
  });

  it("converts DS {type,keyTag,algorithm,digestType,digest} to row (all as strings)", () => {
    const rec = recordToRow({
      type: "DS",
      keyTag: 12345,
      algorithm: 8,
      digestType: 2,
      digest: "ABCDEF0123456789",
    });
    expect(rec).toEqual({
      type: "DS",
      keyTag: "12345",
      algorithm: "8",
      digestType: "2",
      digest: "ABCDEF0123456789",
    });
    // Verify all fields are strings (editor holds text inputs).
    expect(typeof rec?.keyTag).toBe("string");
    expect(typeof rec?.algorithm).toBe("string");
    expect(typeof rec?.digestType).toBe("string");
    expect(typeof rec?.digest).toBe("string");
  });

  it("drops unknown record types", () => {
    expect(recordToRow({ type: "A", address: "1.2.3.4" })).toBeNull();
    expect(recordToRow({ type: "AAAA", address: "2001:db8::1" })).toBeNull();
    expect(recordToRow({ type: "CNAME", name: "example.com." })).toBeNull();
  });

  it("handles missing fields gracefully (coerces to empty strings)", () => {
    // NS missing `ns` → value becomes empty string (row is still created).
    expect(recordToRow({ type: "NS" })).toEqual({ type: "NS", value: "" });
    // GLUE missing `ns` or `address` → row is still created (editor can fill in).
    expect(recordToRow({ type: "GLUE4", address: "1.2.3.4" })).toEqual({
      type: "GLUE4",
      ns: "",
      address: "1.2.3.4",
    });
  });
});

describe("recordsToRows — batch inverse mapper", () => {
  it("round-trips a mixed set of hsd records to rows", () => {
    const records = [
      { type: "TXT", txt: ["hello"] },
      { type: "NS", ns: "ns1.example." },
      { type: "DS", keyTag: 12345, algorithm: 8, digestType: 2, digest: "AA" },
      { type: "GLUE4", ns: "ns2.example.", address: "1.2.3.4" },
    ];
    const rows = recordsToRows(records);
    expect(rows).toHaveLength(4);
    expect(rows[0]).toEqual({ type: "TXT", value: "hello" });
    expect(rows[1]).toEqual({ type: "NS", value: "ns1.example." });
    expect(rows[2]).toEqual({
      type: "DS",
      keyTag: "12345",
      algorithm: "8",
      digestType: "2",
      digest: "AA",
    });
    expect(rows[3]).toEqual({ type: "GLUE4", ns: "ns2.example.", address: "1.2.3.4" });
  });

  it("drops unknown record types from the result", () => {
    const records = [
      { type: "TXT", txt: ["hello"] },
      { type: "A", address: "1.2.3.4" }, // unknown
      { type: "NS", ns: "ns1.example." },
    ];
    const rows = recordsToRows(records);
    expect(rows).toHaveLength(2);
    expect(rows[0]?.type).toBe("TXT");
    expect(rows[1]?.type).toBe("NS");
  });
});
