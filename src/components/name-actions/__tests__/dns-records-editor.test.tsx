import { describe, it, expect } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
import { useState } from "react";

import { DnsRecordsEditor } from "../DnsRecordsEditor";
import { rowToRecord, type DnsRow } from "../../../lib/dnsRecords";

// Task 2: real hsd record-type set (DS/NS/GLUE4/GLUE6/SYNTH4/SYNTH6/TXT) with
// multi-field inputs for DS (4 fields) and GLUE4/GLUE6 (2 fields).

function Harness({ initial }: { initial: DnsRow[] }) {
  const [rows, setRows] = useState<DnsRow[]>(initial);
  const setRow = (i: number, patch: Partial<DnsRow>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const addRow = () => setRows((rs) => [...rs, { type: "TXT", value: "" }]);
  const removeRow = (i: number) => setRows((rs) => rs.filter((_, j) => j !== i));
  return (
    <>
      <DnsRecordsEditor
        variant="guided"
        rows={rows}
        onRowChange={setRow}
        onAddRow={addRow}
        onRemoveRow={removeRow}
      />
      <pre data-testid="rows-json">{JSON.stringify(rows)}</pre>
    </>
  );
}

describe("DnsRecordsEditor — hsd record-type fields (Task 2)", () => {
  it("selecting DS shows 4 inputs and produces a correct DS record", () => {
    render(<Harness initial={[{ type: "TXT", value: "" }]} />);

    fireEvent.change(screen.getByLabelText("record type"), { target: { value: "DS" } });

    fireEvent.change(screen.getByLabelText("key tag"), { target: { value: "12345" } });
    fireEvent.change(screen.getByLabelText("algorithm"), { target: { value: "8" } });
    fireEvent.change(screen.getByLabelText("digest type"), { target: { value: "2" } });
    fireEvent.change(screen.getByLabelText("digest"), { target: { value: "ABCDEF01" } });

    const rows: DnsRow[] = JSON.parse(screen.getByTestId("rows-json").textContent || "[]");
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      type: "DS",
      keyTag: "12345",
      algorithm: "8",
      digestType: "2",
      digest: "ABCDEF01",
    });
    expect(rowToRecord(rows[0]!)).toEqual({
      type: "DS",
      keyTag: 12345,
      algorithm: 8,
      digestType: 2,
      digest: "ABCDEF01",
    });

    // exactly the 4 DS fields are present, no leftover single "record value" input
    expect(screen.queryByLabelText("record value")).not.toBeInTheDocument();
    expect(screen.getByLabelText("key tag")).toBeInTheDocument();
    expect(screen.getByLabelText("algorithm")).toBeInTheDocument();
    expect(screen.getByLabelText("digest type")).toBeInTheDocument();
    expect(screen.getByLabelText("digest")).toBeInTheDocument();
  });

  it("selecting GLUE4 shows 2 inputs (ns + address) and produces a correct GLUE4 record", () => {
    render(<Harness initial={[{ type: "TXT", value: "" }]} />);

    fireEvent.change(screen.getByLabelText("record type"), { target: { value: "GLUE4" } });

    fireEvent.change(screen.getByLabelText("nameserver"), { target: { value: "ns1.example." } });
    fireEvent.change(screen.getByLabelText("address"), { target: { value: "1.2.3.4" } });

    const rows: DnsRow[] = JSON.parse(screen.getByTestId("rows-json").textContent || "[]");
    expect(rows[0]).toMatchObject({ type: "GLUE4", ns: "ns1.example.", address: "1.2.3.4" });
    expect(rowToRecord(rows[0]!)).toEqual({
      type: "GLUE4",
      ns: "ns1.example.",
      address: "1.2.3.4",
    });

    expect(screen.queryByLabelText("record value")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("key tag")).not.toBeInTheDocument();
  });

  it("dropdown lists the real hsd type set and excludes A/AAAA/CNAME", () => {
    render(<Harness initial={[{ type: "TXT", value: "" }]} />);
    const select = screen.getByLabelText("record type") as HTMLSelectElement;
    const optionValues = Array.from(select.options).map((o) => o.value);
    expect(optionValues).toEqual(["DS", "NS", "GLUE4", "GLUE6", "SYNTH4", "SYNTH6", "TXT"]);
    expect(optionValues).not.toContain("A");
    expect(optionValues).not.toContain("AAAA");
    expect(optionValues).not.toContain("CNAME");
  });
});
