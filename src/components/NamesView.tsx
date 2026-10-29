import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  ArrowUpRight,
  CircleCheck,
  Download,
  Ellipsis,
  Gavel,
  RefreshCw,
  Search,
  Send,
  Settings2,
  Signature,
  Star,
  StarOff,
  Tag,
  Upload,
  X,
} from "lucide-react";
import {
  useActiveProfile,
  useSignerSession,
  useTxDrafts,
  useWriteCapability,
} from "../queries/wallet";
import {
  useAuctionPositions,
  useNameActionCapabilities,
  useReadNames,
  useReadRenewals,
} from "../queries/read";
import { useNeedsYou } from "../queries/needsYou";
import {
  exportWatchlistCsv,
  importWatchlistCsv,
  useAddToWatchlist,
  useRemoveFromWatchlist,
  useUpdateWatchlistTags,
  useWatchlist,
  useWatchlistLiveInfo,
  useWatchlistStatuses,
} from "../queries/watchlist";
import { buildNameRows, chipCounts, wallClock, type NameCat, type NameRow } from "../lib/nameRows";
import { nameRowMenu, type RowMenuItem } from "../lib/nameRowMenu";
import {
  activeFilterCount,
  NATURAL_DIR,
  NO_FILTERS,
  namesParams,
  parseNamesParams,
  passesFilters,
  sortRows,
  type NameFilters,
  type NameSort,
  type NamesViewState,
  type SortDir,
} from "../lib/nameFilters";
import {
  CHAR_LABELS,
  expiresLabels,
  LENGTH_LABELS,
  NameFiltersPopover,
} from "./names/NameFiltersPopover";
import { auctionPhase, formatCountdown, taskStateLabel } from "../lib/auction";
import { displayName, nameMatches } from "../lib/idn";
import { icannBadgeTitle } from "../lib/icann";
import { fromInteractiveChild } from "../lib/rowClick";
import { isSignerUnlockedFor } from "../lib/signer";
import { normalizeNameInputAce } from "../lib/utils";
import { subscribeAction } from "../lib/actionBus";
import { useNameBatchActions } from "../hooks/useNameBatchActions";
import { useUiStore } from "../stores/ui";
import { finalizeSentIds, usePurchaseFinalize } from "./market/usePurchase";
import { NAME_TONE_DOT, canUseShakedex, shakedexRefusal } from "./market/marketText";
import { useOpenName } from "../hooks/useOpenName";
import { BatchBidModal } from "./BatchBidModal";
import { BatchConfirmModal } from "./BatchConfirmModal";
import { BlockInfoModal } from "./BlockInfoModal";
import { NameBatchBar } from "./names/NameBatchBar";
import { PageHeader } from "./ui/PageHeader";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { Icon } from "./ui/Icon";
import { Input } from "./ui/Input";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";
import { explorerCoversNetwork } from "../lib/openExternal";

type Filter = "all" | NameCat;
const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "needs", label: "Needs you" },
  { key: "auction", label: "In auction" },
  { key: "owned", label: "Owned" },
  { key: "watching", label: "Watching" },
];
const EMPTY: Record<Filter, string> = {
  all: "No names yet. Look up a name above to start an auction or watch it.",
  needs: "Nothing needs you right now.",
  auction: "No active auctions. Look up a name above to get started.",
  owned: "No owned names found yet. Sync to scan for names this wallet owns.",
  watching: "No names on your watchlist yet. Look up a name above and choose Watch.",
};

