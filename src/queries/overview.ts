import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type {
  DashboardStats,
  OverviewData,
  OverviewMetric,
  StatusTone,
} from "../types";

interface NamebaseStatusPayload {
  connected?: boolean;
  hns_balance?: number;
  [key: string]: unknown;
}

function statusTone(status: string): StatusTone {
  const s = status.toLowerCase();
  if (s.includes("finalized") || s.includes("owned") || s.includes("complete"))
    return "success";
  if (s.includes("failed") || s.includes("stuck") || s.includes("error"))
    return "error";
  if (
    s.includes("pending") ||
    s.includes("waiting") ||
    s.includes("progress") ||
    s.includes("transfer")
  )
    return "warning";
  if (s.includes("staked") || s.includes("do_not_touch")) return "info";
  return "default";
}

export function useOverviewData() {
  return useQuery<OverviewData>({
    queryKey: ["overview"],
    queryFn: async () => {
      const stats = await invoke<DashboardStats>("get_dashboard_stats");

      let namebaseConnected = false;
      let namebaseHnsBalance: number | undefined;
      try {
        const nb = await invoke<NamebaseStatusPayload>("get_namebase_status");
        namebaseConnected = Boolean(nb?.connected);
        namebaseHnsBalance =
          typeof nb?.hns_balance === "number" ? nb.hns_balance : undefined;
      } catch {
        namebaseConnected = false;
      }

      const metrics: OverviewMetric[] = [
        {
          key: "total",
          label: "Total TLDs",
          value: stats.total,
          hint: "All imported names",
        },
        {
          key: "staked",
          label: "Staked",
          value: stats.staked,
          hint: "Do not touch on Namebase",
          tone: stats.staked > 0 ? "warning" : "default",
        },
        {
          key: "unstaked",
          label: "Migratable",
          value: stats.unstaked,
          hint: "Eligible for transfer",
          tone: stats.unstaked > 0 ? "info" : "default",
        },
        {
          key: "namebase",
          label: "Namebase",
          value: namebaseConnected ? "Connected" : "Not connected",
          tone: namebaseConnected ? "success" : "default",
        },
      ];

      return {
        metrics,
        statusCounts: stats.status_counts ?? {},
        recentAudit: stats.recent_audit ?? [],
        namebaseConnected,
        namebaseHnsBalance,
      };
    },
    staleTime: 30_000,
  });
}

export { statusTone };
