import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { DashboardStats, AuditEntry } from "../types";
import { Button } from "./ui/Button";
import { Link } from "react-router-dom";
import { formatDate } from "../lib/utils";

export function Dashboard() {
  const { data: stats, isLoading, error } = useQuery({
    queryKey: ["dashboard"],
    queryFn: async () => {
      const allAssets = await invoke<{ id: number; status: string; is_staked: boolean }[]>(
        "list_assets",
      );
      const total = allAssets.length;
      const staked = allAssets.filter((a) => a.is_staked).length;
      const unstaked = total - staked;
      const statusCounts: Record<string, number> = {};
      for (const a of allAssets) {
        statusCounts[a.status] = (statusCounts[a.status] || 0) + 1;
      }
      const recentAudit = await invoke<AuditEntry[]>("get_audit_log", { limit: 5 }).catch(() => []);
      return { total, staked, unstaked, status_counts: statusCounts, recent_audit: recentAudit } as DashboardStats;
    },
    staleTime: 15_000,
  });

  if (isLoading) return <div className="text-gray-500">Loading dashboard...</div>;
  if (error) return <div className="text-red-600">Error loading dashboard</div>;
  if (!stats) return null;

  const statusEntries = [
    { key: "not_started", label: "Not Started", color: "bg-gray-200" },
    { key: "namebase_transfer_requested", label: "Transfer Requested", color: "bg-yellow-200" },
    { key: "waiting_transfer_tx", label: "Waiting TX", color: "bg-orange-200" },
    { key: "transfer_seen_on_chain", label: "TX Seen", color: "bg-blue-200" },
    { key: "waiting_finalize", label: "Waiting Finalize", color: "bg-indigo-200" },
    { key: "finalized_owned", label: "Finalized", color: "bg-green-200" },
    { key: "failed_or_stuck", label: "Failed/Stuck", color: "bg-red-200" },
    { key: "do_not_touch_staked", label: "Do Not Touch", color: "bg-purple-200" },
  ];

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-bold">Dashboard</h2>

      <div className="grid grid-cols-3 gap-4">
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Total TLDs</div>
          <div className="text-2xl font-bold">{stats.total}</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200 flex items-center gap-4">
          <div
            className="w-16 h-16 rounded-full shrink-0"
            style={{
              background: stats.total > 0
                ? `conic-gradient(#7c3aed ${stats.staked / stats.total * 100}%, #3b82f6 ${stats.staked / stats.total * 100}%)`
                : "#e5e7eb",
            }}
          />
          <div>
            <div className="text-sm text-gray-500">Staked / Unstaked</div>
            <div className="text-lg font-bold">
              <span className="text-purple-700">{stats.staked}</span>
              <span className="text-gray-400 mx-1">/</span>
              <span className="text-blue-700">{stats.unstaked}</span>
            </div>
          </div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Unstaked</div>
          <div className="text-2xl font-bold text-blue-700">{stats.unstaked}</div>
        </div>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200">
        <h3 className="text-sm font-semibold mb-3">Migration Status</h3>
        <div className="space-y-2">
          {statusEntries.map((se) => {
            const count = stats.status_counts[se.key] || 0;
            if (count === 0 && se.key !== "not_started") return null;
            const pct = stats.total > 0 ? (count / stats.total) * 100 : 0;
            return (
              <div key={se.key} className="flex items-center gap-3">
                <div className="w-32 text-xs text-gray-600">{se.label}</div>
                <div className="flex-1 bg-gray-100 rounded h-4 overflow-hidden">
                  <div
                    className={`h-full ${se.color} rounded`}
                    style={{ width: `${pct}%` }}
                  />
                </div>
                <div className="w-10 text-xs text-right text-gray-600">{count}</div>
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex gap-3">
        <Link to="/inventory">
          <Button variant="primary">View Inventory</Button>
        </Link>
        <Link to="/wallet">
          <Button>Wallet</Button>
        </Link>
        <Link to="/sync">
          <Button>Sync Names</Button>
        </Link>
        <Link to="/settings">
          <Button variant="ghost">Settings</Button>
        </Link>
      </div>

      {stats.recent_audit && stats.recent_audit.length > 0 && (
        <div className="bg-white rounded p-4 border border-gray-200">
          <h3 className="text-sm font-semibold mb-3">Recent Activity</h3>
          <div className="space-y-1">
            {stats.recent_audit.map((entry) => (
              <div key={entry.id} className="flex items-center gap-3 text-xs">
                <span className="text-gray-400 w-32 shrink-0">{formatDate(entry.timestamp)}</span>
                <span className="font-medium text-gray-700">{entry.action}</span>
                {entry.entity && (
                  <span className="text-gray-500">{entry.entity}{entry.entity_id ? `#${entry.entity_id}` : ""}</span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
