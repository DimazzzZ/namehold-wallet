import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { PageHeader } from "../ui/PageHeader";
import { Tabs } from "../ui/Tabs";
import { ImportListingDialog } from "./ImportListingDialog";
import { ListingDetail } from "./ListingDetail";
import { ListingsTable, type ListState } from "./ListingsTable";
import { MarketGateBanner } from "./MarketGateBanner";
import { MyListings } from "./MyListings";
import { PendingPurchases, type PendingPurchase } from "./PendingPurchases";
import { finalizeSentIds, usePurchaseFinalize } from "./usePurchase";
import {
  BUYING_ON_TOAST,
  MARKET_MAINNET_ONLY,
  MY_LISTINGS_TAB,
  buyingGated,
  shakedexRefusal,
} from "./marketText";
import {
  buildList,
  entryKey,
  visibleEntries,
  type Entry,
  type ImportedRow,
  type ListFilter,
  type ListSort,
  type ListedEntry,
} from "./marketModel";
import { useImportListing, useMarketPages } from "../../queries/shakedex";
import { useReadNames } from "../../queries/read";
import { useNodeStatus } from "../../queries/node";
import {
  useActiveProfile,
  useTxDrafts,
  useWalletBalances,
  useWriteCapability,
} from "../../queries/wallet";
import { useSettingsStore } from "../../stores/settings";
import { useUiStore } from "../../stores/ui";
import { subscribeAction, type ActionId } from "../../lib/actionBus";
import { displayName } from "../../lib/idn";
import { mapError } from "../../lib/errors";
import type { MarketRow } from "../../types";

type MarketTab = "browse" | "mine";

const TABS: { key: MarketTab; label: string }[] = [
  { key: "browse", label: "Browse" },
  { key: "mine", label: MY_LISTINGS_TAB },
];

/** The Market: Browse (buying) or My listings (selling), `?tab=mine`. */
export default function MarketPage() {
  const [params, setParams] = useSearchParams();
  const tab: MarketTab = params.get("tab") === "mine" ? "mine" : "browse";
  const [importOpen, setImportOpen] = useState(false);
  return (
    <div className="mx-auto max-w-[1180px]">
      <PageHeader
        title="Market"
        subtitle="Names their owners are selling through Shakedex. Your node checks every listing before you can buy."
        actions={
          tab === "browse"
            ? [
                {
                  label: "Import listing…",
                  onClick: () => setImportOpen(true),
                  testId: "market-import",
                },
              ]
            : []
        }
      />
      <Tabs
        className="mb-4"
        tabs={TABS}
        active={tab}
        onChange={(k) => setParams(k === "mine" ? { tab: "mine" } : {})}
      />
      {tab === "mine" ? (
        <MyListings />
      ) : (
        <Browse importOpen={importOpen} setImportOpen={setImportOpen} />
      )}
    </div>
  );
}

/** A row's place in the list moves by one; past either end it stays put. */
function step(rows: Entry[], key: string | null, by: 1 | -1): string | null {
  if (rows.length === 0) return null;
  const at = rows.findIndex((e) => e.key === key);
  const next = at < 0 ? 0 : Math.min(rows.length - 1, Math.max(0, at + by));
  return rows[next]?.key ?? null;
}

/** Focus is on a control that answers Enter itself. */
function focusAnswersEnter(): boolean {
  return Boolean(
    document.activeElement?.closest("button, a, input, select, textarea, [role='button']"),
  );
}

