import { useCallback } from "react";
import { useAssets, useExportCsv } from "../queries/assets";
import type { Asset } from "../types";
import { formatDate } from "../lib/utils";
import { Button } from "./ui/Button";
import { useUiStore } from "../stores/ui";
import { save } from "@tauri-apps/plugin-dialog";

export function Renewals() {
  const { data: assets = [], isLoading } = useAssets({});
  const exportCsv = useExportCsv();
  const showToast = useUiStore((s) => s.showToast);

  const withExpiry = assets
    .filter((a) => a.days_until_expire != null || a.expires_at_height != null)
    .sort((a, b) => (a.days_until_expire ?? 999999) - (b.days_until_expire ?? 999999));

  const getColor = (asset: Asset): string => {
    if (asset.days_until_expire != null) {
      if (asset.days_until_expire < 30) return "text-red-600";
      if (asset.days_until_expire < 90) return "text-yellow-600";
      return "text-green-600";
    }
    return "text-gray-400";
  };

  const handleExport = useCallback(async () => {
    const path = await save({
      filters: [{ name: "CSV", extensions: ["csv"] }],
      defaultPath: "hns-renewals-export.csv",
    });
    if (!path) return;
    try {
      const count = await exportCsv.mutateAsync({ path });
      showToast(`Exported ${count} TLDs`, "success");
    } catch (e) {
      showToast(`Export failed: ${e}`, "error");
    }
  }, [exportCsv, showToast]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Renewals</h2>
        <Button size="sm" onClick={handleExport} disabled={withExpiry.length === 0}>
          Export CSV
        </Button>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600">
        Shows TLDs with known expiration data from hsd. Data is populated after syncing
        with wallet. Color coding: red (&lt;30 days), yellow (&lt;90 days), green (&gt;90 days).
        Renewal tracking is read-only in MVP.
      </div>

      {isLoading ? (
        <div className="text-gray-500">Loading...</div>
      ) : withExpiry.length === 0 ? (
        <div className="text-gray-500 bg-white rounded p-8 border text-center">
          No renewal data available. Run a sync to populate expiration data.
        </div>
      ) : (
        <div className="bg-white rounded border border-gray-200">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-500 border-b">
                <th className="px-3 py-2">TLD</th>
                <th className="px-3 py-2">Status</th>
                <th className="px-3 py-2">Name State</th>
                <th className="px-3 py-2">Days Until Expire</th>
                <th className="px-3 py-2">Expires At Height</th>
                <th className="px-3 py-2">Last Synced</th>
              </tr>
            </thead>
            <tbody>
              {withExpiry.map((asset) => (
                <tr key={asset.id} className="border-t border-gray-100">
                  <td className="px-3 py-2 font-mono">.{asset.tld}</td>
                  <td className="px-3 py-2">{asset.status}</td>
                  <td className="px-3 py-2">{asset.name_state || "—"}</td>
                  <td className={`px-3 py-2 font-mono font-semibold ${getColor(asset)}`}>
                    {asset.days_until_expire != null ? `${Math.round(asset.days_until_expire)}d` : "—"}
                  </td>
                  <td className="px-3 py-2 font-mono text-gray-400">
                    {asset.expires_at_height ? `#${asset.expires_at_height}` : "—"}
                  </td>
                  <td className="px-3 py-2 text-gray-400 text-xs">
                    {formatDate(asset.last_synced_at)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
