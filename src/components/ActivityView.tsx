import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useActionHistory } from "../queries/read";
import { PageHeader } from "./ui/PageHeader";
import { Badge } from "./ui/Badge";
import { NameActionsModal } from "./NameActionsModal";
import { formatHns, formatDateLong, amountTone } from "../lib/utils";
import { displayName } from "../lib/idn";
import { useQueryClient } from "@tanstack/react-query";
import type { ActionRow } from "../lib/zod";

// Action label + badge variant mapping.
const ACTION_META: Record<string, { label: string; variant: "default" | "success" | "warning" | "error" | "info" }> = {
  send: { label: "Send", variant: "warning" },
  receive: { label: "Receive", variant: "success" },
  open: { label: "OPEN", variant: "info" },
  bid: { label: "BID", variant: "info" },
  reveal: { label: "REVEAL", variant: "info" },
  redeem: { label: "REDEEM", variant: "success" },
  register: { label: "REGISTER", variant: "success" },
  update: { label: "UPDATE", variant: "default" },
  renew: { label: "RENEW", variant: "default" },
  transfer: { label: "TRANSFER", variant: "warning" },
  finalize: { label: "FINALIZE", variant: "success" },
  revoke: { label: "REVOKE", variant: "error" },
  claim: { label: "CLAIM", variant: "success" },
  other: { label: "Other", variant: "default" },
};

const ALL_ACTIONS = Object.keys(ACTION_META);

const FALLBACK_META = { label: "Other", variant: "default" as const };

// Client-side page size for the full Activity table. The backend returns the
// whole classified history in one call today, so pagination is purely a
// render optimization + navigation affordance (see plan follow-up 2).
const PAGE_SIZE = 50;

