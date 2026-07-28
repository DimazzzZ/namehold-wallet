import { useReadNameInfo, useNameBids, useNameRecords } from "../queries/read";
import { useActiveProfile } from "../queries/wallet";
import { useNodeLive } from "../queries/node";
import { Dialog } from "./ui/Dialog";
import { Badge } from "./ui/Badge";
import { formatHns } from "../lib/utils";
import { displayName } from "../lib/idn";
import { explorerNameUrl, openExternal } from "../lib/openExternal";
import { auctionPhase, nextTransition, formatCountdown } from "../lib/auction";
import type { NameBid } from "../types";

/**
 * Render a single hsd DNS record as a compact, human-readable row. Handles
 * every real hsd record type (NS, GLUE4/6, TXT, SYNTH4/6, DS) explicitly and
 * falls back to a value dump for unknown types so no data is ever hidden.
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
      // Unknown / future types: dump every field except `type` as key=value
      // pairs so nothing is silently hidden but it still reads cleanly.
      const rest = Object.entries(rec)
        .filter(([k]) => k !== "type")
        .map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`)
        .join(" ");
      return { label: type, value: rest };
    }
  }
}

interface NameInfoModalProps {
  name: string | null;
  open: boolean;
  onClose: () => void;
}

/**
 * Read-only modal displaying current on-chain state for a name from the hsd
 * node. Shows: state badge, countdowns, owner UTXO, registration/renewal
 * heights, DNS records, and your bids on the name. Gracefully degrades when
 * the node is unavailable (state/countdowns from explorer fallback; DNS
 * records show "requires synced node").
 *
 * Separate from `NameActionsModal` — this is purely informational. The
 * "View on explorer" link is preserved for full history.
 */
