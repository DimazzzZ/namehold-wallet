import { useEffect } from "react";
import { useSearchParams } from "react-router-dom";
import { useActionHistory } from "../queries/read";
import { useActiveProfile } from "../queries/wallet";
import { PageHeader } from "./ui/PageHeader";
import { Badge } from "./ui/Badge";
import { formatHns, formatDateLong, amountTone } from "../lib/utils";
import { displayName } from "../lib/idn";
import {
  explorerBlockUrl,
  explorerNameUrl,
  explorerTxUrl,
  openExternal,
} from "../lib/openExternal";
import { useQueryClient } from "@tanstack/react-query";
import type { ActionRow } from "../lib/zod";

// Action label + badge variant mapping.
export const ACTION_META: Record<string, { label: string; variant: "default" | "success" | "warning" | "error" | "info" }> = {
  send: { label: "Send", variant: "warning" },
  receive: { label: "Receive", variant: "success" },
  open: { label: "Open", variant: "info" },
  bid: { label: "Bid", variant: "info" },
  reveal: { label: "Reveal", variant: "info" },
  redeem: { label: "Redeem", variant: "success" },
  register: { label: "Register", variant: "success" },
  update: { label: "Update", variant: "default" },
  renew: { label: "Renew", variant: "default" },
  transfer: { label: "Transfer", variant: "warning" },
  finalize: { label: "Finalize", variant: "success" },
  revoke: { label: "Revoke", variant: "error" },
  claim: { label: "Claim", variant: "success" },
  other: { label: "Other", variant: "default" },
};

const ALL_ACTIONS = Object.keys(ACTION_META);

export const FALLBACK_META = { label: "Other", variant: "default" as const };

// Client-side page size for the full Activity table. The backend returns the
// whole classified history in one call today, so pagination is purely a
// render optimization + navigation affordance (see plan follow-up 2).
const PAGE_SIZE = 50;

export function ActivityView() {
  const { data: rows = [], isLoading, isError, error } = useActionHistory();
  const { data: profile } = useActiveProfile();
  const isMainnet = profile?.network === "mainnet";
  const qc = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();

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
          <div className="bg-white rounded border border-gray-200 overflow-auto p-3">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1 pr-4">Date</th>
                  <th className="py-1 pr-4">Action</th>
                  <th className="py-1 pr-4">Name</th>
                  <th className="py-1 pr-4 text-right">Amount</th>
                  <th className="py-1 pr-4">Status</th>
                  <th className="py-1 pr-4">Block</th>
                  <th className="py-1">Txid</th>
                </tr>
              </thead>
              <tbody>
                {pageRows.map((row) => (
                  <ActivityRow key={row.txid} row={row} isMainnet={isMainnet} />
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
    </div>
  );
}

function ActivityRow({
  row,
  isMainnet,
}: {
  row: ActionRow;
  isMainnet: boolean;
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

  const linkClass =
    "text-blue-500 hover:text-blue-700 hover:underline cursor-pointer";

  return (
    <tr className="border-t border-gray-100 hover:bg-gray-50">
      <td className="py-1 pr-4 text-gray-500 whitespace-nowrap">{timeStr}</td>
      <td className="py-1 pr-4">
        <Badge variant={meta.variant}>{meta.label}</Badge>
      </td>
      <td className="py-1 pr-4 text-xs font-mono">
        {row.name ? (
          isMainnet ? (
            <button
              type="button"
              className={linkClass}
              onClick={() => openExternal(explorerNameUrl(row.name!))}
              title="View on explorer"
              data-testid="activity-name-explorer-link"
            >
              .{displayName(row.name)}
            </button>
          ) : (
            <span>.{displayName(row.name)}</span>
          )
        ) : (
          <span className="text-gray-400">—</span>
        )}
      </td>
      <td
        className="py-1 pr-4 text-right text-xs font-mono whitespace-nowrap"
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
      <td className="py-1 pr-4">
        <Badge
          variant={row.confirmed ? "success" : "warning"}
          title={row.height != null ? `Height #${row.height}` : "Mempool"}
        >
          {row.confirmed ? "Confirmed" : "Pending"}
        </Badge>
      </td>
      <td className="py-1 pr-4 text-xs text-gray-500 font-mono">
        {row.height == null ? (
          <span className="text-gray-400">—</span>
        ) : isMainnet ? (
          <button
            type="button"
            className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
            onClick={() => openExternal(explorerBlockUrl(row.height!))}
            title="View block on explorer"
            data-testid="activity-block-explorer-link"
          >
            #{row.height}
          </button>
        ) : (
          <span>#{row.height}</span>
        )}
      </td>
      <td className="py-1 text-xs font-mono text-gray-500" title={row.txid}>
        {isMainnet ? (
          <button
            type="button"
            className="inline-block max-w-[140px] truncate align-bottom text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
            onClick={() => openExternal(explorerTxUrl(row.txid))}
            title="View on explorer"
            data-testid="activity-tx-explorer-link"
          >
            {row.txid.slice(0, 10)}...
          </button>
        ) : (
          <span className="inline-block max-w-[140px] truncate align-bottom">
            {row.txid.slice(0, 10)}...
          </span>
        )}
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
