import { useNameRecords } from "../../queries/read";
import { useNodeLive } from "../../queries/node";
import { Badge } from "../ui/Badge";

/**
 * Render a single hsd DNS record as a compact, human-readable row. Handles
 * every real hsd record type (NS, GLUE4/6, TXT, SYNTH4/6, DS) explicitly and
 * falls back to a value dump for unknown types so no data is ever hidden.
 */
export function renderRecord(rec: Record<string, unknown>): { label: string; value: string } {
  const type = String(rec.type ?? "?");
  switch (type) {
    case "NS":
      return { label: "NS", value: String(rec.ns ?? "") };
    case "GLUE4":
    case "GLUE6":
      return {
        label: type,
        value: `${String(rec.ns ?? "")} → ${String(rec.address ?? "")}`,
      };
    case "SYNTH4":
    case "SYNTH6":
      return { label: type, value: String(rec.address ?? "") };
    case "TXT": {
      const txt = Array.isArray(rec.txt) ? rec.txt.map(String).join(" ") : "";
      return { label: "TXT", value: txt };
    }
    case "DS": {
      const parts = [
        rec.keyTag !== undefined ? `keyTag=${rec.keyTag}` : null,
        rec.algorithm !== undefined ? `alg=${rec.algorithm}` : null,
        rec.digestType !== undefined ? `digestType=${rec.digestType}` : null,
        rec.digest !== undefined ? `digest=${rec.digest}` : null,
      ].filter(Boolean);
      return { label: "DS", value: parts.join(" ") };
    }
    default: {
      const rest = Object.entries(rec)
        .filter(([k]) => k !== "type")
        .map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`)
        .join(" ");
      return { label: type, value: rest };
    }
  }
}

/**
 * The name's current DNS records, read-only: for a name this wallet can't
 * edit right now. DNS records come ONLY from a synced local node (the explorer
 * can't return resources), and node liveness tells "genuinely no records"
 * from "node unavailable" — both return {records:[]} from the backend.
 */
export function ReadOnlyDnsRecords({
  name,
  profileId,
}: {
  name: string;
  profileId: string | null;
}) {
  const nodeLive = useNodeLive();
  const { data: resource } = useNameRecords(name, profileId);

  return (
    <div className="text-xs space-y-2">
      <div className="font-medium text-gray-700">DNS records</div>
      {!nodeLive ? (
        <div className="text-gray-500" data-testid="name-info-dns-no-node">
          Requires a synced local node to display current records.
        </div>
      ) : (
        <>
          {resource?.ttl !== undefined && resource?.ttl !== null && (
            <div className="text-gray-600">
              TTL: <span className="font-mono">{resource.ttl}s</span>
            </div>
          )}
          {resource?.records && resource.records.length > 0 ? (
            <table className="w-full text-xs" data-testid="name-info-dns-table">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1 pr-4 w-16">Type</th>
                  <th className="py-1">Value</th>
                </tr>
              </thead>
              <tbody>
                {resource.records.map((rec, i) => {
                  const { label, value } = renderRecord(rec);
                  return (
                    <tr key={i} className="border-t border-gray-100 hover:bg-gray-50 align-top">
                      <td className="py-1 pr-4">
                        <Badge variant="default">{label}</Badge>
                      </td>
                      <td className="py-1 font-mono break-all">
                        {value || <span className="text-gray-400">—</span>}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : (
            <div className="text-gray-500">No records</div>
          )}
        </>
      )}
    </div>
  );
}
