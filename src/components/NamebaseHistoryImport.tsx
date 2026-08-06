import { useMemo, useState } from "react";
import { open } from "../lib/dialog";
import {
  useImportNamebaseHistoryFile,
  useImportNamebaseHistoryLive,
  useNamebaseHistory,
  useNamebaseHistorySummary,
  useClearNamebaseHistory,
  useNamebaseStatus,
} from "../queries/namebase";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { EmptyState } from "./ui/EmptyState";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";
import { formatHns, formatDate } from "../lib/utils";
import { displayName } from "../lib/idn";

/**
 * "Account history (imported)" card inside the Move-from-Namebase screen.
 *
 * Displays imported Namebase account-history events (bids, fees, sales, etc.)
 * with a clear "imported — not on-chain" label to distinguish them from
 * node-derived data. Two import sources: live fetch from Namebase (requires
 * connection) and local CSV upload (works offline).
 *
 * The data stored is a one-shot historical artifact — Namebase stopped
 * recording activity on 2026-06-12.
 */
export function NamebaseHistoryImport() {
  const showToast = useUiStore((s) => s.showToast);
  const { data: status } = useNamebaseStatus();
  const isConnected = !!status?.connected;
  const { data: summary } = useNamebaseHistorySummary();
  const [showTable, setShowTable] = useState(false);
  const [familyFilter, setFamilyFilter] = useState<string>("");
  const [nameFilter, setNameFilter] = useState<string>("");
  const filters = useMemo(
    () => ({
      family: familyFilter || undefined,
      search: nameFilter || undefined,
    }),
    [familyFilter, nameFilter],
  );
  const { data: rows = [] } = useNamebaseHistory(showTable ? filters : undefined);

  const importFile = useImportNamebaseHistoryFile();
  const importLive = useImportNamebaseHistoryLive();
  const clearAll = useClearNamebaseHistory();

  const handleUpload = async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!selected) return;
    try {
      const result = await importFile.mutateAsync(selected as string);
      showToast(
        `Imported ${result.inserted} new + ${result.updated} updated (${result.total} total)`,
        "success",
      );
    } catch (e) {
      showToast(`Import failed: ${mapError(e)}`, "error");
    }
  };

  const handleFetchLive = async () => {
    try {
      const result = await importLive.mutateAsync();
      showToast(
        `Fetched ${result.inserted} new + ${result.updated} updated (${result.total} total)`,
        "success",
      );
    } catch (e) {
      const msg = mapError(e);
      // Detect rate-limit error: "Namebase rate limit exceeded (retry after Ns)"
      const rateLimitMatch = msg.match(/retry after (\d+)s/i);
      if (rateLimitMatch) {
        const secs = Number(rateLimitMatch[1]);
        showToast(
          `Namebase is rate-limiting exports — try again in ~${secs} seconds.`,
          "info",
        );
      } else {
        showToast(`Fetch failed: ${msg}`, "error");
      }
    }
  };

  const handleClear = async () => {
    if (!confirm("Wipe all imported Namebase history? This does not affect any on-chain data.")) {
      return;
    }
    try {
      const removed = await clearAll.mutateAsync();
      showToast(`Cleared ${removed} imported rows`, "success");
    } catch (e) {
      showToast(`Clear failed: ${mapError(e)}`, "error");
    }
  };

  const hasData = (summary?.eventCount ?? 0) > 0;
  const totalFeeHns = formatHns(summary?.totalFeeDoos ?? 0);
  const totalUsd =
    ((summary?.totalUsdCents ?? 0) / 100).toLocaleString("en-US", {
      style: "currency",
      currency: "USD",
    });

  return (
    <div className="bg-white rounded border border-gray-200 p-4 space-y-3">
      <div className="flex items-center justify-between flex-wrap gap-2">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold">Account history (imported)</h3>
          <Badge variant="default" title="Imported from your Namebase account export — not derived from on-chain data.">
            Imported — not on-chain data
          </Badge>
        </div>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="ghost"
            onClick={handleFetchLive}
            disabled={!isConnected || importLive.isPending}
            title={
              isConnected
                ? "Fetch fresh history from Namebase"
                : "Connect to Namebase to enable live fetch"
            }
          >
            {importLive.isPending ? "Fetching…" : "Fetch from Namebase"}
          </Button>
          <Button
            size="sm"
            onClick={handleUpload}
            disabled={importFile.isPending}
          >
            {importFile.isPending ? "Importing…" : "Upload CSV"}
          </Button>
        </div>
      </div>

      <p className="text-xs text-gray-500">
        One-shot import of your Namebase account-history export (bids, fees, sales, deposits).
        Namebase stopped recording activity on 2026-06-12. Data here is historical only and
        is stored separately from on-chain wallet data.
      </p>

      {hasData ? (
        <>
          <div className="grid grid-cols-2 md:grid-cols-5 gap-3 text-xs">
            <SummaryStat label="Events" value={summary!.eventCount.toLocaleString()} />
            <SummaryStat label="Names covered" value={summary!.nameCount.toLocaleString()} />
            <SummaryStat label="Namebase fees (HNS)" value={totalFeeHns} />
            <SummaryStat label="USD sale proceeds" value={totalUsd} />
            <SummaryStat
              label="Date range"
              value={
                summary!.earliest && summary!.latest
                  ? `${formatDate(summary!.earliest)} → ${formatDate(summary!.latest)}`
                  : "—"
              }
            />
          </div>
          <div className="flex items-center justify-between pt-1">
            <Button size="sm" variant="ghost" onClick={() => setShowTable((v) => !v)}>
              {showTable ? "Hide details" : "Show details"}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={handleClear}
              disabled={clearAll.isPending}
              title="Delete all imported history rows"
            >
              {clearAll.isPending ? "Clearing…" : "Clear imported history"}
            </Button>
          </div>

          {showTable && (
            <div className="space-y-2">
              <div className="flex flex-wrap gap-2">
                <Input
                  inputSize="sm"
                  className="w-40 border-gray-200"
                  placeholder="Filter by name…"
                  value={nameFilter}
                  onChange={(e) => setNameFilter(e.target.value)}
                />
                <Select
                  inputSize="sm"
                  className="border-gray-200"
                  options={[
                    { value: "", label: "All families" },
                    { value: "auctions", label: "Auctions" },
                    { value: "subdomains", label: "Subdomains" },
                    { value: "marketplace", label: "Marketplace" },
                    { value: "wallet", label: "Wallet" },
                    { value: "misc", label: "Misc" },
                    { value: "matching-engine", label: "Exchange" },
                  ]}
                  value={familyFilter}
                  onChange={(e) => setFamilyFilter(e.target.value)}
                />
                <span className="text-xs text-gray-500 self-center">
                  {rows.length.toLocaleString()} rows
                </span>
              </div>
              <div className="overflow-auto max-h-[400px] border border-gray-100 rounded">
                <table className="w-full text-xs text-gray-700">
                  <thead className="sticky top-0 bg-gray-50">
                    <tr className="text-left text-gray-500 border-b">
                      <th className="px-2 py-1 font-medium">Date</th>
                      <th className="px-2 py-1 font-medium">Family</th>
                      <th className="px-2 py-1 font-medium">Verb</th>
                      <th className="px-2 py-1 font-medium">Name</th>
                      <th className="px-2 py-1 font-medium text-right">Bid</th>
                      <th className="px-2 py-1 font-medium text-right">Stake</th>
                      <th className="px-2 py-1 font-medium text-right">Fee</th>
                      <th className="px-2 py-1 font-medium text-right">USD</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.slice(0, 500).map((r) => (
                      <tr key={r.id} className="border-t border-gray-100">
                        <td className="px-2 py-1 whitespace-nowrap text-gray-500">
                          {formatDate(r.createdAt)}
                        </td>
                        <td className="px-2 py-1">{r.family}</td>
                        <td className="px-2 py-1">{r.verb}</td>
                        <td className="px-2 py-1">
                          {r.name ? `.${displayName(r.name)}` : <span className="text-gray-400">—</span>}
                        </td>
                        <td className="px-2 py-1 text-right font-mono">
                          {r.bidDoos != null ? formatHns(r.bidDoos) : "—"}
                        </td>
                        <td className="px-2 py-1 text-right font-mono">
                          {r.stakeDoos != null ? formatHns(r.stakeDoos) : "—"}
                        </td>
                        <td className="px-2 py-1 text-right font-mono">
                          {r.feeDoos != null ? formatHns(r.feeDoos) : "—"}
                        </td>
                        <td className="px-2 py-1 text-right font-mono">
                          {r.usdCents != null
                            ? `$${(r.usdCents / 100).toFixed(2)}`
                            : "—"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {rows.length > 500 && (
                <p className="text-xs text-gray-400 text-center">
                  Showing first 500 of {rows.length.toLocaleString()} matching rows.
                </p>
              )}
            </div>
          )}
        </>
      ) : (
        <EmptyState
          title="No history imported yet"
          description="Upload your Namebase account-history CSV export, or fetch it live if connected."
        />
      )}
    </div>
  );
}

function SummaryStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-gray-500">{label}</div>
      <div className="font-semibold text-gray-800 text-sm">{value}</div>
    </div>
  );
}
