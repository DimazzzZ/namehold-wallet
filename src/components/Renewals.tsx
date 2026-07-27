import { useCallback, useState } from "react";
import { useReadRenewals } from "../queries/read";
import { useActiveProfile } from "../queries/wallet";
import { useExportCsv } from "../queries/assets";
import type { RenewalRow } from "../types";
import { formatCount } from "../lib/utils";
import { displayName } from "../lib/idn";
import { explorerNameUrl, openExternal } from "../lib/openExternal";
import { Button } from "./ui/Button";
import { NameActionsModal } from "./NameActionsModal";
import { useUiStore } from "../stores/ui";
import { save } from "../lib/dialog";

/**
 * Renewals screen — driven by the chain-computed `read_renewals` command.
 * Days-until-expiry are computed LIVE (renewal height + network renewal window
 * vs the current height); CSV-imported columns only survive as a per-row
 * fallback and are honestly badged as such. On Handshake a missed renewal
 * loses the name forever, so this screen must never show silently stale data.
 */
export function Renewals() {
  const { data, isLoading } = useReadRenewals();
  const { data: activeProfile } = useActiveProfile();
  const isMainnet = activeProfile?.network === "mainnet";
  const exportCsv = useExportCsv();
  const showToast = useUiStore((s) => s.showToast);
  const [manageName, setManageName] = useState<string | null>(null);

  const rows = data?.names ?? [];
  const threshold = data?.expiringSoonThresholdDays ?? 30;

  const getColor = (row: RenewalRow): string => {
    if (row.daysUntilExpire != null) {
      if (row.daysUntilExpire <= threshold) return "text-red-600";
      if (row.daysUntilExpire < 90) return "text-yellow-600";
      return "text-green-600";
    }
    return "text-gray-400";
  };

  const heightSourceNote = (() => {
    switch (data?.heightSource) {
      case "node":
        return `Live from your synced node (height #${data?.currentHeight ?? "?"}).`;
      case "explorer":
        return `Estimated from last sync data (height ~#${data?.currentHeight ?? "?"}). Connect a node and Refresh for exact values.`;
      default:
        return "No chain height available — run Sync or connect a node. Days shown for chain rows require a known height.";
    }
  })();

  const handleExport = useCallback(async () => {
    const path = await save({
      filters: [{ name: "CSV", extensions: ["csv"] }],
      defaultPath: "hns-renewals-export.csv",
    });
    if (!path) return;
    try {
      const count = await exportCsv.mutateAsync({ path });
      showToast(`Exported ${formatCount(count)} TLDs`, "success");
    } catch (e) {
      showToast(`Export failed: ${e}`, "error");
    }
  }, [exportCsv, showToast]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Renewals</h2>
        <Button size="sm" onClick={handleExport} disabled={rows.length === 0}>
          Export CSV
        </Button>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600 space-y-1">
        <div>
          Expiry is computed from on-chain renewal data (source badge{" "}
          <span className="font-mono">chain</span>). Rows marked{" "}
          <span className="font-mono">CSV import</span> only have stale imported data —
          run a Sync to get live values. Color coding: red (&lt;{Math.round(threshold)}{" "}
          days), yellow (&lt;90 days), green (&gt;90 days).
        </div>
        <div className="text-xs text-gray-500" data-testid="renewals-height-source">
          Height source: {heightSourceNote}
        </div>
      </div>

      {isLoading ? (
        <div className="text-gray-500">Loading...</div>
      ) : rows.length === 0 ? (
        <div className="text-gray-500 bg-white rounded p-8 border text-center">
          No renewal data available. Run a sync to populate expiration data.
        </div>
      ) : (
        <div className="bg-white rounded border border-gray-200">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-500 border-b">
                <th className="px-3 py-2">TLD</th>
                <th className="px-3 py-2">Name State</th>
                <th className="px-3 py-2">Days Until Expire</th>
                <th className="px-3 py-2">Expires At Height</th>
                <th className="px-3 py-2">Source</th>
                <th className="px-3 py-2"></th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.name} className="border-t border-gray-100">
                  <td className="px-3 py-2 font-mono">
                    {isMainnet ? (
                      <button
                        type="button"
                        className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                        onClick={() => openExternal(explorerNameUrl(row.name))}
                        title="View on explorer"
                        data-testid="renewals-name-explorer-link"
                      >
                        .{displayName(row.name)}
                      </button>
                    ) : (
                      `.${displayName(row.name)}`
                    )}
                  </td>
                  <td className="px-3 py-2">{row.state || "—"}</td>
                  <td className={`px-3 py-2 font-mono font-semibold ${getColor(row)}`}>
                    {row.daysUntilExpire != null
                      ? row.daysUntilExpire < 0
                        ? "Expired"
                        : `${Math.floor(row.daysUntilExpire)}d`
                      : "—"}
                  </td>
                  <td className="px-3 py-2 font-mono text-gray-400">
                    {row.expiresAtHeight != null ? `#${row.expiresAtHeight}` : "—"}
                  </td>
                  <td className="px-3 py-2">
                    {row.source === "chain" ? (
                      <span
                        className="inline-block px-2 py-0.5 rounded text-xs bg-green-100 text-green-800"
                        data-testid={`source-${row.name}`}
                      >
                        chain
                      </span>
                    ) : (
                      <span
                        className="inline-block px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-600"
                        title="Stale data from the CSV import — run a Sync to get live chain values."
                        data-testid={`source-${row.name}`}
                      >
                        CSV import
                      </span>
                    )}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <Button
                      size="sm"
                      variant={row.expiringSoon ? "primary" : "secondary"}
                      data-testid={`renew-${row.name}`}
                      onClick={() => setManageName(row.name)}
                    >
                      Renew
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {manageName && (
        <NameActionsModal
          name={manageName}
          open={!!manageName}
          onClose={() => setManageName(null)}
        />
      )}
    </div>
  );
}