export function ActivityView() {
  const { data: rows = [], isLoading, isError, error } = useActionHistory();
  const qc = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const [manageName, setManageName] = useState<string | null>(null);

  // Filters from URL params.
  const filterAction = searchParams.get("action") ?? "all";
  const filterStatus = searchParams.get("status") ?? "all";
  const searchQuery = (searchParams.get("q") ?? "").toLowerCase();
  const pageParam = Number.parseInt(searchParams.get("page") ?? "1", 10);
  const page = Number.isFinite(pageParam) && pageParam >= 1 ? pageParam : 1;

  // Set (or clear) a filter param AND reset paging to page 1 in a SINGLE
  // update — two separate setSearchParams calls in one handler race (both
  // read the same base snapshot, so the second clobbers the first).
  const setFilter = (key: string, value: string) => {
    setSearchParams((prev) => {
      if (value === "all" || value === "") prev.delete(key);
      else prev.set(key, value);
      prev.delete("page"); // any filter change → back to page 1
      return prev;
    }, { replace: true });
  };

  // Apply filters.
  const filtered = rows.filter((r) => {
    if (filterAction !== "all" && r.action !== filterAction) return false;
    if (filterStatus === "confirmed" && !r.confirmed) return false;
    if (filterStatus === "pending" && r.confirmed) return false;
    if (searchQuery && !(r.name ?? "").toLowerCase().includes(searchQuery)) return false;
    return true;
  });

  // Pagination math derived from the filtered list.
  const totalRows = filtered.length;
  const totalPages = Math.max(1, Math.ceil(totalRows / PAGE_SIZE));
  const clampedPage = Math.min(page, totalPages);
  const pageStart = (clampedPage - 1) * PAGE_SIZE;
  const pageEnd = Math.min(pageStart + PAGE_SIZE, totalRows);
  const pageRows = filtered.slice(pageStart, pageEnd);

  // Two auto-corrections for the ?page param:
  //  1) Filter change shrinks the list past the current page → snap to page 1.
  //  2) URL carries ?page=999 but there are only 3 pages → snap to the last.
  // Both use replace so browser history doesn't fill up with pager churn.
  useEffect(() => {
    if (page > totalPages) {
      setSearchParams((prev) => {
        if (totalPages <= 1) prev.delete("page");
        else prev.set("page", String(totalPages));
        return prev;
      }, { replace: true });
    }
  }, [page, totalPages, setSearchParams]);

  const goToPage = (n: number) => {
    setSearchParams((prev) => {
      if (n <= 1) prev.delete("page");
      else prev.set("page", String(n));
      return prev;
    }, { replace: true });
  };

  const isIndexError =
    isError && error instanceof Error && error.message.includes("address index not enabled");

  return (
    <div>
      <PageHeader
        title="Activity"
        subtitle="Every transaction and name action from this wallet."
        actions={[
          {
            label: "Refresh",
            variant: "ghost",
            loading: isLoading,
            onClick: () => qc.invalidateQueries({ queryKey: ["read", "action_history"] }),
          },
        ]}
      />

      {/* Toolbar: search + action chips + status filter */}
      <div className="flex flex-wrap items-center gap-2 mb-4">
        <input
          type="text"
          placeholder="Search by name..."
          className="border border-gray-200 rounded px-2 py-1 text-sm w-48"
          value={searchParams.get("q") ?? ""}
          onChange={(e) => setFilter("q", e.target.value)}
        />
        <select
          className="border border-gray-200 rounded px-2 py-1 text-sm"
          value={filterAction}
          onChange={(e) => setFilter("action", e.target.value)}
        >
          <option value="all">All actions</option>
          {ALL_ACTIONS.map((a) => (
            <option key={a} value={a}>{ACTION_META[a]?.label ?? a}</option>
          ))}
        </select>
        <select
          className="border border-gray-200 rounded px-2 py-1 text-sm"
          value={filterStatus}
          onChange={(e) => setFilter("status", e.target.value)}
        >
          <option value="all">All statuses</option>
          <option value="confirmed">Confirmed</option>
          <option value="pending">Pending</option>
        </select>
      </div>

      {/* Error states */}
      {isIndexError && (
        <div className="bg-yellow-50 border border-yellow-200 rounded p-4 mb-4 text-sm text-yellow-800">
          Your node does not have <code className="font-mono">--index-address</code> enabled.
          Restart it with <code className="font-mono">--index-address --index-tx</code> to
          enable full history. App-managed nodes already have both enabled.
        </div>
      )}
      {isError && !isIndexError && (
        <div className="bg-red-50 border border-red-200 rounded p-4 mb-4 text-sm text-red-800">
          Failed to load activity: {error instanceof Error ? error.message : "Unknown error"}
        </div>
      )}

      {/* Table */}
      {isLoading ? (
        <div className="text-gray-500 py-8 text-center">Loading activity...</div>
      ) : filtered.length === 0 ? (
        <div className="text-gray-400 text-sm py-8 text-center bg-white rounded border border-gray-200">
          {rows.length === 0
            ? "No activity yet on this wallet."
            : "No transactions match the current filters."}
        </div>
      ) : (
        <>
          <div className="bg-white rounded border border-gray-200 overflow-auto">
            <table className="w-full text-sm text-gray-700">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="px-3 py-2 font-medium">Date</th>
                  <th className="px-3 py-2 font-medium">Action</th>
                  <th className="px-3 py-2 font-medium">Name</th>
                  <th className="px-3 py-2 font-medium text-right">Amount</th>
                  <th className="px-3 py-2 font-medium">Status</th>
                  <th className="px-3 py-2 font-medium">Txid</th>
                </tr>
              </thead>
              <tbody>
                {pageRows.map((row) => (
                  <ActivityRow
                    key={row.txid}
                    row={row}
                    onNameClick={setManageName}
                  />
                ))}
              </tbody>
            </table>
          </div>
          {totalRows > PAGE_SIZE && (
            <Pager
              page={clampedPage}
              totalPages={totalPages}
              startRow={pageStart + 1}
              endRow={pageEnd}
              totalRows={totalRows}
              onGoTo={goToPage}
            />
          )}
        </>
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

function ActivityRow({
  row,
  onNameClick,
}: {
  row: ActionRow;
  onNameClick: (name: string) => void;
}) {
  const meta = ACTION_META[row.action] ?? FALLBACK_META;
  const timeStr = row.time
    ? formatDateLong(new Date(row.time * 1000).toISOString())
    : "Pending";
  const tone = amountTone(row);
  const toneClass =
    tone === "income"
      ? "text-green-600"
      : tone === "spend"
      ? "text-red-600"
      : "text-gray-700";
  const sign = tone === "income" ? "+" : tone === "spend" ? "-" : "";

  return (
    <tr className="border-t border-gray-100 hover:bg-gray-50">
      <td className="px-3 py-2 text-gray-500 whitespace-nowrap">{timeStr}</td>
      <td className="px-3 py-2">
        <Badge variant={meta.variant}>{meta.label}</Badge>
      </td>
      <td className="px-3 py-2">
        {row.name ? (
          <button
            className="text-blue-600 hover:underline"
            onClick={() => onNameClick(row.name!)}
          >
            .{displayName(row.name)}
          </button>
        ) : (
          <span className="text-gray-400">—</span>
        )}
      </td>
      <td
        className="px-3 py-2 text-right font-mono whitespace-nowrap"
        title={
          row.valueDoos === 0 && row.direction !== "receive"
            ? "Name's locked value is re-homed to your own coin — no HNS spent beyond the fee."
            : undefined
        }
      >
        <span className={toneClass}>
          {sign}
          {formatHns(Math.abs(row.valueDoos))}
        </span>
      </td>
      <td className="px-3 py-2">
        <Badge variant={row.confirmed ? "success" : "warning"} title={row.height != null ? `Height #${row.height}` : "Mempool"}>
          {row.confirmed ? "Confirmed" : "Pending"}
        </Badge>
      </td>
      <td className="px-3 py-2 font-mono text-gray-500 truncate max-w-[140px]" title={row.txid}>
        {row.txid.slice(0, 10)}...
      </td>
    </tr>
  );
}

function Pager({
  page,
  totalPages,
  startRow,
  endRow,
  totalRows,
  onGoTo,
}: {
  page: number;
  totalPages: number;
  startRow: number;
  endRow: number;
  totalRows: number;
  onGoTo: (n: number) => void;
}) {
  const canPrev = page > 1;
  const canNext = page < totalPages;
  const btn =
    "text-blue-600 hover:underline disabled:opacity-40 disabled:cursor-not-allowed disabled:no-underline";
  return (
    <div className="flex items-center justify-between mt-3 text-sm text-gray-500">
      <div>
        Rows {startRow}–{endRow} of {totalRows}
      </div>
      <div className="flex items-center gap-4">
        <button
          type="button"
          className={btn}
          disabled={!canPrev}
          onClick={() => onGoTo(page - 1)}
        >
          ← Prev
        </button>
        <span className="text-gray-600">
          Page {page} of {totalPages}
        </span>
        <button
          type="button"
          className={btn}
          disabled={!canNext}
          onClick={() => onGoTo(page + 1)}
        >
          Next →
        </button>
      </div>
    </div>
  );
}
