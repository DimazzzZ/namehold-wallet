import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { writeBlockedReason } from "../lib/providerMode";
import type {
  DashboardStats,
  HsdBalance,
  OverviewData,
  OverviewMetric,
  ReadContext,
  StatusTone,
  WalletReadModel,
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

      // Provider-aware read context + balance (best-effort: these must never
      // break the overview when the active provider is unreachable).
      const readContext = await invoke<ReadContext>("get_read_context").catch(
        () => null,
      );
      const balance = await invoke<HsdBalance | null>("read_balance").catch(
        () => null,
      );

      const providerWarnings: string[] = [];
      if (readContext) {
        const provider = readContext.activeReadProvider;
        if (provider && !provider.healthy) {
          providerWarnings.push(
            `${provider.label} is currently unavailable${
              provider.reason ? `: ${provider.reason}` : "."
            }`,
          );
        }
        if (readContext.fallbackActive) {
          providerWarnings.push(
            "Local node is unavailable — running on read-only fallback.",
          );
        }
        const blocked = writeBlockedReason(readContext);
        if (blocked) {
          providerWarnings.push(blocked);
        }
      }

      const walletSummary: WalletReadModel | null = readContext
        ? {
            context: readContext,
            address: null,
            watchAddresses: [],
            balance: balance ?? null,
            names: [],
            transactions: [],
            lastUpdatedAt: new Date().toISOString(),
            readOnlyReason: writeBlockedReason(readContext),
          }
        : null;

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
        readContext,
        walletSummary,
        providerWarnings,
      };
    },
    staleTime: 30_000,
  });
}

export { statusTone };
