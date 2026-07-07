import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useImportFromNamebase } from "../queries/namebase";
import { Button } from "./ui/Button";
import { formatDate } from "../lib/utils";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";
import type { AuditEntry } from "../types";

interface InventoryComparison {
  providerKind: string;
  providerLabel: string;
  matchedTransferable: string[];
  matchedStaked: string[];
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

  const { data: auditLog } = useQuery({
    queryKey: ["audit", "sync"],
    queryFn: () => invoke<AuditEntry[]>("get_audit_log", { limit: 10 }),
    staleTime: 30_000,
  });
  const syncEntries = auditLog?.filter((e) => e.action === "sync") ?? [];

  // The comparison result lives in the query cache (not local state), so it
  // survives navigating away and back. It only runs on demand (enabled: false →
  // refetch() on the button) and never auto-expires during the session.
  const {
    data: report,
    isFetching: loading,
    error,
    refetch,
    dataUpdatedAt,
  } = useQuery({
    queryKey: ["sync", "comparison"],
    queryFn: () => invoke<InventoryComparison>("compare_inventory_with_provider"),
    enabled: false,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  });

  useEffect(() => {
    if (error) showToast(`Compare failed: ${error}`, "error");
  }, [error, showToast]);

  const qc = useQueryClient();
  const importMutation = useImportFromNamebase();

  const handleCompare = () => {
    void refetch();
  };

  const handleImportMissing = async () => {
    try {
      const result = await importMutation.mutateAsync();
      showToast(
        `Imported ${result.imported} domains (${result.staked_imported ?? 0} staked-only added). Refreshing comparison…`,
        "success",
      );
      // Invalidate Sync cache so the next Compare is fresh.
      qc.invalidateQueries({ queryKey: ["sync"] });
      // Auto-refetch the comparison.
      void refetch();
    } catch (e) {
      showToast(mapError(e), "error");
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
        <h2 className="text-xl font-bold">Reconcile with Namebase inventory</h2>
        <Button variant="primary" onClick={handleCompare} disabled={loading}>
          {loading ? "Comparing…" : "Compare inventory"}
        </Button>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600">
        Compares your locally-imported inventory against what Namebase still holds.
        Use the <strong>Namebase</strong> tab to see live Namebase balances and domains.
        Fast, read-only — no statuses are changed.
      </div>

      {report && (
        <div className="space-y-4" data-testid="compare-report">
          {/* Always-visible summary so a completed compare never looks blank. */}
          <div className="bg-white rounded p-4 border border-gray-200 text-sm">
            <div className="text-gray-500 mb-1">
              Source: <strong>{report.providerLabel}</strong>
              {dataUpdatedAt && (
                <span className="ml-3 text-xs text-gray-400" data-testid="compare-timestamp">
                  Last compared: {formatDate(new Date(dataUpdatedAt).toISOString())}
                </span>
              )}
            </div>
            <div className="flex flex-wrap gap-x-4 gap-y-1">
              <span className="text-green-700 font-medium">
                Still at Namebase (transferable): {report.matchedTransferable.length}
              </span>
              <span className="text-purple-700 font-medium">
                Still at Namebase (staked): {report.matchedStaked.length}
              </span>
              <span className="text-yellow-700 font-medium">
                Left Namebase / elsewhere: {report.missingAtProvider.length}
              </span>
              <span className="text-blue-700 font-medium">
                On Namebase only: {report.extraAtProvider.length}
              </span>
            </div>
            {report.matchedTransferable.length === 0 &&
              report.matchedStaked.length === 0 &&
              report.missingAtProvider.length === 0 &&
              report.extraAtProvider.length === 0 && (
                <div className="text-gray-500 mt-2">
                  Nothing to compare yet — import your domains on the Namebase tab first.
                </div>
              )}
            {/* Repair CTA: when Namebase has names that are missing from inventory */}
            {report.extraAtProvider.length > 0 && (
              <div className="mt-3 pt-3 border-t border-gray-200 space-y-2">
                <div className="text-amber-700 text-xs">
                  {report.extraAtProvider.length} name{report.extraAtProvider.length === 1 ? "" : "s"} 
                  {" "}at Namebase {report.matchedStaked.length > 0 ? "and staked-only " : ""}
                  are not yet in your local inventory.
                </div>
                <Button
                  size="sm"
                  variant="primary"
                  onClick={handleImportMissing}
                  disabled={importMutation.isPending}
                >
                  {importMutation.isPending
                    ? "Importing…"
                    : `Add ${report.extraAtProvider.length} missing name${report.extraAtProvider.length === 1 ? "" : "s"}`}
                </Button>
              </div>
            )}
          </div>
          <Section title="Still at Namebase (transferable)" names={report.matchedTransferable} tone="text-green-700" />
          <Section title="Still at Namebase (staked)" names={report.matchedStaked} tone="text-purple-700" />
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
