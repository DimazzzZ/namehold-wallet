import { useNavigate } from "react-router-dom";
import { PageHeader } from "./ui/PageHeader";
import { Card } from "./ui/Card";
import { Badge } from "./ui/Badge";
import { EmptyState } from "./ui/EmptyState";
import { useOverviewData, statusTone } from "../queries/overview";
import { cn } from "../lib/utils";
import type { StatusTone } from "../types";

const METRIC_TONE_RING: Record<StatusTone, string> = {
  default: "ring-gray-200",
  info: "ring-blue-200",
  success: "ring-emerald-200",
  warning: "ring-amber-200",
  error: "ring-red-200",
};

function formatStatusLabel(status: string): string {
  return status.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

export function Overview() {
  const navigate = useNavigate();
  const { data, isLoading, isError } = useOverviewData();

  return (
    <div>
      <PageHeader
        title="Overview"
        subtitle="Operational summary of your Handshake portfolio and infrastructure."
        actions={[
          { label: "View Portfolio", to: "/portfolio", variant: "secondary" },
          { label: "Run Migration", to: "/migration", variant: "primary" },
        ]}
      />

      {isError && (
        <Card className="mb-5">
          <p className="text-sm text-red-600">
            Failed to load overview data. Check that the node and database are
            reachable.
          </p>
        </Card>
      )}

      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
        {(data?.metrics ?? []).map((metric) => (
          <div
            key={metric.key}
            className={cn(
              "bg-white rounded-lg border border-gray-200 p-4 shadow-sm ring-1",
              METRIC_TONE_RING[metric.tone ?? "default"],
            )}
          >
            <div className="text-xs font-medium text-gray-500">
              {metric.label}
            </div>
            <div className="text-2xl font-semibold text-gray-900 mt-1">
              {isLoading ? "…" : metric.value}
            </div>
            {metric.hint && (
              <div className="text-[11px] text-gray-400 mt-1">{metric.hint}</div>
            )}
          </div>
        ))}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <Card title="Status Breakdown" subtitle="TLDs by migration status">
          {data && Object.keys(data.statusCounts).length > 0 ? (
            <ul className="space-y-2">
              {Object.entries(data.statusCounts)
                .sort((a, b) => b[1] - a[1])
                .map(([status, count]) => (
                  <li
                    key={status}
                    className="flex items-center justify-between"
                  >
                    <Badge variant={statusTone(status)}>
                      {formatStatusLabel(status)}
                    </Badge>
                    <span className="text-sm font-medium text-gray-700">
                      {count}
                    </span>
                  </li>
                ))}
            </ul>
          ) : (
            <EmptyState
              title="No TLDs yet"
              description="Import names from Namebase to populate your portfolio."
              actions={[
                {
                  label: "Go to Migration",
                  variant: "primary",
                  onClick: () => navigate("/migration"),
                },
              ]}
            />
          )}
        </Card>

        <Card title="Recent Activity" subtitle="Latest audit log entries">
          {data && data.recentAudit.length > 0 ? (
            <ul className="divide-y divide-gray-100">
              {data.recentAudit.map((entry) => (
                <li key={entry.id} className="py-2 flex items-start gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="text-sm text-gray-800">
                      <span className="font-medium">{entry.action}</span>
                      {entry.entity && (
                        <span className="text-gray-500">
                          {" "}
                          · {entry.entity}
                        </span>
                      )}
                    </div>
                    {entry.detail && (
                      <div className="text-xs text-gray-500 truncate">
                        {entry.detail}
                      </div>
                    )}
                  </div>
                  <time className="text-[11px] text-gray-400 shrink-0">
                    {new Date(entry.created_at).toLocaleString()}
                  </time>
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState
              title="No activity"
              description="Actions you take will be recorded here."
            />
          )}
        </Card>
      </div>
    </div>
  );
}
