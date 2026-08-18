import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useActionHistory } from "../queries/read";
import { useActiveProfile, useTxDrafts } from "../queries/wallet";
import { PageHeader } from "./ui/PageHeader";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { formatHns, formatDate, amountTone } from "../lib/utils";
import { displayName, nameMatches } from "../lib/idn";
import { useQueryClient } from "@tanstack/react-query";
import { NameInfoModal } from "./NameInfoModal";
import { BlockInfoModal } from "./BlockInfoModal";
import { TxInfoModal } from "./TxInfoModal";
import { mergeActivity, type MergedRow } from "../lib/activity";

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
  const [infoName, setInfoName] = useState<string | null>(null);
  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  const [infoTx, setInfoTx] = useState<string | null>(null);
  const { data: rows = [], isLoading, isError, error } = useActionHistory();
  const { data: drafts = [] } = useTxDrafts();
  const { data: profile } = useActiveProfile();
  const isMainnet = profile?.network === "mainnet";
  const qc = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();

  // Merge on-chain history with local drafts, deduped by txid.
  const merged = mergeActivity(rows, drafts);

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
  const filtered = merged.filter((r) => {
    if (filterAction !== "all" && r.action !== filterAction) return false;
    if (filterStatus === "confirmed" && !r.confirmed) return false;
    if (filterStatus === "pending" && r.confirmed) return false;
    // Match both the raw punycode name and the decoded Unicode form the
    // user actually sees (`displayName`), so searching "сбер" finds a row
    // stored as `xn--90ai7ab`.
    if (searchQuery && !nameMatches(r.name, searchQuery)) return false;
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
        <Input
          inputSize="sm"
          className="w-48 border-gray-200"
          placeholder="Search by name..."
          value={searchParams.get("q") ?? ""}
          onChange={(e) => setFilter("q", e.target.value)}
        />
        <Select
          inputSize="sm"
          className="border-gray-200"
          options={[
            { value: "all", label: "All actions" },
            ...ALL_ACTIONS.map((a) => ({
              value: a,
              label: ACTION_META[a]?.label ?? a,
            })),
          ]}
          value={filterAction}
          onChange={(e) => setFilter("action", e.target.value)}
        />
        <Select
          inputSize="sm"
          className="border-gray-200"
          options={[
            { value: "all", label: "All statuses" },
            { value: "confirmed", label: "Confirmed" },
            { value: "pending", label: "Pending" },
          ]}
          value={filterStatus}
          onChange={(e) => setFilter("status", e.target.value)}
        />
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
          {merged.length === 0
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
                  <th className="py-1 pr-4 text-right">Fee</th>
                  <th className="py-1 pr-4">Status</th>
                  <th className="py-1 pr-4">Block</th>
                  <th className="py-1">Txid</th>
                </tr>
              </thead>
              <tbody>
              {pageRows.map((row) => (
                <ActivityRow
                  key={row.key}
                  row={row}
                  onNameClick={setInfoName}
                  onBlockClick={setInfoBlock}
                  onTxClick={setInfoTx}
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

      {infoName && (
        <NameInfoModal
          name={infoName}
          open={!!infoName}
          onClose={() => setInfoName(null)}
        />
      )}

      {infoBlock != null && (
        <BlockInfoModal
          height={infoBlock}
          open={infoBlock != null}
          onClose={() => setInfoBlock(null)}
          isMainnet={isMainnet}
        />
      )}

      {infoTx != null && (
        <TxInfoModal
          txid={infoTx}
          open={infoTx != null}
          onClose={() => setInfoTx(null)}
          isMainnet={isMainnet}
        />
      )}
    </div>
  );
}

/**
 * Map a MergedRow status to a badge variant and display label.
 */
function statusBadge(status: string): {
  variant: "default" | "success" | "warning" | "error" | "info";
  label: string;
} {
  if (status === "onchain") {
    // Handled by the caller (confirmed/pending badge).
    return { variant: "default", label: "Onchain" };
  }
  if (status === "confirmed") {
    return { variant: "success", label: "Confirmed" };
  }
  if (status === "broadcasted" || status === "broadcast_pending") {
    return { variant: "warning", label: "Pending" };
  }
  if (status === "dropped") {
    return { variant: "error", label: "Not confirmed" };
  }
  if (status === "failed") {
    return { variant: "error", label: "Failed" };
  }
  // draft, signed, etc.
  return { variant: "default", label: status };
}

export function ActivityRow({
  row,
  onNameClick,
  onBlockClick,
  onTxClick,
}: {
  row: MergedRow;
  onNameClick: (name: string) => void;
  onBlockClick: (height: number) => void;
  onTxClick: (txid: string) => void;
}) {
  const meta = ACTION_META[row.action] ?? FALLBACK_META;
  const timeStr = row.sortTs > 0
    ? formatDate(new Date(row.sortTs * 1000).toISOString())
    : "Pending";
  const tone = amountTone(row);
  const toneClass =
    tone === "income"
      ? "text-green-600"
      : tone === "spend"
      ? "text-red-600"
      : "text-gray-700";
  const sign = tone === "income" ? "+" : tone === "spend" ? "-" : "";

  const badge = statusBadge(row.status);
  // For onchain-only rows, use the confirmed/pending badge; for drafts,
  // use the status badge.
  const badgeVariant =
    row.status === "onchain"
      ? row.confirmed
        ? "success"
        : "warning"
      : badge.variant;
  const badgeLabel =
    row.status === "onchain"
      ? row.confirmed
        ? "Confirmed"
        : "Pending"
      : badge.label;

  const linkClass =
    "text-blue-500 hover:text-blue-700 hover:underline cursor-pointer";

  return (
    <tr className="border-t border-gray-100 hover:bg-gray-50">
      <td className="py-1 pr-4 text-xs text-gray-500 whitespace-nowrap">{timeStr}</td>
      <td className="py-1 pr-4">
        <Badge variant={meta.variant}>{meta.label}</Badge>
      </td>
      <td className="py-1 pr-4 text-xs font-mono">
        {row.nameList && row.nameList.length > 1 ? (
          // Batch action: render composite label as plain text + list of real names
          <div className="space-y-1">
            <div className="text-gray-600 italic">
              .{displayName(row.name ?? "")}
            </div>
            <ul className="text-xs space-y-0.5">
              {row.nameList.map((n) => (
                <li key={n}>
                  <button
                    type="button"
                    className={linkClass}
                    onClick={() => onNameClick(n)}
                    title="View name info"
                    data-testid="activity-name-info-link"
                  >
                    .{displayName(n)}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ) : row.name ? (
          <button
            type="button"
            className={linkClass}
            onClick={() => onNameClick(row.name!)}
            title="View name info"
            data-testid="activity-name-info-link"
          >
            .{displayName(row.name)}
          </button>
        ) : (
          <span className="text-gray-400">—</span>
        )}
      </td>
      <td
        className="py-1 pr-4 text-right text-xs font-mono whitespace-nowrap"
        title={
          row.nameValueDoos != null
            ? `Name value ${formatHns(row.nameValueDoos)} HNS is carried to your own new coin — not spent; only the fee applies.`
            : row.valueDoos === 0 && row.direction !== "receive"
            ? "Name's locked value is re-homed to your own coin — no HNS spent beyond the fee."
            : undefined
        }
      >
        <span className={toneClass}>
          {sign}
          {formatHns(Math.abs(row.valueDoos))}
        </span>
      </td>
      <td className="py-1 pr-4 text-right text-xs font-mono text-gray-500">
        {row.feeDoos == null ? "—" : formatHns(row.feeDoos)}
      </td>
      <td className="py-1 pr-4">
        <Badge
          variant={badgeVariant}
          title={row.height != null ? `Height #${row.height}` : "Mempool"}
        >
          {badgeLabel}
        </Badge>
      </td>
      <td className="py-1 pr-4 text-xs text-gray-500 font-mono">
        {row.height == null ? (
          <span className="text-gray-400">—</span>
        ) : (
          <button
            type="button"
            className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
            onClick={() => onBlockClick(row.height!)}
            title="View block info"
            data-testid="activity-block-info-link"
          >
            #{row.height}
          </button>
        )}
      </td>
      <td className="py-1 text-xs font-mono text-gray-500" title={row.txid ?? undefined}>
        <button
          type="button"
          className="inline-block max-w-[140px] truncate align-bottom text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
          onClick={() => row.txid && onTxClick(row.txid)}
          title="View transaction info"
          data-testid="activity-tx-info-link"
          disabled={!row.txid}
        >
          {row.txid ? `${row.txid.slice(0, 10)}…` : "—"}
        </button>
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
