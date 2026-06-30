import { useState } from "react";
import { useSyncNames, useSyncReport } from "../queries/sync";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { Button } from "./ui/Button";
import { formatDate, formatHns } from "../lib/utils";
import { useUiStore } from "../stores/ui";
import type { AuditEntry, WalletSnapshot } from "../types";

export function SyncVerification() {
  const syncNames = useSyncNames();
  const syncReport = useSyncReport();
  const showToast = useUiStore((s) => s.showToast);
  const [showReport, setShowReport] = useState(false);

  const { data: auditLog } = useQuery({
    queryKey: ["audit", "sync"],
    queryFn: () => invoke<AuditEntry[]>("get_audit_log", { limit: 10 }),
    staleTime: 30_000,
  });

  const { data: snapshots } = useQuery({
    queryKey: ["wallet", "snapshots"],
    queryFn: () => invoke<WalletSnapshot[]>("get_wallet_snapshots", { limit: 10 }),
    staleTime: 30_000,
  });

  const syncEntries = auditLog?.filter((e) => e.action === "sync") ?? [];

  const handleSync = async () => {
    try {
      await syncNames.mutateAsync();
      showToast("Sync complete", "success");
    } catch (e) {
      showToast(`Sync failed: ${e}`, "error");
    }
  };

  const handleReport = async () => {
    try {
      await syncReport.refetch();
      setShowReport(true);
    } catch (e) {
      showToast(`Report failed: ${e}`, "error");
    }
  };

  const result = syncNames.data;
  const report = syncReport.data;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Sync & Verification</h2>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            onClick={handleReport}
            disabled={syncReport.isFetching}
          >
            {syncReport.isFetching ? "Loading..." : "Compare Names"}
          </Button>
          <Button
            variant="primary"
            onClick={handleSync}
            disabled={syncNames.isPending}
          >
            {syncNames.isPending ? "Syncing..." : "Sync Now"}
          </Button>
        </div>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600">
        <p>
          <strong>Sync Now</strong> fetches names from your local hsd wallet, matches them against
          your imported inventory, and marks matched names as <strong>finalized_owned</strong>.
        </p>
        <p className="mt-1">
          <strong>Compare Names</strong> shows the diff without updating any statuses.
        </p>
      </div>

      {result && (
        <div className="space-y-4">
          <h3 className="text-sm font-semibold">Last Sync Result</h3>
          <div className="grid grid-cols-4 gap-4">
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Matched</div>
              <div className="text-2xl font-bold text-green-700">{result.matched}</div>
            </div>
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Wallet Names</div>
              <div className="text-2xl font-bold">{result.wallet_count}</div>
            </div>
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Extra in Wallet</div>
              <div className="text-2xl font-bold text-yellow-700">{result.extra_count}</div>
            </div>
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Not in Wallet</div>
              <div className="text-2xl font-bold text-orange-700">{result.missing_count}</div>
            </div>
          </div>

          {result.extra_names.length > 0 && (
            <div className="bg-white rounded p-4 border border-gray-200">
              <h4 className="text-sm font-semibold mb-2">
                Extra Wallet Names (not in inventory)
              </h4>
              <div className="max-h-40 overflow-auto">
                {result.extra_names.map((name) => (
                  <div key={name} className="text-sm font-mono py-0.5">
                    .{name}
                  </div>
                ))}
              </div>
            </div>
          )}

          {result.missing_names && result.missing_names.length > 0 && (
            <div className="bg-white rounded p-4 border border-gray-200">
              <h4 className="text-sm font-semibold mb-2">
                In Inventory, Not in Wallet ({result.missing_count})
              </h4>
              <div className="max-h-40 overflow-auto">
                {result.missing_names.map((name) => (
                  <div key={name} className="text-sm font-mono py-0.5">
                    .{name}
                  </div>
                ))}
              </div>
            </div>
          )}

          {result.errors.length > 0 && (
            <div className="bg-red-50 rounded p-4 border border-red-200">
              <h4 className="text-sm font-semibold mb-2 text-red-700">Errors</h4>
              {result.errors.map((err, i) => (
                <div key={i} className="text-sm text-red-600">
                  {err}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {showReport && report && (
        <div className="space-y-4">
          <h3 className="text-sm font-semibold">Name Comparison</h3>

          {report.matched.length > 0 && (
            <div className="bg-white rounded p-4 border border-gray-200">
              <h4 className="text-sm font-semibold mb-2 text-green-700">
                Matched ({report.matched.length})
              </h4>
              <div className="max-h-40 overflow-auto">
                {report.matched.map((name) => (
                  <div key={name} className="text-sm font-mono py-0.5">.{name}</div>
                ))}
              </div>
            </div>
          )}

          {report.missing.length > 0 && (
            <div className="bg-white rounded p-4 border border-gray-200">
              <h4 className="text-sm font-semibold mb-2 text-yellow-700">
                In Inventory, Not in Wallet ({report.missing.length})
              </h4>
              <div className="max-h-40 overflow-auto">
                {report.missing.map((name) => (
                  <div key={name} className="text-sm font-mono py-0.5">.{name}</div>
                ))}
              </div>
            </div>
          )}

          {report.extra.length > 0 && (
            <div className="bg-white rounded p-4 border border-gray-200">
              <h4 className="text-sm font-semibold mb-2 text-blue-700">
                In Wallet, Not in Inventory ({report.extra.length})
              </h4>
              <div className="max-h-40 overflow-auto">
                {report.extra.map((name) => (
                  <div key={name} className="text-sm font-mono py-0.5">.{name}</div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {syncNames.isError && (
        <div className="bg-red-50 rounded p-4 border border-red-200 text-red-700">
          Sync failed: {syncNames.error?.message || "Check your wallet connection settings and try again."}
        </div>
      )}

      {syncEntries.length > 0 && (
        <div className="bg-white rounded p-4 border border-gray-200">
          <h3 className="text-sm font-semibold mb-3">Sync History</h3>
          <div className="space-y-1">
            {syncEntries.map((entry) => (
              <div key={entry.id} className="flex items-center gap-3 text-xs">
                <span className="text-gray-400 w-32 shrink-0">{formatDate(entry.timestamp)}</span>
                <span className="font-medium text-gray-700">sync</span>
                <span className="text-gray-500 truncate">{entry.detail || ""}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {snapshots && snapshots.length > 0 && (
        <div className="bg-white rounded p-4 border border-gray-200">
          <h3 className="text-sm font-semibold mb-3">Wallet Snapshots</h3>
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-500 border-b">
                <th className="px-2 py-1">Time</th>
                <th className="px-2 py-1">Wallet</th>
                <th className="px-2 py-1">Balance</th>
                <th className="px-2 py-1">Names</th>
                <th className="px-2 py-1">Address</th>
              </tr>
            </thead>
            <tbody>
              {snapshots.map((snap) => (
                <tr key={snap.id} className="border-t border-gray-100">
                  <td className="px-2 py-1 text-xs text-gray-400">{formatDate(snap.snapshot_at)}</td>
                  <td className="px-2 py-1">{snap.wallet_name}</td>
                  <td className="px-2 py-1 font-mono">{formatHns(snap.balance)}</td>
                  <td className="px-2 py-1">{snap.name_count}</td>
                  <td className="px-2 py-1 text-xs text-gray-500 truncate max-w-[150px]">{snap.address || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