/** Buying: the imports and LearnHNS Market's listings, one list beside the selected one. */
function Browse({
  importOpen,
  setImportOpen,
}: {
  importOpen: boolean;
  setImportOpen: (open: boolean) => void;
}) {
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const { data: balances } = useWalletBalances();
  const { data: names = [] } = useReadNames();
  const { data: drafts = [] } = useTxDrafts();
  const { data: node } = useNodeStatus();
  const settings = useSettingsStore((s) => s.settings);
  const updateSetting = useSettingsStore((s) => s.update);
  const market = useMarketPages();
  const recheck = useImportListing();
  const { finalize, finalizingId } = usePurchaseFinalize();

  const [imported, setImported] = useState<ImportedRow[]>([]);
  // A Retry's answers, for the market fetch they were asked against: a
  // later fetch has its own verdicts, fresher than any of them.
  const [rechecks, setRechecks] = useState<{ at: number; rows: Map<string, MarketRow> }>({
    at: 0,
    rows: new Map(),
  });
  const [checkingKey, setCheckingKey] = useState<string | null>(null);
  // Listings bought in this session, as they were when bought.
  const [sent, setSent] = useState<Map<string, ListedEntry>>(new Map());
  // The fee box per listing, for the session.
  const [payByKey, setPayByKey] = useState<Map<string, boolean>>(new Map());
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ListFilter>("all");
  const [sort, setSort] = useState<ListSort>("price");
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [hiddenOpen, setHiddenOpen] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const primaryRef = useRef<(() => void) | null>(null);

  const refusal = shakedexRefusal(profile?.kind, writeCap);
  const gated = buyingGated(profile?.network, settings?.shakedex_experimental);
  const linkRefusal = profile != null && profile.network !== "mainnet" ? MARKET_MAINNET_ONLY : null;

  const pages = useMemo(() => market.data?.pages ?? [], [market.data]);
  const first = pages[0];
  const fetchedAt = market.dataUpdatedAt;
  const rechecked = useMemo(
    () => (rechecks.at === fetchedAt ? rechecks.rows : new Map<string, MarketRow>()),
    [rechecks, fetchedAt],
  );

  // Only a local node's state is known here; a remote node answers for itself.
  const local = settings?.chain_source === "local_node";
  const offline = local && node?.connected === false;
  const syncing = local && node?.connected === true && !node.synced;
  // Whether the market's own rows can be trusted on screen; imports always can.
  const marketShown =
    !market.isPending && !market.isError && !offline && !syncing && !(first && !first.verified);

  const { entries, hidden } = useMemo(
    () =>
      marketShown
        ? buildList({ imported, pages, rechecked, sent })
        : buildList({ imported, pages: [], rechecked, sent }),
    [marketShown, imported, pages, rechecked, sent],
  );
  const rows = useMemo(
    () => visibleEntries(entries, { query, filter, sort }),
    [entries, query, filter, sort],
  );
  // The chosen row while it is on screen; else the first verified one.
  const shownKey = rows.some((e) => e.key === selectedKey)
    ? selectedKey
    : (rows.find((e) => e.kind === "listed")?.key ?? null);
  const selected = rows.find((e) => e.key === shownKey) ?? null;
  const selectedSent = selected != null && sent.has(selected.key);
  // The page shows one filled button (design rules §1): the banner's while
  // buying is off, else the panel's Buy while it has one, else Finalize.
  const panelBuys = selected?.kind === "listed" && !selectedSent && !gated && refusal === null;

  const finalizeSent = useMemo(() => finalizeSentIds(drafts), [drafts]);
  const purchases: PendingPurchase[] = names.flatMap((n) =>
    n.shakedex && !finalizeSent.has(n.shakedex.purchaseId)
      ? [{ name: n.name, shakedex: n.shakedex }]
      : [],
  );
  const boughtNames = new Set(names.filter((n) => n.shakedex).map((n) => n.name));
  const isBought = (e: Entry) => sent.has(e.key) || boughtNames.has(e.name);
  const lastPage = pages[pages.length - 1];
  const hasMore = marketShown && lastPage?.networkHasMarket === true && market.hasNextPage;

  let state: ListState = { kind: "rows" };
  // Pending, not loading: a query just enabled renders once before it fetches.
  if (market.isPending) {
    state = { kind: "loading", text: "Checking listings against your node…" };
  } else if (market.isError) {
    state = {
      kind: "message",
      title: "Couldn't load LearnHNS Market",
      text: mapError(market.error),
      testId: "market-error",
    };
  } else if (offline) {
    state = {
      kind: "message",
      title: "Listings can't be checked right now",
      text: "Namehold can't reach your node. Start it, or pick another in Settings › Connections; listings appear once it answers.",
      testId: "market-offline",
    };
  } else if (syncing) {
    const pct =
      node?.verification_progress != null
        ? ` (${Math.floor(node.verification_progress * 100)}%)`
        : "";
    state = {
      kind: "message",
      title: "Listings can't be checked yet",
      text: `Your node is still syncing${pct}. Namehold only shows listings it has verified itself, so they appear once the node catches up.`,
      testId: "market-unavailable",
    };
  } else if (first && !first.verified) {
    state = {
      kind: "message",
      title: "Listings can't be checked yet",
      text: "Namehold only shows listings it has verified itself, and checking needs a full or remote node. Connect one in Settings › Connections.",
      testId: "market-unavailable",
    };
  } else if (entries.length === 0) {
    state = {
      kind: "message",
      title: "Nothing is listed right now",
      text:
        first && !first.networkHasMarket
          ? "There is no LearnHNS Market on this network. If a seller sent you a listing directly, import it."
          : hasMore
            ? "None of the listings loaded so far can be bought. Load more to check the next ones, or import a listing a seller sent you."
            : hidden.length > 0
              ? "None of LearnHNS Market's listings can be bought right now; the list's foot says why. If a seller sent you a listing directly, import it."
              : "LearnHNS Market has no verified listings at the moment. If a seller sent you a listing directly, import it.",
      testId: "market-empty",
    };
  }

  const turnOnBuying = async () => {
    try {
      await updateSetting("shakedex_experimental", "true");
      showToast(BUYING_ON_TOAST, "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const retry = (e: Entry) => {
    if (e.kind !== "unchecked" || checkingKey) return;
    setCheckingKey(e.key);
    const at = fetchedAt;
    recheck.mutate(
      { kind: "text", json: e.listingJson },
      {
        onSuccess: (row) =>
          setRechecks((r) => ({
            at,
            rows: new Map(r.at === at ? r.rows : []).set(e.listingJson, row),
          })),
        onError: (err) => showToast(mapError(err), "error"),
        onSettled: () => setCheckingKey(null),
      },
    );
  };

  const onImported = (i: ImportedRow) => {
    setImported((prev) => [i, ...prev.filter((p) => p.row.listingJson !== i.row.listingJson)]);
    setQuery("");
    setFilter("all");
    setSelectedKey(entryKey(i.origin, i.row.listingJson));
    setImportOpen(false);
    showToast(`.${displayName(i.row.name)} imported and verified`, "success");
  };

  const onPay = useCallback(
    (pay: boolean) => {
      if (shownKey) setPayByKey((m) => new Map(m).set(shownKey, pay));
    },
    [shownKey],
  );

  const select = (key: string | null) => {
    setSelectedKey(key);
    if (key) document.getElementById(`listing-${key}`)?.scrollIntoView?.({ block: "nearest" });
  };

  const handler = useRef<(id: ActionId) => void>(() => {});
  handler.current = (id) => {
    if (id === "market:focusSearch") searchRef.current?.focus();
    else if (id === "market:import") setImportOpen(true);
    else if (id === "market:list:next") select(step(rows, shownKey, 1));
    else if (id === "market:list:prev") select(step(rows, shownKey, -1));
    else if (id === "market:primary" && !focusAnswersEnter()) primaryRef.current?.();
  };
  useEffect(() => subscribeAction((id) => handler.current(id)), []);

  return (
    <div className="space-y-5">
      {gated && <MarketGateBanner onTurnOn={() => void turnOnBuying()} />}
      <PendingPurchases
        purchases={purchases}
        finalizeRefusal={refusal}
        finalizingId={finalizingId}
        primary={!gated && !panelBuys}
        onFinalize={(p) =>
          void finalize(
            p.shakedex.purchaseId,
            () => `.${displayName(p.name)} finalize sent · it moves into your wallet once mined`,
          )
        }
      />
      <div className="flex flex-wrap items-start gap-5">
        <div className="min-w-0 flex-[999_1_540px]">
          <ListingsTable
            state={state}
            rows={rows}
            hidden={hidden}
            hiddenOpen={hiddenOpen}
            onToggleHidden={() => setHiddenOpen((o) => !o)}
            query={query}
            onQuery={setQuery}
            filter={filter}
            onFilter={setFilter}
            sort={sort}
            onSort={setSort}
            searchRef={searchRef}
            selectedKey={shownKey}
            onSelect={setSelectedKey}
            isBought={isBought}
            checkingKey={checkingKey}
            onRetry={retry}
            more={
              marketShown && lastPage?.networkHasMarket
                ? {
                    hasMore,
                    loading: market.isFetchingNextPage,
                    onLoadMore: () => void market.fetchNextPage(),
                  }
                : null
            }
          />
        </div>
        <ListingDetail
          // A new selection starts the fee rate over.
          key={shownKey ?? "none"}
          entry={selected}
          sent={selectedSent}
          refusal={refusal}
          gated={gated}
          onTurnOn={() => void turnOnBuying()}
          availableDoos={balances?.liquidDoos ?? null}
          pay={shownKey ? (payByKey.get(shownKey) ?? null) : null}
          onPay={onPay}
          checking={selected != null && checkingKey === selected.key}
          onRetry={() => selected && retry(selected)}
          onBought={(e) => {
            setSent((m) => new Map(m).set(e.key, e));
            showToast(`Purchase of .${displayName(e.name)} sent`, "success");
          }}
          primaryRef={primaryRef}
        />
      </div>
      {/* Mounted while open only: each opening starts empty. */}
      {importOpen && (
        <ImportListingDialog
          open
          onClose={() => setImportOpen(false)}
          onImported={onImported}
          linkRefusal={linkRefusal}
        />
      )}
    </div>
  );
}