export function NameInfoModal({ name, open, onClose }: NameInfoModalProps) {
  const { data: profile } = useActiveProfile();
  const profileId = profile?.id ?? null;
  // DNS records come ONLY from a synced local node (the explorer can't return
  // resources). Use node-liveness to distinguish "genuinely no records" from
  // "node unavailable" — both return `{records:[]}` from the backend.
  const nodeLive = useNodeLive();
  const { data: nameInfo, isLoading, isError } = useReadNameInfo(open ? name : null);
  const { data: resource } = useNameRecords(open ? name : null, profileId);
  const { data: bids } = useNameBids(open ? name : null, profileId);

  if (!open || !name) return null;

  const decodedName = displayName(name);
  const badge = nameInfo ? auctionPhase(nameInfo.state) : null;
  const countdown = nameInfo ? nextTransition(nameInfo.state, nameInfo.stats) : null;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      className="max-w-2xl"
      title={
        decodedName === name ? (
          `.${name}`
        ) : (
          <>
            .{decodedName}{" "}
            <span className="text-xs font-normal text-gray-400">(.{name})</span>
          </>
        )
      }
    >
      <div className="space-y-4 text-sm max-h-[70vh] overflow-y-auto">
        {/* Explorer link */}
        <button
          type="button"
          className="text-xs text-blue-500 hover:text-blue-700 hover:underline cursor-pointer inline-flex items-center gap-1"
          onClick={() => openExternal(explorerNameUrl(name))}
          data-testid="name-explorer-link"
        >
          View on explorer ↗
        </button>

        {/* Loading state */}
        {isLoading && (
          <div className="text-center py-4">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600 mx-auto"></div>
            <div className="mt-2 text-sm text-gray-600">Loading name info...</div>
          </div>
        )}

        {/* Error state */}
        {isError && (
          <div className="bg-red-50 border border-red-300 rounded p-3 text-sm text-red-800">
            Failed to load name info. Try again or view on the explorer.
          </div>
        )}

        {/* Content */}
        {nameInfo && !isLoading && (
          <div className="space-y-4">
            {/* State + flags */}
            <div className="flex items-center gap-2 flex-wrap">
              {badge && <Badge variant={badge.variant}>{badge.label}</Badge>}
              {nameInfo.registered && (
                <Badge variant="success">Registered</Badge>
              )}
              {nameInfo.expired &&
                !["OPENING", "BIDDING", "REVEAL"].includes(
                  (nameInfo.state ?? "").toUpperCase()
                ) && <Badge variant="error">Expired</Badge>}
            </div>

            {/* Countdown (if applicable) */}
            {countdown && (
              <div className="text-xs text-gray-600" data-testid="name-info-countdown">
                {countdown.label} {formatCountdown(countdown)}
              </div>
            )}

            {/* Registration heights */}
            {(nameInfo.height !== null || nameInfo.renewal !== null) && (
              <div className="text-xs space-y-1 border-t border-gray-200 pt-2">
                {nameInfo.height !== null && (
                  <div className="flex justify-between">
                    <span className="text-gray-600">Opened:</span>
                    <span className="font-mono">Block {nameInfo.height}</span>
                  </div>
                )}
                {nameInfo.renewal !== null && (
                  <div className="flex justify-between">
                    <span className="text-gray-600">Renewed:</span>
                    <span className="font-mono">Block {nameInfo.renewal}</span>
                  </div>
                )}
              </div>
            )}

            {/* Transfer status */}
            {nameInfo.transfer && nameInfo.transfer > 0 && (
              <div className="text-xs bg-yellow-50 border border-yellow-200 rounded p-2">
                <span className="text-yellow-900">
                  Transfer in progress (height {nameInfo.transfer})
                </span>
              </div>
            )}

            {/* Owner UTXO */}
            {nameInfo.owner && (
              <div className="text-xs space-y-1 border-t border-gray-200 pt-2">
                <div className="text-gray-600">Owner UTXO:</div>
                <div className="font-mono text-xs break-all">
                  {nameInfo.owner.hash}:{nameInfo.owner.index}
                </div>
              </div>
            )}

            {/* Auction values (CLOSED) */}
            {nameInfo.state === "CLOSED" && (nameInfo.value !== null || nameInfo.highest !== null) && (
              <div className="text-xs space-y-1 border-t border-gray-200 pt-2">
                {nameInfo.value !== null && (
                  <div className="flex justify-between">
                    <span
                      className="text-gray-600 cursor-help"
                      title="Handshake uses a Vickrey second-price auction: the winner pays the second-highest bid, not their own bid."
                    >
                      Paid price (2nd-price):
                    </span>
                    <span className="font-mono">{formatHns(nameInfo.value)}</span>
                  </div>
                )}
                {nameInfo.highest !== null && (
                  <div className="flex justify-between">
                    <span className="text-gray-600">Top bid:</span>
                    <span className="font-mono">{formatHns(nameInfo.highest)}</span>
                  </div>
                )}
              </div>
            )}

            {/* DNS records — node-only. */}
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
                            <tr
                              key={i}
                              className="border-t border-gray-100 hover:bg-gray-50 align-top"
                            >
                              <td className="py-1 pr-4">
                                <Badge variant="default">{label}</Badge>
                              </td>
                              <td className="py-1 font-mono break-all">
                                {value || (
                                  <span className="text-gray-400">—</span>
                                )}
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

            {/* My bids on this name */}
            {bids && bids.bids && bids.bids.length > 0 && (
              <div className="text-xs space-y-2 border-t border-gray-200 pt-2">
                <div className="font-medium text-gray-700">
                  Your bids ({bids.bids.length} ·{" "}
                  {bids.bids.filter((b: NameBid) => b.revealed).length} revealed)
                </div>
                <div className="space-y-1 max-h-32 overflow-y-auto">
                  {bids.bids.map((bid: NameBid, i: number) => (
                    <div
                      key={i}
                      className="bg-gray-50 rounded p-1 flex justify-between"
                    >
                      <span className="font-mono">
                        {bid.value !== null && bid.value !== undefined
                          ? formatHns(bid.value)
                          : "masked"}
                      </span>
                      <span className="text-gray-500">
                        {bid.revealed ? "revealed" : "masked"}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}
