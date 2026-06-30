import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { Button } from "./ui/Button";
import { formatDate } from "../lib/utils";
import { useUiStore } from "../stores/ui";
import type { AuditEntry } from "../types";

interface InventoryComparison {
  providerKind: string;
  providerLabel: string;
  matched: string[];
  missingAtProvider: string[];
  extraAtProvider: string[];
}

/**
 * Reconciles your local inventory (imported TLDs) against the names Namebase
 * still lists for your account. One bulk call — fast and read-only; it does not
 * change any statuses.
 */
export function SyncVerification() {
  const showToast = useUiStore((s) => s.showToast);
  const [report, setReport] = useState<InventoryComparison | null>(null);
  const [loading, setLoading] = useState(false);

  const { data: auditLog } = useQuery({
    queryKey: ["audit", "sync"],
    queryFn: () => invoke<AuditEntry[]>("get_audit_log", { limit: 10 }),
    staleTime: 30_000,
  });
  const syncEntries = auditLog?.filter((e) => e.action === "sync") ?? [];

  const handleCompare = async () => {
    setLoading(true);
    try {
      const r = await invoke<InventoryComparison>("compare_inventory_with_provider");
      setReport(r);
    } catch (e) {
      showToast(`Compare failed: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const Section = ({ title, names, tone }: { title: string; names: string[]; tone: string }) =>
    names.length > 0 ? (
      <div className="bg-white rounded p-4 border border-gray-200">
        <h4 className={`text-sm font-semibold mb-2 ${tone}`}>
          {title} ({names.length})
        </h4>
        <div className="max-h-40 overflow-auto">
          {names.map((n) => (
            <div key={n} className="text-sm font-mono py-0.5">.{n}</div>
          ))}
        </div>
      </div>
    ) : null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Verify against Namebase</h2>
        <Button variant="primary" onClick={handleCompare} disabled={loading}>
          {loading ? "Comparing…" : "Compare inventory"}
        </Button>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600">
        Reconciles your imported inventory against the domains Namebase still
        lists for your account. Fast, read-only — no statuses are changed.
      </div>

      {report && (
        <div className="space-y-4" data-testid="compare-report">
          {/* Always-visible summary so a completed compare never looks blank. */}
          <div className="bg-white rounded p-4 border border-gray-200 text-sm">
            <div className="text-gray-500 mb-1">
              Source: <strong>{report.providerLabel}</strong>
            </div>
            <div className="flex flex-wrap gap-x-4 gap-y-1">
              <span className="text-green-700 font-medium">
                Still at Namebase: {report.matched.length}
              </span>
              <span className="text-yellow-700 font-medium">
                Left Namebase / elsewhere: {report.missingAtProvider.length}
              </span>
              <span className="text-blue-700 font-medium">
                On Namebase only: {report.extraAtProvider.length}
              </span>
            </div>
            {report.matched.length === 0 &&
              report.missingAtProvider.length === 0 &&
              report.extraAtProvider.length === 0 && (
                <div className="text-gray-500 mt-2">
                  Nothing to compare yet — import your domains on the Namebase tab first.
                </div>
              )}
          </div>
          <Section title="Still at Namebase" names={report.matched} tone="text-green-700" />
          <Section
            title="In inventory, not on Namebase (left / transferred out)"
            names={report.missingAtProvider}
            tone="text-yellow-700"
          />
          <Section
            title="On Namebase, not in your inventory"
            names={report.extraAtProvider}
            tone="text-blue-700"
          />
        </div>
      )}

      {syncEntries.length > 0 && (
        <div className="bg-white rounded p-4 border border-gray-200">
          <h3 className="text-sm font-semibold mb-3">History</h3>
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
    </div>
  );
}
