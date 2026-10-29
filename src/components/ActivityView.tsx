import { RefreshCw, Search } from "lucide-react";
import { Icon } from "./ui/Icon";
import { explorerCoversNetwork } from "../lib/openExternal";
import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useActiveProfile } from "../queries/wallet";
import { useMergedActivity } from "../queries/activity";
import { PageHeader } from "./ui/PageHeader";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { nameMatches } from "../lib/idn";
import { useQueryClient } from "@tanstack/react-query";
import { NameActionsModal } from "./NameActionsModal";
import { BlockInfoModal } from "./BlockInfoModal";
import { TxInfoModal } from "./TxInfoModal";
import { PaidSwapOffers } from "./PaidSwapOffers";
import { partitionActivity } from "../lib/activity";
import { subscribeAction } from "../lib/actionBus";
import { ACTION_META } from "./activity/activityText";
import { ToFinishList } from "./activity/ToFinishList";
import { WaitingList } from "./activity/WaitingList";
import { HistoryTable } from "./activity/HistoryTable";

// Kept for existing imports: the vocabulary moved to activity/activityText.ts
// and lib/activity.ts.
export { ACTION_META, FALLBACK_META } from "./activity/activityText";
export { draftActionsForStatus } from "../lib/activity";

const ALL_ACTIONS = Object.keys(ACTION_META);

// Client-side page size for History. The backend returns the whole
// classified history in one call today, so pagination is purely a render
// optimization + navigation affordance (see plan follow-up 2).
const PAGE_SIZE = 50;

