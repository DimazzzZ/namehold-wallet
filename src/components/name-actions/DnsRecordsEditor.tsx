import {
  DNS_RECORD_TYPES,
  valuePlaceholder,
  type DnsRecordType,
  type DnsRow,
} from "../../lib/dnsRecords";
import { inputSizes } from "../ui/Input";

/**
 * The typed DNS rows editor (type dropdown + per-type field inputs + add/remove)
 * — used by both the guided REGISTER panel and the advanced "DNS records"
 * section (Task 13 / F6: this used to be duplicated JSX in `NameActionsModal`).
 * Rows state lives in the orchestrator; the two call sites only differ in their
 * data-testids, switched by `variant`.
 *
 * Field layout depends on the row's record type (Task 2 — real hsd type set):
 *  - TXT / NS / SYNTH4 / SYNTH6: a single value input.
 *  - GLUE4 / GLUE6: two inputs (nameserver + address).
 *  - DS: four inputs (keyTag, algorithm, digestType, digest).
 */
export interface DnsRecordsEditorProps {
  variant: "guided" | "advanced";
  rows: DnsRow[];
  onRowChange: (index: number, patch: Partial<DnsRow>) => void;
  onAddRow: () => void;
  onRemoveRow: (index: number) => void;
}

const inputClass = `flex-1 border border-gray-300 rounded font-mono min-w-0 ${inputSizes.sm}`;

function RowFields({
  row,
  onChange,
}: {
  row: DnsRow;
  onChange: (patch: Partial<DnsRow>) => void;
}) {
  switch (row.type) {
    case "TXT":
    case "NS":
    case "SYNTH4":
    case "SYNTH6":
      return (
        <input
          className={inputClass}
          value={row.value ?? ""}
          onChange={(e) => onChange({ value: e.target.value })}
          placeholder={valuePlaceholder(row.type)}
          aria-label="record value"
        />
      );
    case "GLUE4":
    case "GLUE6":
      return (
        <>
          <input
            className={inputClass}
            value={row.ns ?? ""}
            onChange={(e) => onChange({ ns: e.target.value })}
            placeholder="ns1.example."
            aria-label="nameserver"
          />
          <input
            className={inputClass}
            value={row.address ?? ""}
            onChange={(e) => onChange({ address: e.target.value })}
            placeholder={valuePlaceholder(row.type)}
            aria-label="address"
          />
        </>
      );
    case "DS":
      return (
        <>
          <input
            className={inputClass}
            value={row.keyTag ?? ""}
            onChange={(e) => onChange({ keyTag: e.target.value })}
            placeholder="12345"
            aria-label="key tag"
          />
          <input
            className={inputClass}
            value={row.algorithm ?? ""}
            onChange={(e) => onChange({ algorithm: e.target.value })}
            placeholder="algorithm"
            aria-label="algorithm"
          />
          <input
            className={inputClass}
            value={row.digestType ?? ""}
            onChange={(e) => onChange({ digestType: e.target.value })}
            placeholder="digest type"
            aria-label="digest type"
          />
          <input
            className={inputClass}
            value={row.digest ?? ""}
            onChange={(e) => onChange({ digest: e.target.value })}
            placeholder="hex digest"
            aria-label="digest"
          />
        </>
      );
  }
}

export function DnsRecordsEditor({
  variant,
  rows,
  onRowChange,
  onAddRow,
  onRemoveRow,
}: DnsRecordsEditorProps) {
  const rowsTestId = variant === "guided" ? "dns-rows" : "dns-rows-advanced";
  const addRowTestId = variant === "guided" ? "dns-add-row" : "dns-add-row-advanced";

  return (
    <div className="space-y-2" data-testid={rowsTestId}>
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-2">
          <select
            className="border border-gray-300 rounded px-2 py-1 text-xs"
            value={row.type}
            onChange={(e) => onRowChange(i, { type: e.target.value as DnsRecordType })}
            aria-label="record type"
          >
            {DNS_RECORD_TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
          <RowFields row={row} onChange={(patch) => onRowChange(i, patch)} />
          <button
            type="button"
            className="text-xs text-gray-400 hover:text-red-600 px-1"
            onClick={() => onRemoveRow(i)}
            aria-label="remove record"
          >
            ✕
          </button>
        </div>
      ))}
      <button
        type="button"
        className="text-xs text-blue-600 hover:underline"
        onClick={onAddRow}
        data-testid={addRowTestId}
      >
        + Add record
      </button>
    </div>
  );
}
