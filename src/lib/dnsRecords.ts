// Convert a simple {type, value} row editor into the hsd resource-record array
// that `build_register_draft` / `build_update_draft` accept (the same shape the
// raw-JSON textarea used: `[{"type":"TXT","txt":["…"]}, …]`).
//
// We cover the common record types with a single "value" field; anything more
// exotic (DS, GLUE4, …) is left to the Advanced raw-JSON editor, so the backend
// contract is unchanged either way.

export type DnsRecordType = "A" | "AAAA" | "CNAME" | "NS" | "TXT";

export const DNS_RECORD_TYPES: DnsRecordType[] = ["A", "AAAA", "CNAME", "NS", "TXT"];

export interface DnsRow {
  type: DnsRecordType;
  value: string;
}

/** Placeholder/help text per record type for the editor inputs. */
export function valuePlaceholder(type: DnsRecordType): string {
  switch (type) {
    case "A":
      return "1.2.3.4";
    case "AAAA":
      return "2001:db8::1";
    case "CNAME":
      return "target.example.";
    case "NS":
      return "ns1.example.";
    case "TXT":
      return "free text";
  }
}

/** Serialize one row to its hsd record object (null if the value is blank). */
export function rowToRecord(row: DnsRow): Record<string, unknown> | null {
  const v = row.value.trim();
  if (!v) return null;
  switch (row.type) {
    case "A":
    case "AAAA":
      return { type: row.type, address: v };
    case "CNAME":
      return { type: "CNAME", target: v };
    case "NS":
      return { type: "NS", ns: v };
    case "TXT":
      return { type: "TXT", txt: [v] };
  }
}

/**
 * Serialize the editor rows to the record array. Returns `null` when there are
 * no non-empty rows (→ an EMPTY resource, matching the old `safeRecords`).
 */
export function rowsToRecords(rows: DnsRow[]): Record<string, unknown>[] | null {
  const recs = rows.map(rowToRecord).filter((r): r is Record<string, unknown> => r !== null);
  return recs.length > 0 ? recs : null;
}