export function ActivityView() {
  const [infoName, setInfoName] = useState<string | null>(null);
  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  const [infoTx, setInfoTx] = useState<string | null>(null);
  // The one way a list opens a name (the name page replaces it later).
  const openName = setInfoName;

  // --- Keyboard shortcut subscriptions (action bus) ---
  const searchInputRef = useRef<HTMLInputElement>(null);
  const actionHandlerRef = useRef<(id: string) => void>(() => {});
  actionHandlerRef.current = (actionId: string) => {
    switch (actionId) {
      case "activity:focusSearch":
        searchInputRef.current?.focus();
        break;
    }
  };
  useEffect(() => subscribeAction((id) => actionHandlerRef.current(id)), []);

  const { rows: merged, isLoading, isFetching, isError, error } = useMergedActivity();
  const { data: profile } = useActiveProfile();
  const isMainnet = explorerCoversNetwork(profile?.network);
  const qc = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();

  // Filters from URL params.
  const filterAction = searchParams.get("action") ?? "all";
  const filterStatus = searchParams.get("status") ?? "all";
  const rawQuery = searchParams.get("q") ?? "";
  const searchQuery = rawQuery.toLowerCase();
  const pageParam = Number.parseInt(searchParams.get("page") ?? "1", 10);
  const page = Number.isFinite(pageParam) && pageParam >= 1 ? pageParam : 1;
  const filtering = filterAction !== "all" || filterStatus !== "all" || rawQuery !== "";

  // Set (or clear) a filter param AND reset paging to page 1 in a SINGLE
  // update — two separate setSearchParams calls in one handler race (both
  // read the same base snapshot, so the second clobbers the first).
  const setFilter = (key: string, value: string) => {
    setSearchParams(
      (prev) => {
        if (value === "all" || value === "") prev.delete(key);
        else prev.set(key, value);
        prev.delete("page"); // any filter change → back to page 1
        return prev;
      },
      { replace: true },
    );
  };
  const clearFilters = () => {
    setSearchParams(
      (prev) => {
        for (const k of ["q", "action", "status", "page"]) prev.delete(k);
        return prev;
      },
      { replace: true },
    );
  };

  // Filters apply to all three lists: filter first, then partition.
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
  const { toFinish, waiting, history } = partitionActivity(filtered);

  // Only History is paged.
  const totalRows = history.length;
  const totalPages = Math.max(1, Math.ceil(totalRows / PAGE_SIZE));
  const clampedPage = Math.min(page, totalPages);
  const pageStart = (clampedPage - 1) * PAGE_SIZE;
  const pageEnd = Math.min(pageStart + PAGE_SIZE, totalRows);
  const pageRows = history.slice(pageStart, pageEnd);

  // Two auto-corrections for the ?page param:
  //  1) Filter change shrinks the list past the current page → snap to page 1.
  //  2) URL carries ?page=999 but there are only 3 pages → snap to the last.
  // Both use replace so browser history doesn't fill up with pager churn.
  useEffect(() => {
    if (page > totalPages) {
      setSearchParams(
        (prev) => {
          if (totalPages <= 1) prev.delete("page");
          else prev.set("page", String(totalPages));
          return prev;
        },
        { replace: true },
      );
    }
  }, [page, totalPages, setSearchParams]);

  const goToPage = (n: number) => {
    setSearchParams(
      (prev) => {
        if (n <= 1) prev.delete("page");
        else prev.set("page", String(n));
        return prev;
      },
      { replace: true },
    );
  };

  // Tauri rejects with the command's error as a plain string; a test or a
  // wrapper may hand an `Error`.
  const errorText = error instanceof Error ? error.message : typeof error === "string" ? error : "";
  // `read_action_history` refuses an index-less node with a sentence naming
  // exactly the index it lacks ("address index", "transaction index", or
  // both); the banner names that flag.
  const isIndexError = isError && errorText.includes("not enabled on this hsd node");
  const missingFlags = errorText.includes("address and transaction indexes not enabled")
    ? ["--index-address", "--index-tx"]
    : errorText.includes("transaction index not enabled")
      ? ["--index-tx"]
      : ["--index-address"];

  const noActivity = !isLoading && !isError && merged.length === 0;
  const historyEmptyText =
    filtered.length === 0
      ? "No transactions match the current filters."
      : filtering
        ? "No finished transactions match the current filters."
        : "Nothing finished yet.";

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Activity"
        subtitle="Every transaction and name action from this wallet."
        actions={[
          {
            label: isFetching ? "Working..." : "Refresh",
            icon: RefreshCw,
            variant: "secondary",
            disabled: isFetching,
            onClick: () => {
              qc.invalidateQueries({ queryKey: ["read", "action_history"] });
              // To finish reads drafts.
              qc.invalidateQueries({ queryKey: ["wallet", "drafts"] });
            },
          },
        ]}
      />

      {isIndexError && (
        <div
          className="bg-yellow-50 border border-yellow-200 rounded p-4 text-sm text-yellow-800"
          data-testid="activity-index-banner"
        >
          Your node does not have{" "}
          {missingFlags.map((flag, i) => (
            <span key={flag}>
              {i > 0 && " or "}
              <code className="font-mono">{flag}</code>
            </span>
          ))}{" "}
          enabled. Restart it with <code className="font-mono">--index-address --index-tx</code> to
          enable full history. App-managed nodes already have both enabled.
        </div>
      )}
      {isError && !isIndexError && (
        <div className="bg-red-50 border border-red-200 rounded p-4 text-sm text-red-800">
          Failed to load activity: {errorText || "Unknown error"}
        </div>
      )}

      {isLoading ? (
        <div className="bg-white rounded-md border border-gray-200 py-8 text-center text-gray-500">
          Loading activity...
        </div>
      ) : noActivity ? (
        <div className="bg-white rounded-md border border-gray-200 py-8 text-center text-sm text-gray-400">
          No activity yet on this wallet.
        </div>
      ) : (
        <>
          {/* Drafts are local: these two lists show even when the history read failed. */}
          {toFinish.length > 0 && (
            <ToFinishList
              rows={toFinish}
              profileId={profile?.id ?? null}
              onNameClick={openName}
              onTxClick={setInfoTx}
            />
          )}
          {waiting.length > 0 && (
            <WaitingList rows={waiting} onNameClick={openName} onTxClick={setInfoTx} />
          )}
          {!isError && (
            <HistoryTable
              rows={pageRows}
              allRows={merged}
              resetKey={`${rawQuery}|${filterAction}|${filterStatus}`}
              toolbar={
                <>
                  <span className="relative inline-flex items-center">
                    <Icon
                      icon={Search}
                      size={14}
                      className="pointer-events-none absolute left-2 text-gray-400"
                    />
                    <Input
                      ref={searchInputRef}
                      inputSize="sm"
                      className="w-52 border-gray-200 pl-7"
                      placeholder="Search by name..."
                      value={rawQuery}
                      onChange={(e) => setFilter("q", e.target.value)}
                      data-testid="activity-search-input"
                    />
                  </span>
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
                  {filtering && (
                    <button
                      type="button"
                      className="text-xs text-blue-600 hover:underline"
                      onClick={clearFilters}
                      data-testid="activity-clear-filters"
                    >
                      Clear
                    </button>
                  )}
                </>
              }
              note={
                filtering && (toFinish.length > 0 || waiting.length > 0) ? (
                  <p className="px-4 pt-2 text-xs text-gray-500">
                    Filters apply to To finish and Waiting for a block too.
                  </p>
                ) : null
              }
              emptyText={historyEmptyText}
              pager={
                totalRows > 0
                  ? {
                      page: clampedPage,
                      totalPages,
                      startRow: pageStart + 1,
                      endRow: pageEnd,
                      totalRows,
                      onGoTo: goToPage,
                    }
                  : null
              }
              onNameClick={openName}
              onBlockClick={setInfoBlock}
              onTxClick={setInfoTx}
            />
          )}
        </>
      )}

      <PaidSwapOffers onNameClick={openName} />

      {infoName && (
        <NameActionsModal name={infoName} open={!!infoName} onClose={() => setInfoName(null)} />
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
