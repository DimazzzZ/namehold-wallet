import {
  DNS_RECORD_TYPES,
  valuePlaceholder,
  type DnsRecordType,
  type DnsRow,
} from "../../lib/dnsRecords";

/**
 * The typed DNS rows editor (type dropdown + value input + add/remove) — used
 * by both the guided REGISTER panel and the advanced "DNS records" section
 * (Task 13 / F6: this used to be duplicated JSX in `NameActionsModal`). Rows
 * state lives in the orchestrator; the two call sites only differ in their
 * data-testids, switched by `variant`.
 */
export interface DnsRecordsEditorProps {
  variant: "guided" | "advanced";
  rows: DnsRow[];
  onRowChange: (index: number, patch: Partial<DnsRow>) => void;
  onAddRow: () => void;
  onRemoveRow: (index: number) => void;
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
          <input
            className="flex-1 border border-gray-300 rounded px-2 py-1 text-xs font-mono"
            value={row.value}
            onChange={(e) => onRowChange(i, { value: e.target.value })}
            placeholder={valuePlaceholder(row.type)}
            aria-label="record value"
          />
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
