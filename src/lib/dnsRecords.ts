// Convert a simple {type, ...fields} row editor into the hsrd resource-record
// array that `build_register_draft` / `build_update_draft` accept (the same
// shape the raw-JSON textarea used, e.g. `[{"type":"TXT","txt":["…"]}, …]`).
//
// The record-type set mirrors what the Rust backend (`resource.rs::encode`)
// actually supports for Handshake root-zone resources. A/AAAA/CNAME are NOT
// valid hsrd record types — the backend rejects them, so they are intentionally
// absent here.

export type DnsRecordType = "DS" | "NS" | "GLUE4" | "GLUE6" | "SYNTH4" | "SYNTH6" | "TXT";

export const DNS_RECORD_TYPES: DnsRecordType[] = [
  "DS",
  "NS",
  "GLUE4",
  "GLUE6",
  "SYNTH4",
  "SYNTH6",
  "TXT",
];

export interface DnsRow {
  type: DnsRecordType;
  /** Single-value types: NS (nameserver), SYNTH4/SYNTH6 (address), TXT (text). */
  value?: string;
  /** GLUE4/GLUE6 nameserver name. */
  ns?: string;
  /** GLUE4/GLUE6 address (IPv4/IPv6). */
  address?: string;
  /** DS fields (held as text inputs; converted to numbers in rowToRecord). */
  keyTag?: string;
  algorithm?: string;
  digestType?: string;
  digest?: string;
}

/** Placeholder/help text per record type for the editor inputs. */
export function valuePlaceholder(type: DnsRecordType): string {
  switch (type) {
    case "NS":
      return "ns1.example.";
    case "GLUE4":
      return "1.2.3.4";
    case "GLUE6":
      return "2001:db8::1";
    case "SYNTH4":
      return "1.2.3.4";
    case "SYNTH6":
      return "2001:db8::1";
    case "TXT":
      return "free text";
    case "DS":
      return "";
  }
}

/** Serialize one row to its hsrd record object (null if required fields are blank). */
export function rowToRecord(row: DnsRow): Record<string, unknown> | null {
  switch (row.type) {
    case "TXT": {
      const v = (row.value ?? "").trim();
      if (!v) return null;
      return { type: "TXT", txt: [v] };
    }
    case "NS": {
      const v = (row.value ?? "").trim();
      if (!v) return null;
      return { type: "NS", ns: v };
    }
    case "SYNTH4":
    case "SYNTH6": {
      const v = (row.value ?? "").trim();
      if (!v) return null;
      return { type: row.type, address: v };
    }
    case "GLUE4":
    case "GLUE6": {
      const ns = (row.ns ?? "").trim();
      const address = (row.address ?? "").trim();
      if (!ns || !address) return null;
      return { type: row.type, ns, address };
    }
    case "DS": {
      const keyTag = (row.keyTag ?? "").trim();
      const algorithm = (row.algorithm ?? "").trim();
      const digestType = (row.digestType ?? "").trim();
      const digest = (row.digest ?? "").trim();
      if (!keyTag || !algorithm || !digestType || !digest) return null;
      return {
        type: "DS",
        keyTag: Number(keyTag),
        algorithm: Number(algorithm),
        digestType: Number(digestType),
        digest,
      };
    }
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

// ---------------------------------------------------------------------------
// Inverse: hsrd record → editor row (for prefilling current records from node)
// ---------------------------------------------------------------------------

/** Convert a single hsrd record object back into an editor row. Returns `null`
 *  for unrecognized types (they're silently dropped from the row editor; the
 *  user can still see/edit them in the raw-JSON Advanced mode). */
export function recordToRow(rec: Record<string, unknown>): DnsRow | null {
  if (!rec || typeof rec !== "object") return null;
  switch (rec.type) {
    case "TXT": {
      const txt = Array.isArray(rec.txt) ? rec.txt : [];
      return { type: "TXT", value: txt.map(String).join(" ") };
    }
    case "NS":
      return { type: "NS", value: String(rec.ns ?? "") };
    case "SYNTH4":
    case "SYNTH6":
      return { type: rec.type as "SYNTH4" | "SYNTH6", value: String(rec.address ?? "") };
    case "GLUE4":
    case "GLUE6":
      return {
        type: rec.type as "GLUE4" | "GLUE6",
        ns: String(rec.ns ?? ""),
        address: String(rec.address ?? ""),
      };
    case "DS":
      return {
        type: "DS",
        keyTag: String(rec.keyTag ?? ""),
        algorithm: String(rec.algorithm ?? ""),
        digestType: String(rec.digestType ?? ""),
        digest: String(rec.digest ?? ""),
      };
    default:
      return null;
  }
}

/** Convert an array of hsrd records into editor rows, dropping unknown types. */
export function recordsToRows(records: Record<string, unknown>[]): DnsRow[] {
  return records.map(recordToRow).filter((r): r is DnsRow => r !== null);
}