/** Names you own, bid on or watch, and finding new ones — one table, one row per name. */
export function NamesView() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const [params, setParams] = useSearchParams();
  // The whole view lives in the URL (Back, reload and links keep it).
  const view = parseNamesParams(params);
  const { filter, sort, dir, filters } = view;
  const setView = (patch: Partial<NamesViewState>) =>
    setParams(namesParams({ ...view, ...patch }), { replace: true });

  const { data: profile } = useActiveProfile();
  const profileId = profile?.id ?? null;
  const isWatchOnly = profile?.watchOnly ?? false;
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const unlocked =
    profile?.kind === "ledger_hardware" ? true : isSignerUnlockedFor(signer, profileId);

  const namesQuery = useReadNames();
  const owned = useMemo(() => namesQuery.data ?? [], [namesQuery.data]);
  const { data: positions = [] } = useAuctionPositions(profileId);
  // Needs you owns the one capability fetch (owned names + positions), as
  // the Wallet page does; this page reuses it.
  const needsYou = useNeedsYou();
  const { capsByName } = needsYou;
  const { data: renewals } = useReadRenewals();
  const { data: drafts = [] } = useTxDrafts();
  const finalizeSent = useMemo(() => finalizeSentIds(drafts), [drafts]);
  const { data: watched = [] } = useWatchlist();
  const watchedNames = useMemo(() => watched.map((w) => w.name), [watched]);
  const { data: watchStatus = [] } = useWatchlistStatuses(watchedNames);
  const { data: watchInfo = {} } = useWatchlistLiveInfo(watchedNames);
  const addWatch = useAddToWatchlist();
  const removeWatch = useRemoveFromWatchlist();
  const updateTags = useUpdateWatchlistTags();
  const { finalize, finalizingId } = usePurchaseFinalize();
  const finalizeRefusal = shakedexRefusal(profile?.kind, writeCap);

  const rows = useMemo(
    () =>
      buildNameRows({
        owned,
        positions,
        capsByName,
        watched,
        watchInfo,
        watchStatus,
        needs: needsYou.items,
        renewals: renewals?.names,
        finalizeSent,
        wallClock: wallClock(profile?.network),
        isWatchOnly,
      }),
    [
      owned,
      positions,
      capsByName,
      watched,
      watchInfo,
      watchStatus,
      needsYou.items,
      renewals,
      finalizeSent,
      profile?.network,
      isWatchOnly,
    ],
  );
  const counts = chipCounts(rows);

  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const clock = wallClock(profile?.network);
  const tagCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of rows) for (const t of r.tags) m.set(t, (m.get(t) ?? 0) + 1);
    return [...m].sort((a, b) => a[0].localeCompare(b[0]));
  }, [rows]);
  // A tag no row carries any more drops out of the view.
  const liveFilters: NameFilters = {
    ...filters,
    tags: filters.tags.filter((t) => tagCounts.some(([k]) => k === t)),
  };
  const filtering = activeFilterCount(liveFilters) > 0;
  const shown = useMemo(
    () =>
      sortRows(
        rows.filter(
          (r) =>
            (filter === "all" || r.cats.includes(filter)) &&
            (!q || nameMatches(r.name, q) || r.tags.some((t) => t.toLowerCase().includes(q))) &&
            passesFilters(r, liveFilters, clock),
        ),
        sort,
        dir,
      ),
    // liveFilters is rebuilt each render; its parts are the real inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      rows,
      filter,
      q,
      sort,
      dir,
      clock,
      filters.expires,
      filters.len,
      filters.chars,
      filters.icann,
      liveFilters.tags.join("\u0000"),
    ],
  );
  const setFilters = (f: NameFilters) => setView({ filters: f });
  const clearFilters = () => setView({ filters: NO_FILTERS });
  const sortBy = (col: NameSort) =>
    setView(
      col === sort ? { dir: dir === "asc" ? "desc" : "asc" } : { sort: col, dir: NATURAL_DIR[col] },
    );

  // What the chain says about each owned name: enough to know whether a
  // TRANSFER is possible without the capability query.
  const chainFacts = useMemo(() => {
    const m = new Map<string, string | null>();
    for (const n of namesQuery.data ?? []) {
      // hsd's `expired` flag outlives a renewal (it was true on a name whose
      // TRANSFER the node then accepted), so expiry is read from the countdown.
      // A field the read did not fill in blocks nothing.
      const blocksLeft = n.stats?.blocksUntilExpire;
      const why =
        n.state && n.state !== "CLOSED"
          ? "still in its auction"
          : n.registered === false
            ? "not registered"
            : blocksLeft != null && blocksLeft <= 0
              ? "expired"
              : n.transfer
                ? "a transfer is already pending"
                : null;
      m.set(n.name, why);
    }
    return m;
  }, [namesQuery.data]);
  const transferableByChain = useMemo(
    () => new Set([...chainFacts].filter(([, why]) => why === null).map(([n]) => n)),
    [chainFacts],
  );
  const batch = useNameBatchActions({
    profile,
    capsByName,
    unlocked,
    transferable: transferableByChain,
  });
  // A batch never includes a row the chip, text or filters hide.
  const { setSelected } = batch;
  useEffect(() => {
    const visible = new Set(shown.map((r) => r.name));
    setSelected((prev) => {
      const next = new Set([...prev].filter((n) => visible.has(n)));
      return next.size === prev.size ? prev : next;
    });
  }, [shown, setSelected]);

  // Which selected names stop a batch transfer, with the backend's reason, and
  // a one-click selection of the next MAX_BATCH transferable names in table
  // order: a wallet with hundreds of names cannot pick 100 by hand.
  const transferBlockers = useMemo(
    () =>
      [...batch.selected]
        .map((n) => {
          const c = capsByName.get(n);
          const capWhy =
            c && !c.canTransfer.allowed ? (c.canTransfer.reason ?? "not transferable") : null;
          const chainWhy = chainFacts.has(n) ? chainFacts.get(n) : "not a name of this wallet";
          return { name: n, reason: capWhy ?? chainWhy ?? null };
        })
        .filter((b): b is { name: string; reason: string } => b.reason != null),
    [batch.selected, capsByName, chainFacts],
  );
  const transferableShown = useMemo(
    () =>
      shown
        .filter((r) => r.selectable && transferableByChain.has(r.name))
        .filter((r) => capsByName.get(r.name)?.canTransfer.allowed ?? true)
        .map((r) => r.name),
    [shown, capsByName, transferableByChain],
  );
  const transferableLeft = transferableShown.filter((n) => !batch.selected.has(n)).length;
  const selectTransferable = () => {
    setSelected(new Set(transferableShown));
  };
  // Keep the user's own selection, minus the names that would stop the batch
  // (no capability, not transferable, or expired).
  const dropBlockers = () => {
    const blocked = new Set(transferBlockers.map((b) => b.name));
    setSelected(new Set([...batch.selected].filter((n) => !blocked.has(n))));
  };

  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  const [batchBidOpen, setBatchBidOpen] = useState(false);
  const [editingTags, setEditingTags] = useState<string | null>(null);
  const [tagValue, setTagValue] = useState("");

  // Seed the name page's single-name query from the batch so it opens on the
  // row's status, with no flash of the raw phase.
  const openNamePage = useOpenName();
  const openName = (name: string) => openNamePage(name, { seedCaps: capsByName.get(name) });

  // --- Lookup ---
  const [lookup, setLookup] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const lookupRef = useRef<HTMLInputElement>(null);
  const resultCaps = useNameActionCapabilities(result, profileId);
  const submitLookup = () => {
    const ace = normalizeNameInputAce(lookup);
    if (!ace) return;
    if (rows.some((r) => r.name === ace)) {
      setResult(null);
      openName(ace);
    } else {
      setResult(ace);
    }
  };
  const clearLookup = () => {
    setLookup("");
    setResult(null);
  };
  useEffect(() => {
    if (!result) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector('[role="dialog"]')) setResult(null);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [result]);

  // --- Keyboard list ---
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const selectedRowRef = useRef<HTMLTableRowElement>(null);
  useEffect(() => setSelectedIndex(-1), [filter, query]);
  useEffect(() => {
    selectedRowRef.current?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);
  // A purchase on its way is not ours yet: its row does what View does
  // (Market, where Your purchases lists it) instead of opening the name.
  const openRow = (r: NameRow) => (r.kind === "purchase" ? navigate("/market") : openName(r.name));
  const actionHandlerRef = useRef<(id: string) => void>(() => {});
  actionHandlerRef.current = (id: string) => {
    switch (id) {
      case "names:focusLookup":
        lookupRef.current?.focus();
        break;
      case "names:batchBid":
        if (!isWatchOnly) setBatchBidOpen(true);
        break;
      case "names:list:next":
        setSelectedIndex((i) => Math.min(i + 1, shown.length - 1));
        break;
      case "names:list:prev":
        setSelectedIndex((i) => Math.max(i - 1, 0));
        break;
      case "names:list:open":
        if (shown[selectedIndex]) openRow(shown[selectedIndex]!);
        break;
    }
  };
  useEffect(() => subscribeAction((id) => actionHandlerRef.current(id)), []);

  // Chips merge into the view: the sort and filters stay.
  const setFilter = (key: Filter) => setView({ filter: key });

  const selectable = isWatchOnly ? [] : shown.filter((r) => r.selectable);
  const allSelected = selectable.length > 0 && selectable.every((r) => batch.selected.has(r.name));
  const someSelected = selectable.some((r) => batch.selected.has(r.name));

  const saveTags = (name: string) => {
    updateTags.mutate({ name, tags: tagValue }, { onSuccess: () => setEditingTags(null) });
  };

  const rowAction = (r: NameRow) => {
    if (!r.action) return null;
    if (r.action.kind === "myListings") {
      return (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => navigate("/market?tab=mine")}
          data-testid="owned-name-my-listings"
        >
          {r.action.label}
        </Button>
      );
    }
    if (r.action.kind === "viewPurchase") {
      return (
        <Button size="sm" variant="ghost" onClick={() => navigate("/market")}>
          View
        </Button>
      );
    }
    if (isWatchOnly && r.action.signs) {
      return (
        <Button size="sm" variant="ghost" onClick={() => openName(r.name)}>
          View
        </Button>
      );
    }
    if (r.action.kind === "finalizePurchase") {
      const blocked = !canUseShakedex(profile?.kind, writeCap);
      return (
        <Tooltip content={blocked ? finalizeRefusal : undefined}>
          <Button
            size="sm"
            variant="secondary"
            data-testid="owned-name-finalize"
            disabled={finalizingId !== null || blocked}
            onClick={() =>
              void finalize(r.purchaseId!, (txid) => `Finalize broadcast — ${txid.slice(0, 12)}…`)
            }
          >
            {finalizingId === r.purchaseId ? "Finalizing…" : "Finalize"}
          </Button>
        </Tooltip>
      );
    }
    return (
      <Button size="sm" variant="secondary" onClick={() => openName(r.name)}>
        {r.action.label}
      </Button>
    );
  };

  const resultData = resultCaps.data;
  const resultAvailable =
    resultData &&
    (auctionPhase(resultData.phase).phase === "AVAILABLE" ||
      resultData.taskState === "availableToOpen");
  const resultText = resultCaps.isError
    ? "Couldn't check this name. You can still open it."
    : !resultData
      ? null
      : resultAvailable
        ? "No auction yet. Anyone can open one; bidding starts once the opening period ends."
        : [
            taskStateLabel(resultData.taskState),
            resultData.countdownBlocks != null &&
              formatCountdown({
                label: resultData.countdownLabel ?? "",
                blocks: resultData.countdownBlocks,
                hours: resultData.countdownHours,
              }),
          ]
            .filter(Boolean)
            .join(" · ");
  const alreadyWatched = result != null && watched.some((w) => w.name === result);

  const loading = namesQuery.isPending && !namesQuery.data;

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Names"
        subtitle="Names you own, bid on or watch."
        aside={
          <>
            <Tooltip
              content={
                isWatchOnly
                  ? "Watch-only wallets can't bid"
                  : "Bid on several names in one transaction"
              }
            >
              <Button
                size="md"
                variant="secondary"
                disabled={isWatchOnly}
                onClick={() => setBatchBidOpen(true)}
                className="inline-flex items-center gap-1.5"
                data-testid="open-batch-bid-btn"
              >
                <Icon icon={Gavel} size={14} /> Batch bid…
              </Button>
            </Tooltip>
            <span className="h-6 w-px bg-gray-200" aria-hidden />
            <Tooltip content="Add names to your watchlist from a CSV file">
              <Button
                size="md"
                variant="secondary"
                className="inline-flex items-center gap-1.5"
                onClick={() =>
                  void importWatchlistCsv(showToast, () =>
                    qc.invalidateQueries({ queryKey: ["watchlist"] }),
                  )
                }
              >
                <Icon icon={Upload} size={14} /> Import CSV
              </Button>
            </Tooltip>
            <Tooltip content={watched.length === 0 ? "Your watchlist is empty" : undefined}>
              <Button
                size="md"
                variant="secondary"
                className="inline-flex items-center gap-1.5"
                disabled={watched.length === 0}
                onClick={() => void exportWatchlistCsv(showToast)}
              >
                <Icon icon={Download} size={14} /> Export CSV
              </Button>
            </Tooltip>
          </>
        }
      />

      {/* Lookup: open a known name, or check a new one and bid on / watch it. */}
      <section className="bg-white rounded-md border border-gray-200 p-4 flex flex-col gap-3">
        <div className="flex items-center gap-2">
          <span className="relative flex-1 flex items-center [&>div]:flex-1">
            <Icon
              icon={Search}
              size={14}
              className="pointer-events-none absolute left-2.5 text-gray-400"
            />
            <span className="pointer-events-none absolute left-7 font-mono text-gray-400">.</span>
            <Input
              ref={lookupRef}
              inputSize="md"
              className="w-full pl-9 font-mono"
              placeholder="Find a name to open, bid on or watch"
              value={lookup}
              onChange={(e) => setLookup(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && lookup.trim()) submitLookup();
              }}
              data-testid="names-lookup-input"
            />
          </span>
          <Tooltip content={lookup.trim() ? undefined : "Type a name first"}>
            <Button variant="primary" disabled={!lookup.trim()} onClick={submitLookup}>
              Look up
            </Button>
          </Tooltip>
        </div>
        {result && (
          <div
            className="bg-gray-50 border border-gray-200 rounded-md px-3 py-2.5 flex flex-wrap items-center gap-3"
            data-testid="names-lookup-result"
          >
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="font-mono text-[15px] font-semibold text-gray-900">
                  .{displayName(result)}
                </span>
                {resultCaps.isPending && !resultData ? (
                  <Badge>Checking…</Badge>
                ) : resultAvailable ? (
                  <Badge>Available</Badge>
                ) : resultData ? (
                  <Badge variant={auctionPhase(resultData.phase).variant}>
                    {auctionPhase(resultData.phase).label}
                  </Badge>
                ) : null}
              </div>
              {resultText && <div className="text-xs text-gray-600 mt-0.5">{resultText}</div>}
            </div>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => {
                openName(result);
                clearLookup();
              }}
            >
              {resultAvailable && !isWatchOnly ? "Open auction…" : "Open name"}
            </Button>
            {!alreadyWatched && (
              <Button
                size="sm"
                variant="ghost"
                className="inline-flex items-center gap-1"
                disabled={addWatch.isPending}
                onClick={() => addWatch.mutate(result, { onSuccess: clearLookup })}
                data-testid="names-lookup-watch"
              >
                <Icon icon={Star} size={13} /> Watch
              </Button>
            )}
            <Tooltip content="Dismiss">
              <button
                type="button"
                aria-label="Dismiss"
                className="p-1 rounded text-gray-400 hover:text-gray-600"
                onClick={() => setResult(null)}
              >
                <Icon icon={X} size={14} />
              </button>
            </Tooltip>
          </div>
        )}
      </section>

      <div className="flex flex-wrap items-center gap-2">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            type="button"
            onClick={() => setFilter(f.key)}
            aria-pressed={filter === f.key}
            data-testid={`names-chip-${f.key}`}
            className={`rounded-full border px-3 py-1 text-[13px] ${
              filter === f.key
                ? "bg-gray-900 text-white border-gray-900"
                : "bg-white text-gray-700 border-gray-200 hover:bg-gray-50"
            }`}
          >
            {f.label} <span className="text-[11px] opacity-75 tabular-nums">{counts[f.key]}</span>
          </button>
        ))}
        <span className="ml-auto text-xs text-gray-500 tabular-nums" data-testid="names-count">
          {filter !== "all" || q || filtering
            ? `${shown.length} of ${rows.length}`
            : `${rows.length} names`}
        </span>
        <Input
          inputSize="sm"
          className="w-48"
          placeholder="Filter by name or tag…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          data-testid="names-filter"
        />
        <NameFiltersPopover
          filters={liveFilters}
          onChange={setFilters}
          tagCounts={tagCounts}
          clock={clock}
        />
      </div>

      {filtering && (
        <div className="flex flex-wrap items-center gap-2 -mt-2" data-testid="names-active-filters">
          {activePills(liveFilters, clock).map((p) => (
            <span
              key={p.key}
              className="inline-flex items-center gap-1 rounded-full border border-blue-200 bg-blue-50 px-2.5 py-0.5 text-xs text-blue-800"
            >
              {p.label}
              <button
                type="button"
                aria-label="Remove filter"
                className="rounded-full hover:bg-blue-100"
                onClick={() => setFilters(p.remove(liveFilters))}
                data-testid="names-active-filter-remove"
              >
                <Icon icon={X} size={12} />
              </button>
            </span>
          ))}
          <button
            type="button"
            className="text-xs text-blue-600 hover:underline"
            onClick={clearFilters}
          >
            Clear filters
          </button>
        </div>
      )}

      {!isWatchOnly && (batch.selected.size > 0 || transferableLeft > 0) && (
        <NameBatchBar
          batch={batch}
          transferBlockers={transferBlockers}
          onSelectTransferable={selectTransferable}
          onDropBlockers={dropBlockers}
          transferableLeft={transferableLeft}
        />
      )}

      {needsYou.isDegraded && (
        <div
          className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
          data-testid="names-tasks-degraded"
        >
          Couldn't verify urgent auction tasks — data may be stale.
        </div>
      )}

      <section className="bg-white rounded-md border border-gray-200">
        {loading ? (
          <div className="p-4 flex flex-col gap-2" data-testid="names-loading">
            <div className="text-sm text-gray-500">Loading your names…</div>
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-5 rounded bg-gray-200/70 animate-pulse" />
            ))}
          </div>
        ) : shown.length === 0 ? (
          <div
            className="py-8 text-center text-sm text-gray-500 flex flex-col items-center gap-3"
            data-testid="names-empty"
          >
            {filtering ? (
              <>
                No names match these filters.
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={clearFilters}
                  data-testid="names-filters-empty-clear"
                >
                  Clear filters
                </Button>
              </>
            ) : q ? (
              `No names match "${query.trim()}"`
            ) : (
              EMPTY[filter]
            )}
          </div>
        ) : (
          <div className="p-4 pb-0">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1 pr-4 w-6">
                    {selectable.length > 0 && (
                      <input
                        type="checkbox"
                        checked={allSelected}
                        ref={(el) => {
                          if (el) el.indeterminate = someSelected && !allSelected;
                        }}
                        onChange={(e) =>
                          batch.setSelected((prev) => {
                            const next = new Set(prev);
                            for (const r of selectable) {
                              if (e.target.checked) next.add(r.name);
                              else next.delete(r.name);
                            }
                            return next;
                          })
                        }
                        aria-label="Select all names"
                      />
                    )}
                  </th>
                  {SORT_COLUMNS.map((c) => (
                    <SortHeader
                      key={c.key}
                      column={c}
                      active={sort === c.key}
                      dir={dir}
                      onSort={sortBy}
                    />
                  ))}
                  <th className="py-1" />
                </tr>
              </thead>
              <tbody>
                {shown.map((r, i) => {
                  const picked = batch.selected.has(r.name);
                  const tags = (r.watch?.tags ?? "")
                    .split(",")
                    .map((t) => t.trim())
                    .filter(Boolean);
                  return (
                    <tr
                      key={r.name}
                      ref={i === selectedIndex ? selectedRowRef : undefined}
                      aria-selected={i === selectedIndex}
                      className={`border-t border-gray-100 hover:bg-gray-50 cursor-pointer ${
                        i === selectedIndex
                          ? "bg-blue-50 ring-1 ring-blue-300"
                          : picked
                            ? "bg-blue-50"
                            : ""
                      }`}
                      onClick={(e) => {
                        if (!fromInteractiveChild(e)) openRow(r);
                      }}
                      data-testid="names-row"
                    >
                      <td className="py-1 pr-4 align-top">
                        {r.selectable && !isWatchOnly && (
                          <input
                            type="checkbox"
                            checked={picked}
                            onChange={() => batch.toggle(r.name)}
                            aria-label={`Select ${displayName(r.name)}`}
                          />
                        )}
                      </td>
                      <td className="py-1 pr-4">
                        <div className="flex items-center gap-1.5">
                          {r.watch && (
                            <Tooltip content="On your watchlist" className="inline-flex">
                              <Icon icon={Star} size={13} className="text-amber-500" />
                            </Tooltip>
                          )}
                          <span className="font-mono text-[13px] text-gray-900">
                            .{displayName(r.name)}
                          </span>
                          {displayName(r.name) !== r.name && (
                            <span className="text-[11px] text-gray-500">{r.name}</span>
                          )}
                        </div>
                        {(r.height != null || r.renewal != null) && (
                          <div className="text-[11px] text-gray-500">
                            {r.height != null && (
                              <>
                                opened <BlockLink height={r.height} onClick={setInfoBlock} />
                              </>
                            )}
                            {r.renewal != null && (
                              <>
                                {r.height != null && " · "}renewed{" "}
                                <BlockLink height={r.renewal} onClick={setInfoBlock} />
                              </>
                            )}
                          </div>
                        )}
                        {r.watch && (
                          <div className="text-[11px] text-gray-500 flex items-center gap-1">
                            {editingTags === r.name ? (
                              <input
                                className="border border-blue-300 rounded px-1 py-0.5 text-xs w-32"
                                value={tagValue}
                                onChange={(e) => setTagValue(e.target.value)}
                                onBlur={() => saveTags(r.name)}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") saveTags(r.name);
                                  if (e.key === "Escape") {
                                    e.stopPropagation();
                                    setEditingTags(null);
                                  }
                                }}
                                autoFocus
                                placeholder="tag1, tag2"
                              />
                            ) : (
                              <Tooltip content="Click to edit tags">
                                <button
                                  type="button"
                                  className="hover:bg-gray-100 rounded px-0.5"
                                  aria-label={`Edit tags for ${r.name}`}
                                  data-testid="watchlist-tags-cell"
                                  onClick={() => {
                                    setEditingTags(r.name);
                                    setTagValue(r.watch!.tags);
                                  }}
                                >
                                  {tags.length > 0 ? (
                                    <span className="flex flex-wrap gap-1">
                                      {tags.map((t) => (
                                        <span
                                          key={t}
                                          className="bg-gray-100 text-gray-600 text-xs rounded px-1.5"
                                        >
                                          {t}
                                        </span>
                                      ))}
                                    </span>
                                  ) : (
                                    "+ tags"
                                  )}
                                </button>
                              </Tooltip>
                            )}
                            {r.watch.addedAt && (
                              <span>
                                · added{" "}
                                {new Date(r.watch.addedAt).toLocaleDateString("en-US", {
                                  year: "numeric",
                                  month: "short",
                                  day: "numeric",
                                })}
                              </span>
                            )}
                          </div>
                        )}
                      </td>
                      <td className="py-1 pr-4">
                        <Tooltip
                          content={r.statusTitle || undefined}
                          className="inline-flex items-center gap-1.5 flex-wrap"
                        >
                          <span
                            className={`h-2 w-2 rounded-full shrink-0 ${NAME_TONE_DOT[r.tone]}`}
                          />
                          <span className="text-gray-900">{r.status}</span>
                          {r.detail && <span className="text-gray-500">· {r.detail}</span>}
                        </Tooltip>
                        {r.pending && (
                          <Badge
                            variant="info"
                            className="ml-1.5"
                            title="Sent from this wallet. Nothing changes on-chain until it lands in a block."
                            data-testid="auction-pending-block"
                          >
                            {r.pending}
                          </Badge>
                        )}
                        {/* One badge per row (DESIGN_SYSTEM "Badge / tag"): a pending
                            broadcast of ours outranks the durable ICANN flag (spec R4). */}
                        {!r.pending && r.icann && (
                          <Badge
                            variant="warning"
                            className="ml-1.5"
                            title={icannBadgeTitle(r.icann)}
                            data-testid="icann-badge"
                          >
                            ICANN
                          </Badge>
                        )}
                      </td>
                      <td className="py-1 pr-4 text-right font-mono tabular-nums whitespace-nowrap">
                        {r.value ?? <span className="text-gray-500">—</span>}
                      </td>
                      <td
                        className={`py-1 pr-4 text-xs whitespace-nowrap ${r.expires?.className ?? "text-gray-500"}`}
                      >
                        {r.expires?.text ?? "—"}
                      </td>
                      <td className="py-1 text-right whitespace-nowrap">
                        <span className="inline-flex items-center gap-1">
                          {rowAction(r)}
                          <Popover
                            align="right"
                            panelClassName="w-[200px]"
                            trigger={({ toggle }) => (
                              <Tooltip content="More">
                                <button
                                  type="button"
                                  aria-label="More"
                                  className="p-1 rounded text-gray-500 hover:bg-gray-100"
                                  onClick={toggle}
                                >
                                  <Icon icon={Ellipsis} size={16} />
                                </button>
                              </Tooltip>
                            )}
                          >
                            {({ close }) =>
                              nameRowMenu({
                                caps: capsByName.get(r.name),
                                watched: !!r.watch,
                                watchOnly: isWatchOnly,
                              }).map((item) => (
                                <MenuItem
                                  key={item.label}
                                  icon={menuIcon(item)}
                                  className={
                                    item.do.kind === "unwatch" ? "text-red-700" : undefined
                                  }
                                  onClick={() => {
                                    close();
                                    const d = item.do;
                                    if (d.kind === "open")
                                      openNamePage(r.name, {
                                        tab: d.tab,
                                        seedCaps: capsByName.get(r.name),
                                      });
                                    else if (d.kind === "run")
                                      void batch.runFor(d.action, [r.name]);
                                    else if (d.kind === "watch") addWatch.mutate(r.name);
                                    else if (d.kind === "unwatch") removeWatch.mutate(r.name);
                                    else {
                                      setEditingTags(r.name);
                                      setTagValue(r.watch?.tags ?? "");
                                    }
                                  }}
                                >
                                  {item.label}
                                </MenuItem>
                              ))
                            }
                          </Popover>
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        <div className="border-t border-gray-100 px-4 py-2.5 text-xs text-gray-500 mt-2">
          Values in HNS. Click a row to open the name.
        </div>
      </section>

      {infoBlock != null && (
        <BlockInfoModal
          height={infoBlock}
          open
          onClose={() => setInfoBlock(null)}
          isMainnet={explorerCoversNetwork(profile?.network)}
        />
      )}
      <BatchBidModal
        open={batchBidOpen}
        onClose={() => setBatchBidOpen(false)}
        activeProfileId={profileId}
      />
      {batch.modal && (
        <BatchConfirmModal
          open={batch.modal.open}
          action={batch.modal.action}
          names={batch.modal.names}
          batches={batch.modal.batches}
          estimatedFeeDoos={batch.modal.feeDoos}
          amountDoos={batch.modal.amountDoos}
          recipient={batch.modal.recipient}
          onConfirm={batch.confirm}
          onCancel={batch.cancel}
        />
      )}
    </div>
  );
}

function BlockLink({ height, onClick }: { height: number; onClick: (h: number) => void }) {
  return (
    <Tooltip content="View block info" className="inline-flex">
      <button
        type="button"
        className="font-mono text-blue-600 hover:underline"
        onClick={() => onClick(height)}
        data-testid="names-block-link"
      >
        #{height}
      </button>
    </Tooltip>
  );
}

const RUN_ICONS = {
  renew: RefreshCw,
  reveal: CircleCheck,
  redeem: CircleCheck,
  finalize: CircleCheck,
};

function menuIcon(item: RowMenuItem): Parameters<typeof Icon>[0]["icon"] {
  const d = item.do;
  if (d.kind === "run") return RUN_ICONS[d.action];
  if (d.kind === "watch") return Star;
  if (d.kind === "editTags") return Tag;
  if (d.kind === "unwatch") return StarOff;
  if (d.tab === "transfer") return Send;
  if (d.tab === "dns") return Settings2;
  if (d.tab === "sign") return Signature;
  return item.label.startsWith("Cancel") ? X : ArrowUpRight;
}

function MenuItem({
  icon,
  onClick,
  className,
  children,
}: {
  icon: Parameters<typeof Icon>[0]["icon"];
  onClick: () => void;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className={`w-full flex items-center gap-2 px-3 py-1.5 text-left text-[13px] hover:bg-gray-50 ${className ?? "text-gray-700"}`}
    >
      <Icon icon={icon} size={14} /> {children}
    </button>
  );
}

const SORT_COLUMNS: { key: NameSort; label: string; tip: string; right?: boolean }[] = [
  { key: "name", label: "Name", tip: "Sort by name" },
  { key: "status", label: "Status", tip: "Sort by what needs you first (default)" },
  { key: "value", label: "Value / bid", tip: "Sort by value or bid", right: true },
  { key: "expires", label: "Expires", tip: "Sort by expiry, soonest first" },
];

/** A sortable column header: click sorts, a second click reverses. */
function SortHeader({
  column,
  active,
  dir,
  onSort,
}: {
  column: (typeof SORT_COLUMNS)[number];
  active: boolean;
  dir: SortDir;
  onSort: (c: NameSort) => void;
}) {
  return (
    <th
      className={`py-1 pr-4 font-normal ${column.right ? "text-right" : ""}`}
      aria-sort={active ? (dir === "asc" ? "ascending" : "descending") : "none"}
    >
      <Tooltip content={column.tip} className="inline-flex">
        <button
          type="button"
          onClick={() => onSort(column.key)}
          data-testid={`names-sort-${column.key}`}
          className={`inline-flex items-center gap-1 ${
            active ? "text-gray-900 font-medium" : "text-gray-500 hover:text-gray-700"
          }`}
        >
          {column.label}
          {active ? (
            <Icon icon={dir === "asc" ? ArrowUp : ArrowDown} size={13} />
          ) : (
            <Icon icon={ArrowUpDown} size={13} className="text-gray-300" />
          )}
        </button>
      </Tooltip>
    </th>
  );
}

/** One removable pill per active filter. */
function activePills(
  f: NameFilters,
  clock: boolean,
): { key: string; label: string; remove: (f: NameFilters) => NameFilters }[] {
  const pills: { key: string; label: string; remove: (f: NameFilters) => NameFilters }[] = [];
  if (f.expires !== "any") {
    const label = expiresLabels(clock)[f.expires];
    pills.push({
      key: "expires",
      label: `Expires ${label.charAt(0).toLowerCase()}${label.slice(1)}`,
      remove: (x) => ({ ...x, expires: "any" }),
    });
  }
  if (f.len !== "any") {
    pills.push({
      key: "len",
      label: `Length ${LENGTH_LABELS[f.len]}`,
      remove: (x) => ({ ...x, len: "any" }),
    });
  }
  if (f.chars !== "any") {
    pills.push({
      key: "chars",
      label: CHAR_LABELS[f.chars],
      remove: (x) => ({ ...x, chars: "any" }),
    });
  }
  if (f.icann !== "any") {
    pills.push({
      key: "icann",
      label: "ICANN collision",
      remove: (x) => ({ ...x, icann: "any" }),
    });
  }
  for (const t of f.tags) {
    pills.push({
      key: `tag:${t}`,
      label: `Tag: ${t}`,
      remove: (x) => ({ ...x, tags: x.tags.filter((y) => y !== t) }),
    });
  }
  return pills;
}
