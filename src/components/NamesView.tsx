import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight,
  Download,
  Ellipsis,
  Gavel,
  Search,
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
  nameCapabilitiesQueryKey,
  useAuctionPositions,
  useNameActionCapabilities,
  useReadNames,
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
import { auctionPhase, formatCountdown, taskStateLabel } from "../lib/auction";
import { displayName, nameMatches } from "../lib/idn";
import { fromInteractiveChild } from "../lib/rowClick";
import { isSignerUnlockedFor } from "../lib/signer";
import { normalizeNameInputAce } from "../lib/utils";
import { subscribeAction } from "../lib/actionBus";
import { useNameBatchActions } from "../hooks/useNameBatchActions";
import { useUiStore } from "../stores/ui";
import { finalizeSentIds, usePurchaseFinalize } from "./market/usePurchase";
import { NAME_TONE_DOT, canUseShakedex, shakedexRefusal } from "./market/marketText";
import { NameActionsModal } from "./NameActionsModal";
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

function parseFilter(raw: string | null): Filter {
  return FILTERS.some((f) => f.key === raw) ? (raw as Filter) : "all";
}

/** Names you own, bid on or watch, and finding new ones — one table, one row per name. */
export function NamesView() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const [params, setParams] = useSearchParams();
  const filter = parseFilter(params.get("filter"));

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
        needs: [...new Set(needsYou.items.map((i) => i.name))],
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
      finalizeSent,
      profile?.network,
      isWatchOnly,
    ],
  );
  const counts = chipCounts(rows);

  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const shown = rows.filter(
    (r) => (filter === "all" || r.cats.includes(filter)) && (!q || nameMatches(r.name, q)),
  );

  const batch = useNameBatchActions({ profile, capsByName, unlocked });

  const [manageName, setManageName] = useState<string | null>(null);
  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  const [batchBidOpen, setBatchBidOpen] = useState(false);
  const [editingTags, setEditingTags] = useState<string | null>(null);
  const [tagValue, setTagValue] = useState("");

  // One way to open a name (the name page replaces the modal later). Seed
  // the modal's single-name query from the batch so it opens on the row's
  // status, with no flash of the raw phase.
  const openName = (name: string) => {
    const caps = capsByName.get(name);
    if (caps) qc.setQueryData(nameCapabilitiesQueryKey(profileId, name), caps);
    setManageName(name);
  };

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
        if (shown[selectedIndex] && shown[selectedIndex]!.kind !== "purchase")
          openName(shown[selectedIndex]!.name);
        break;
    }
  };
  useEffect(() => subscribeAction((id) => actionHandlerRef.current(id)), []);

  const setFilter = (key: Filter) =>
    setParams(key === "all" ? {} : { filter: key }, { replace: true });

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
          {filter !== "all" || q ? `${shown.length} of ${rows.length}` : `${rows.length} names`}
        </span>
        <Input
          inputSize="sm"
          className="w-40"
          placeholder="Filter…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          data-testid="names-filter"
        />
      </div>

      {batch.selected.size > 0 && !isWatchOnly && <NameBatchBar batch={batch} />}

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
          <div className="py-8 text-center text-sm text-gray-500" data-testid="names-empty">
            {q ? `No names match "${query.trim()}"` : EMPTY[filter]}
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
                  <th className="py-1 pr-4 font-normal">Name</th>
                  <th className="py-1 pr-4 font-normal">Status</th>
                  <th className="py-1 pr-4 font-normal text-right">Value / bid</th>
                  <th className="py-1 pr-4 font-normal">Expires</th>
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
                      className={`border-t border-gray-100 hover:bg-gray-50 ${r.kind === "purchase" ? "" : "cursor-pointer"} ${
                        i === selectedIndex
                          ? "bg-blue-50 ring-1 ring-blue-300"
                          : picked
                            ? "bg-blue-50"
                            : ""
                      }`}
                      onClick={(e) => {
                        // A purchase on its way is not ours yet: nothing to open
                        // (its Finalize / View act on the purchase).
                        if (fromInteractiveChild(e) || r.kind === "purchase") return;
                        openName(r.name);
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
                          {r.watch && (
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
                              {({ close }) => (
                                <>
                                  <MenuItem
                                    icon={ArrowUpRight}
                                    onClick={() => {
                                      close();
                                      openName(r.name);
                                    }}
                                  >
                                    Open name
                                  </MenuItem>
                                  <MenuItem
                                    icon={Tag}
                                    onClick={() => {
                                      close();
                                      setEditingTags(r.name);
                                      setTagValue(r.watch!.tags);
                                    }}
                                  >
                                    Edit tags
                                  </MenuItem>
                                  <MenuItem
                                    icon={StarOff}
                                    className="text-red-700"
                                    onClick={() => {
                                      close();
                                      removeWatch.mutate(r.name);
                                    }}
                                  >
                                    Remove from watchlist
                                  </MenuItem>
                                </>
                              )}
                            </Popover>
                          )}
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

      {manageName && (
        <NameActionsModal name={manageName} open onClose={() => setManageName(null)} />
      )}
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
