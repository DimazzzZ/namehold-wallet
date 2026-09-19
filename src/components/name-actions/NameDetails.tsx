import { useNameRecords } from "../../queries/read";
import { useNodeLive } from "../../queries/node";
import { Badge } from "../ui/Badge";
import { formatHns } from "../../lib/utils";
import { Tooltip } from "../ui/Tooltip";
import type { HsdName } from "../../types";

/**
 * Render a single hsd DNS record as a compact, human-readable row. Handles
 * every real hsd record type (NS, GLUE4/6, TXT, SYNTH4/6, DS) explicitly and
 * falls back to a value dump for unknown types so no data is ever hidden.
 *
 * Moved here from the former NameInfoModal (Task T4: unify the read-only name
 * inspector into NameActionsModal). Single source of truth for read-only
 * name details now.
 */
function renderRecord(rec: Record<string, unknown>): { label: string; value: string } {
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

interface NameDetailsProps {
  name: string;
  profileId: string | null;
  info: HsdName | null | undefined;
  /**
   * When true, the DNS records table is owned by NameActionsModal's editable
   * DnsRecordsEditor (owned names in the advanced section), so this read-only
   * section suppresses its own DNS block to avoid showing records twice.
   */
  hideDnsRecords?: boolean;
}

/**
 * Read-only on-chain details for a name: registration/renewal heights,
 * transfer status, owner UTXO, closed-auction values, and DNS records.
 * Formerly the body of the standalone NameInfoModal; now embedded inside
 * NameActionsModal so one modal serves both inspection and actions.
 *
 * DNS records come ONLY from a synced local node (the explorer can't return
 * resources). Node-liveness distinguishes "genuinely no records" from "node
 * unavailable" — both return {records:[]} from the backend.
 */
export function NameDetails({ name, profileId, info, hideDnsRecords }: NameDetailsProps) {
  const nodeLive = useNodeLive();
  const { data: resource } = useNameRecords(name, profileId);

  if (!info) return null;

  const hasHeights = info.height !== null || info.renewal !== null;
  const hasTransfer = info.transfer != null && info.transfer > 0;
  const hasOwner = !!info.owner;
  const hasClosedValues = info.state === "CLOSED" && (info.value !== null || info.highest !== null);

  return (
    <div className="space-y-4 text-sm" data-testid="name-details">
      {hasHeights && (
        <div className="text-xs space-y-1 border-t border-gray-200 pt-2">
          {info.height !== null && (
            <div className="flex justify-between">
              <span className="text-gray-600">Opened:</span>
              <span className="font-mono">Block {info.height}</span>
            </div>
          )}
          {info.renewal !== null && (
            <div className="flex justify-between">
              <span className="text-gray-600">Renewed:</span>
              <span className="font-mono">Block {info.renewal}</span>
            </div>
          )}
        </div>
      )}

      {hasTransfer && (
        <div className="text-xs bg-yellow-50 border border-yellow-200 rounded p-2">
          <span className="text-yellow-900">Transfer in progress (height {info.transfer})</span>
        </div>
      )}

      {hasOwner && (
        <div className="text-xs space-y-1 border-t border-gray-200 pt-2">
          <div className="text-gray-600">Owner UTXO:</div>
          <div className="font-mono text-xs break-all">
            {info.owner!.hash}:{info.owner!.index}
          </div>
        </div>
      )}

      {hasClosedValues && (
        <div className="text-xs space-y-1 border-t border-gray-200 pt-2">
          {info.value !== null && (
            <div className="flex justify-between">
              <Tooltip content="Handshake uses a Vickrey second-price auction: the winner pays the second-highest bid, not their own bid.">
                <span className="text-gray-600 cursor-help">Paid price (2nd-price):</span>
              </Tooltip>
              <span className="font-mono">{formatHns(info.value)}</span>
            </div>
          )}
          {info.highest !== null && (
            <div className="flex justify-between">
              <span className="text-gray-600">Top bid:</span>
              <span className="font-mono">{formatHns(info.highest)}</span>
            </div>
          )}
        </div>
      )}

      {!hideDnsRecords && (
        <div className="text-xs space-y-2 border-t border-gray-200 pt-2">
          <div className="font-medium text-gray-700">DNS Records</div>
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
      )}
    </div>
  );
}
