import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
const openMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...a: unknown[]) => openMock(...a),
  save: vi.fn(),
}));

import MarketPage from "../market/MarketPage";
import { IMPORT_BAD_FILE, IMPORT_BAD_LINK, MARKET_MAINNET_ONLY } from "../market/marketText";
import { dispatchAction } from "../../lib/actionBus";
import { useSettingsStore, DEFAULT_SETTINGS } from "../../stores/settings";
import { useUiStore } from "../../stores/ui";
import { makeNodeStatus } from "../../test/fixtures/nodeStatus";
import { makeAuctionRow, makeMarketPage, makeMarketRow } from "../../test/fixtures/listings";
import type { HiddenRow, MarketPage as Page, MarketRow, ShakedexNameState } from "../../types";

function profile(kind = "mnemonic_hot", network = "regtest") {
  return {
    id: "p1",
    label: "Primary",
    network,
    receiveAddress: "rs1q",
    watchOnly: kind === "watch_only_xpub",
    hasPassphrase: false,
    active: true,
    kind,
  };
}

// The row `read_shakedex_purchase_names` sends: no chain state of its own yet.
function purchaseRow(name: string, shakedex: Partial<ShakedexNameState> = {}) {
  return {
    name,
    state: null,
    height: null,
    renewal: null,
    owner: null,
    owner_address: "rs1qdest",
    registered: true,
    expired: null,
    stats: null,
    shakedex: {
      state: "awaitingFinalize",
      blocksRemaining: 142,
      purchaseId: `p-${name}`,
      purchaseDraftId: `draft-p-${name}`,
      priceDoos: 410_200_000,
      lockupBlocks: 288,
      wallClockTiming: true,
      ...shakedex,
    },
  };
}

const couldNotCheck = (name: string): HiddenRow => ({
  name,
  reason: { kind: "couldNotCheck", reason: "Node RPC error: operation timed out" },
  listingJson: JSON.stringify({ version: 2, name }),
});

interface World {
  kind?: string;
  network?: string;
  canSend?: boolean;
  /** The market's pages, by number from 1. */
  pages?: Page[];
  market?: () => Promise<unknown>;
  importRow?: (source: { kind: string }) => Promise<unknown>;
  names?: unknown[];
  drafts?: unknown[];
  node?: ReturnType<typeof makeNodeStatus>;
  balance?: number;
}

function mock(w: World = {}) {
  invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile(w.kind, w.network)]);
      case "get_write_capability": {
        const canSend = w.canSend ?? true;
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: canSend,
          canWrite: canSend,
          reason: null,
        });
      }
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: 0 });
      case "shakedex_list_market": {
        if (w.market) return w.market();
        const pages = w.pages ?? [makeMarketPage()];
        return Promise.resolve(pages[(args?.page as number) - 1]);
      }
      case "shakedex_import_listing":
        return w.importRow
          ? w.importRow(args?.source as { kind: string })
          : Promise.resolve(makeMarketRow({ name: "imported" }));
      case "shakedex_preview_purchase":
        return Promise.resolve({
          name: "x",
          priceDoos: 250_000_000,
          marketFee: null,
          networkFeeDoos: 10_000,
          totalDoos: 250_010_000,
          finalizeWait: "after a finalize, 10 blocks after the purchase is mined",
          lockupBlocks: 10,
          wallClockTiming: false,
          warnExpiry: false,
        });
      case "get_wallet_balances":
        return Promise.resolve({
          liquidDoos: w.balance ?? 10_000_000_000,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          immatureDoos: 0,
          immatureInBlocks: null,
          totalDoos: w.balance ?? 10_000_000_000,
        });
      case "read_names":
        return Promise.resolve(w.names ?? []);
      case "list_tx_drafts":
        return Promise.resolve(w.drafts ?? []);
      case "node_status":
        return Promise.resolve(w.node ?? makeNodeStatus({ connected: true, synced: true }));
      case "shakedex_build_purchase_draft":
        return Promise.resolve({ id: "draft-1" });
      case "shakedex_build_purchase_finalize_draft":
        return Promise.resolve({ id: "fin-1", summary: { warnings: [] } });
      case "sign_tx_draft":
        return Promise.resolve({ id: "fin-1" });
      case "broadcast_tx_draft":
        return Promise.resolve({ txid: "ab".repeat(32) });
      default:
        return Promise.resolve(null);
    }
  });
}

function newClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

function settings(over: Record<string, string> = {}) {
  useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS, ...over }, loaded: true });
}

function renderPage(entry = "/market", qc = newClient()) {
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[entry]}>
        <MarketPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const rows = () => screen.getAllByTestId("market-row");
const rowNamed = (name: string) => rows().find((r) => within(r).queryByText(`.${name}`) !== null)!;
const toasts = () => useUiStore.getState().toastQueue.map((t) => t.message);

beforeEach(() => {
  invokeMock.mockReset();
  openMock.mockReset();
  useUiStore.setState({ toastQueue: [] });
  settings();
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  useSettingsStore.setState({ settings: null, loaded: false });
});

describe("MarketPage: tabs", () => {
  it("opens on Browse, and on My listings with ?tab=mine", async () => {
    mock();
    renderPage("/market?tab=mine");
    expect(
      await screen.findByText("No listings yet. Sell a name from its actions in Owned Names."),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("market-row")).toBeNull();
    expect(screen.queryByRole("button", { name: "Import listing…" })).toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "Browse" }));
    await screen.findAllByText(".dexreviews");
    expect(screen.getByRole("button", { name: "Import listing…" })).toBeInTheDocument();
  });
});

describe("MarketPage: the list", () => {
  it("shows each verified listing's price, next price and check, and selects the first", async () => {
    mock({
      pages: [
        makeMarketPage({
          rows: [makeMarketRow({ name: "plain" }), makeAuctionRow({ name: "falling" })],
        }),
      ],
    });
    renderPage();
    await screen.findAllByText(".plain");
    const falling = rowNamed("falling");
    expect(within(falling).getByTestId("market-row-price")).toHaveTextContent("1,200 HNS");
    expect(falling).toHaveTextContent("↓ 1,100 HNS in ~6 h");
    expect(within(falling).getByTestId("market-row-check")).toHaveTextContent("✓ Verified");
    expect(rowNamed("plain")).toHaveTextContent("Fixed price");
    // Lowest price first, and selected.
    expect(rows()[0]).toHaveTextContent(".plain");
    expect(rows()[0]).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("listbox", { name: "Listings" })).toBeInTheDocument();
    expect(within(screen.getByTestId("listing-detail")).getByText(".plain")).toBeInTheDocument();
  });

  it("selects a row on click and shows it in the panel", async () => {
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow({ name: "plain" }), makeAuctionRow()] })],
    });
    renderPage();
    await screen.findAllByText(".coffee");
    fireEvent.click(rowNamed("coffee"));
    expect(rowNamed("coffee")).toHaveAttribute("aria-selected", "true");
    expect(
      within(screen.getByTestId("listing-detail")).getByTestId("detail-price"),
    ).toHaveTextContent("1,200 HNS");
  });

  it("filters by kind and finds a name, and says when nothing matches", async () => {
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow({ name: "plain" }), makeAuctionRow()] })],
    });
    renderPage();
    await screen.findAllByText(".coffee");
    fireEvent.click(screen.getByTestId("market-filter-drops"));
    expect(rows().map((r) => r.textContent)).toEqual([expect.stringContaining(".coffee")]);
    fireEvent.click(screen.getByTestId("market-filter-all"));
    fireEvent.change(screen.getByTestId("market-search"), { target: { value: ".pla" } });
    expect(rows()).toHaveLength(1);
    fireEvent.change(screen.getByTestId("market-search"), { target: { value: "zzz" } });
    expect(screen.getByTestId("market-no-match")).toHaveTextContent("No listings match");
  });

  it("sorts by the next price drop", async () => {
    mock({
      pages: [
        makeMarketPage({
          rows: [
            makeMarketRow({ name: "plain", currentPrice: 1 }),
            makeAuctionRow({ name: "falling" }),
          ],
        }),
      ],
    });
    renderPage();
    await screen.findAllByText(".falling");
    expect(rows()[0]).toHaveTextContent(".plain");
    fireEvent.change(screen.getByTestId("market-sort"), { target: { value: "drop" } });
    expect(rows()[0]).toHaveTextContent(".falling");
  });

  it("tags a listing being bought, one expiring soon, and an import", async () => {
    const soon = makeMarketRow({ name: "soon" });
    if (soon.verdict.verdict === "buyable") soon.verdict.warnExpiry = true;
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow({ name: "bought" }), soon] })],
      names: [purchaseRow("bought")],
    });
    renderPage();
    await waitFor(() =>
      expect(within(rowNamed("bought")).getByTestId("market-row-tag")).toHaveTextContent("Bought"),
    );
    expect(within(rowNamed("soon")).getByTestId("market-row-tag")).toHaveTextContent(
      "Expires soon",
    );
  });

  it("keeps a bought listing, its Sent panel and its tag when the market then reads it as sold", async () => {
    const row = makeMarketRow({ name: "bought", currentPrice: 1 });
    const other = makeMarketRow({ name: "other", currentPrice: 2 });
    const bought = () => calls("shakedex_build_purchase_draft").length > 0;
    mock({
      // hsd's getCoin reads a lock coin spent in the mempool as missing.
      market: () =>
        Promise.resolve(
          bought()
            ? makeMarketPage({
                rows: [other],
                hiddenRows: [
                  {
                    name: "bought",
                    reason: { kind: "soldOrCancelled" },
                    listingJson: row.listingJson,
                  },
                ],
              })
            : makeMarketPage({ rows: [row, other] }),
        ),
    });
    renderPage();
    await screen.findAllByText(".bought");
    const buyButton = screen.getByTestId("purchase-buy");
    await waitFor(() => expect(buyButton).toBeEnabled());
    fireEvent.click(buyButton);
    expect(await screen.findByTestId("detail-sent")).toBeInTheDocument();
    await waitFor(() => expect(calls("shakedex_list_market").length).toBeGreaterThan(1));
    // Let the refetched page land.
    await act(async () => {});
    expect(screen.getByTestId("detail-sent")).toBeInTheDocument();
    expect(within(rowNamed("bought")).getByTestId("market-row-tag")).toHaveTextContent("Bought");
    expect(rowNamed("bought")).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByTestId("hidden-toggle")).toBeNull();
  });

  it("gives each row a DOM id the listbox can point at, whatever its file holds", async () => {
    const spaced = JSON.stringify({ version: 2, name: "spaced", note: "a b c" }, null, 2);
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow({ name: "spaced", listingJson: spaced })] })],
    });
    renderPage();
    await screen.findAllByText(".spaced");
    const listbox = screen.getByRole("listbox", { name: "Listings" });
    const id = listbox.getAttribute("aria-activedescendant")!;
    expect(id).toMatch(/^\S+$/);
    expect(document.getElementById(id)).toBe(rowNamed("spaced"));
  });

  it("renders a price at the u64 maximum without throwing", async () => {
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow({ currentPrice: 18446744073709552000 })] })],
    });
    renderPage();
    await screen.findAllByText(".dexreviews");
    expect(screen.getByTestId("market-row-price")).toHaveTextContent("HNS");
  });
});

describe("MarketPage: listings that cannot be bought", () => {
  it("counts each hidden reason in the footer and lists them when expanded, unselectable", async () => {
    mock({
      pages: [
        makeMarketPage({
          hiddenRows: [
            { name: "gone", reason: { kind: "soldOrCancelled" }, listingJson: "{}" },
            { name: "gone2", reason: { kind: "soldOrCancelled" }, listingJson: "{}" },
            {
              name: "bad",
              reason: { kind: "failedVerification", reason: "the signature does not match" },
              listingJson: "{}",
            },
            {
              name: null,
              reason: { kind: "notYetValid", firstValidInSecs: 18_000 },
              listingJson: null,
            },
          ],
        }),
      ],
    });
    renderPage();
    const toggle = await screen.findByTestId("hidden-toggle");
    expect(toggle).toHaveTextContent(
      "▸ 4 unavailable listings hidden: 2 sold, 1 failed the check, 1 not valid yet",
    );
    expect(screen.queryByTestId("hidden-row")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("▾ Hide 4 unavailable listings");
    const hidden = screen.getAllByTestId("hidden-row");
    expect(hidden.map((h) => h.textContent)).toEqual([
      ".gone—Already sold or cancelled",
      ".gone2—Already sold or cancelled",
      ".bad—Failed the check: the signature does not match",
      "Unnamed listing—Not valid yet: the first price becomes valid in ~5 h",
    ]);
    expect(hidden[0]).not.toHaveAttribute("role", "option");
  });

  it("lists a listing the node could not check without a price, and Retry checks it again", async () => {
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow()], hiddenRows: [couldNotCheck("cooljobs")] })],
      importRow: () =>
        Promise.resolve(
          makeMarketRow({ name: "cooljobs", listingJson: couldNotCheck("cooljobs").listingJson! }),
        ),
    });
    renderPage();
    await screen.findAllByText(".cooljobs");
    const row = rowNamed("cooljobs");
    expect(within(row).getByTestId("market-row-price")).toHaveTextContent("—");
    expect(within(row).getByTestId("market-row-check")).toHaveTextContent("Couldn't check");
    fireEvent.click(within(row).getByTestId("market-row-retry"));
    // Retry does not select the row.
    expect(row).toHaveAttribute("aria-selected", "false");
    await waitFor(() =>
      expect(within(rowNamed("cooljobs")).getByTestId("market-row-check")).toHaveTextContent(
        "✓ Verified",
      ),
    );
    expect(calls("shakedex_import_listing")[0]![1]).toEqual({
      source: { kind: "text", json: couldNotCheck("cooljobs").listingJson },
    });
  });
});

describe("MarketPage: a Retry's answer", () => {
  it("lasts until the market is fetched again, and no longer", async () => {
    const qc = newClient();
    mock({
      pages: [makeMarketPage({ rows: [makeMarketRow()], hiddenRows: [couldNotCheck("cooljobs")] })],
      importRow: () =>
        Promise.resolve(
          makeMarketRow({ name: "cooljobs", listingJson: couldNotCheck("cooljobs").listingJson! }),
        ),
    });
    renderPage("/market", qc);
    await screen.findAllByText(".cooljobs");
    fireEvent.click(within(rowNamed("cooljobs")).getByTestId("market-row-retry"));
    await waitFor(() =>
      expect(within(rowNamed("cooljobs")).getByTestId("market-row-check")).toHaveTextContent(
        "✓ Verified",
      ),
    );
    // The market fetched again still could not check it: that is the news.
    await act(() => qc.invalidateQueries({ queryKey: ["shakedex", "market"] }));
    await waitFor(() =>
      expect(within(rowNamed("cooljobs")).getByTestId("market-row-check")).toHaveTextContent(
        "Couldn't check",
      ),
    );
  });
});

describe("MarketPage: Load more", () => {
  it("appends the next page to the same list, then says every listing is loaded", async () => {
    mock({
      pages: [
        makeMarketPage({ rows: [makeMarketRow({ name: "first" })], pageCount: 2 }),
        makeMarketPage({ rows: [makeMarketRow({ name: "second" })], page: 2, pageCount: 2 }),
      ],
    });
    renderPage();
    await screen.findAllByText(".first");
    expect(screen.queryByTestId("market-all-loaded")).toBeNull();
    fireEvent.click(screen.getByTestId("market-load-more"));
    expect((await screen.findAllByText(".second"))[0]).toBeInTheDocument();
    expect(rowNamed("first")).toBeInTheDocument();
    expect(screen.getByTestId("market-all-loaded")).toHaveTextContent(
      "All LearnHNS listings loaded",
    );
    expect(calls("shakedex_list_market").map((c) => (c[1] as { page: number }).page)).toEqual([
      1, 2,
    ]);
  });

  it("keeps Load more and the hidden listings when a first page has nothing to buy", async () => {
    mock({
      pages: [
        makeMarketPage({
          rows: [],
          hiddenRows: [{ name: "gone", reason: { kind: "soldOrCancelled" }, listingJson: "{}" }],
          pageCount: 2,
        }),
        makeMarketPage({ rows: [makeMarketRow({ name: "second" })], page: 2, pageCount: 2 }),
      ],
    });
    renderPage();
    expect(await screen.findByTestId("market-empty")).toHaveTextContent(
      "None of the listings loaded so far can be bought. Load more to check the next ones, or import a listing a seller sent you.",
    );
    expect(screen.getByTestId("hidden-toggle")).toHaveTextContent("1 sold");
    fireEvent.click(screen.getByTestId("market-load-more"));
    expect((await screen.findAllByText(".second"))[0]).toBeInTheDocument();
    expect(screen.queryByTestId("market-empty")).toBeNull();
  });

  it("shows no Load more where the network has no market", async () => {
    mock({ pages: [makeMarketPage({ rows: [], networkHasMarket: false })] });
    renderPage();
    expect(await screen.findByTestId("market-empty")).toHaveTextContent(
      "There is no LearnHNS Market on this network. If a seller sent you a listing directly, import it.",
    );
    expect(screen.queryByTestId("market-load-more")).toBeNull();
  });
});

describe("MarketPage: list states", () => {
  it("shows skeleton rows and what is going on while the first page loads", async () => {
    mock({ market: () => new Promise(() => {}) });
    renderPage();
    const loading = await screen.findByTestId("market-loading");
    expect(loading).toHaveAttribute("aria-busy", "true");
    expect(loading).toHaveTextContent("Checking listings against your node…");
    expect(
      screen.getByText(
        "Pick a listing to see its price, what your node checked, and what buying involves.",
      ),
    ).toBeInTheDocument();
  });

  it("says listings cannot be checked without a full or remote node (SPV, Explorer)", async () => {
    mock({
      pages: [
        makeMarketPage({
          verified: false,
          rows: [makeMarketRow({ verdict: { verdict: "hidden", kind: "unverified" } })],
        }),
      ],
    });
    renderPage();
    expect(await screen.findByTestId("market-unavailable")).toHaveTextContent(
      "Listings can't be checked yet",
    );
    expect(screen.queryByTestId("market-row")).toBeNull();
  });

  it("says the local node is still syncing, with its progress", async () => {
    settings({ chain_source: "local_node" });
    mock({
      node: makeNodeStatus({ connected: true, synced: false, verification_progress: 0.823 }),
    });
    renderPage();
    expect(await screen.findByTestId("market-unavailable")).toHaveTextContent(
      "Your node is still syncing (82%). Namehold only shows listings it has verified itself, so they appear once the node catches up.",
    );
  });

  it("says nothing is listed when the market has no verified listing", async () => {
    mock({ pages: [makeMarketPage({ rows: [] })] });
    renderPage();
    expect(await screen.findByTestId("market-empty")).toHaveTextContent(
      "LearnHNS Market has no verified listings at the moment.",
    );
  });

  it("says why the market could not be loaded", async () => {
    mock({ market: () => Promise.reject("HTTP error: LearnHNS Market returned 502") });
    renderPage();
    expect(await screen.findByTestId("market-error")).toHaveTextContent(/502/);
  });

  it("still lists an import, selectable, while the market cannot be loaded", async () => {
    mock({ market: () => Promise.reject("HTTP error: LearnHNS Market returned 502") });
    renderPage();
    await screen.findByTestId("market-error");
    fireEvent.click(screen.getByTestId("market-import"));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("listing-paste"), {
      target: { value: '{"version":2,"name":"imported"}' },
    });
    fireEvent.click(within(dialog).getByTestId("import-listing-text"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("market-error")).toBeInTheDocument();
    expect(rowNamed("imported")).toHaveAttribute("aria-selected", "true");
    expect(within(screen.getByTestId("listing-detail")).getByText(".imported")).toBeInTheDocument();
  });

  it("says the local node is not answering, instead of a Retry on every row", async () => {
    settings({ chain_source: "local_node" });
    mock({
      node: makeNodeStatus({ connected: false }),
      pages: [makeMarketPage({ rows: [], hiddenRows: [couldNotCheck("a"), couldNotCheck("b")] })],
    });
    renderPage();
    expect(await screen.findByTestId("market-offline")).toHaveTextContent(
      "Namehold can't reach your node. Start it, or pick another in Settings › Connections; listings appear once it answers.",
    );
    expect(screen.queryByTestId("market-row")).toBeNull();
  });
});

describe("MarketPage: the mainnet gate", () => {
  it("offers to turn buying on from the banner, and the panel's button does it too", async () => {
    settings({ shakedex_experimental: "false" });
    mock({ network: "mainnet" });
    renderPage();
    expect(await screen.findByTestId("market-gate-banner")).toHaveTextContent(
      "Buying on mainnet is off.",
    );
    // One filled button on the page: the panel's is the fix-it kind.
    expect(await screen.findByTestId("purchase-buy")).toHaveTextContent("Turn on buying");
    fireEvent.click(screen.getByTestId("market-gate-turn-on"));
    await waitFor(() => expect(screen.queryByTestId("market-gate-banner")).toBeNull());
    expect(calls("update_setting")[0]![1]).toEqual({ key: "shakedex_experimental", value: "true" });
    expect(toasts()).toContain("Mainnet buying turned on · also in Settings › Shakedex");
    await waitFor(() =>
      expect(screen.getByTestId("purchase-buy")).toHaveTextContent(/^Buy \.dexreviews for/),
    );
  });

  it("shows no gate off mainnet, and Buy is enabled", async () => {
    settings({ shakedex_experimental: "false" });
    mock({ network: "regtest" });
    renderPage();
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    expect(screen.queryByTestId("market-gate-banner")).toBeNull();
  });
});

describe("MarketPage: Your purchases", () => {
  it("shows each purchase's lockup progress, and none when there are none", async () => {
    mock({
      names: [
        purchaseRow("waiting", { blocksRemaining: 142 }),
        purchaseRow("pending", { state: "unconfirmed", blocksRemaining: null }),
      ],
    });
    renderPage();
    const card = await screen.findByTestId("pending-purchases");
    const [waiting, pending] = within(card).getAllByTestId("pending-purchase");
    expect(waiting).toHaveTextContent(".waiting");
    expect(waiting).toHaveTextContent("Paid 410.2 HNS");
    expect(waiting).toHaveTextContent("146 of 288 blocks");
    expect(waiting).toHaveTextContent("~1 d 0 h left");
    expect(waiting).not.toHaveTextContent("blocks left");
    expect(pending).toHaveTextContent("Waiting for the purchase to be mined");
    expect(within(card).queryByTestId("pending-purchase-finalize")).toBeNull();
  });

  it("counts a regtest lockup in blocks only, as regtest mines on demand (R10)", async () => {
    mock({
      names: [
        purchaseRow("waiting", { blocksRemaining: 4, lockupBlocks: 10, wallClockTiming: false }),
      ],
    });
    renderPage();
    const [waiting] = within(await screen.findByTestId("pending-purchases")).getAllByTestId(
      "pending-purchase",
    );
    expect(waiting).toHaveTextContent("6 of 10 blocks");
    expect(waiting).toHaveTextContent("4 blocks left");
    expect(waiting).not.toHaveTextContent("~");
  });

  it("hides the card without purchases", async () => {
    mock();
    renderPage();
    await screen.findAllByText(".dexreviews");
    expect(screen.queryByTestId("pending-purchases")).toBeNull();
  });

  it("finalizes a ready purchase even while mainnet buying is off", async () => {
    settings({ shakedex_experimental: "false" });
    mock({ network: "mainnet", names: [purchaseRow("ready", { blocksRemaining: 0 })] });
    renderPage();
    const button = await screen.findByTestId("pending-purchase-finalize");
    expect(screen.getByTestId("pending-purchase-progress")).toHaveTextContent("Ready to finalize");
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    await waitFor(() =>
      expect(calls("shakedex_build_purchase_finalize_draft")[0]![1]).toEqual({
        purchaseId: "p-ready",
        feeRate: null,
      }),
    );
    await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(1));
    await waitFor(() =>
      expect(toasts()).toContain(".ready finalize sent · it moves into your wallet once mined"),
    );
  });

  it("fills Finalize only while no other filled button is on the page", async () => {
    const filled = () => document.querySelectorAll("button.bg-blue-600");
    const ready = [purchaseRow("ready", { blocksRemaining: 0 })];

    // Nothing to buy: Finalize is the page's one primary.
    mock({ names: ready, pages: [makeMarketPage({ rows: [] })] });
    const { unmount } = renderPage();
    const finalize = await screen.findByTestId("pending-purchase-finalize");
    await waitFor(() => expect(finalize).toHaveClass("bg-blue-600"));
    expect(filled()).toHaveLength(1);
    unmount();

    // A buyable listing in the panel: its Buy is.
    mock({ names: ready });
    const second = renderPage();
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toHaveClass("bg-blue-600"));
    expect(screen.getByTestId("pending-purchase-finalize")).not.toHaveClass("bg-blue-600");
    expect(filled()).toHaveLength(1);
    second.unmount();

    // Mainnet buying off: the banner's Turn on buying is.
    settings({ shakedex_experimental: "false" });
    mock({ network: "mainnet", names: ready, pages: [makeMarketPage({ rows: [] })] });
    renderPage();
    await screen.findByTestId("market-gate-banner");
    expect(await screen.findByTestId("pending-purchase-finalize")).not.toHaveClass("bg-blue-600");
    expect(filled()).toHaveLength(1);
  });

  it("refuses Finalize with the backend's sentence where Shakedex cannot run", async () => {
    mock({ kind: "ledger_hardware", names: [purchaseRow("ready", { blocksRemaining: 0 })] });
    renderPage();
    const button = await screen.findByTestId("pending-purchase-finalize");
    await waitFor(() => expect(button).toBeDisabled());
    expect(screen.getByTestId("pending-purchases")).toHaveTextContent(
      "Shakedex works with a recovery-phrase wallet for now",
    );
  });

  it("drops a purchase whose finalize is already sent", async () => {
    mock({
      names: [purchaseRow("ready", { blocksRemaining: 0 })],
      drafts: [
        {
          id: "fin-1",
          walletProfileId: "p1",
          action: "shakedex_purchase_finalize",
          status: "broadcasted",
          summary: { purchaseId: "p-ready" },
          errorMessage: null,
          txid: null,
          confirmationHeight: null,
          createdAt: "2026-01-01",
          purchaseLostReason: null,
        },
      ],
    });
    renderPage();
    await screen.findAllByText(".dexreviews");
    await waitFor(() => expect(calls("list_tx_drafts").length).toBeGreaterThan(0));
    await waitFor(() => expect(screen.queryByTestId("pending-purchases")).toBeNull());
  });
});

describe("MarketPage: Import", () => {
  async function openImport() {
    fireEvent.click(await screen.findByRole("button", { name: "Import listing…" }));
    return screen.getByRole("dialog");
  }

  it("imports a pasted listing: closes, puts it in the list selected, and says so", async () => {
    mock({
      importRow: () =>
        Promise.resolve(makeMarketRow({ name: "imported", currentPrice: 999_000_000_000 })),
    });
    renderPage();
    await screen.findAllByText(".dexreviews");
    fireEvent.change(screen.getByTestId("market-search"), { target: { value: "dex" } });
    const dialog = await openImport();
    fireEvent.change(within(dialog).getByTestId("listing-paste"), {
      target: { value: '{"version":2,"name":"imported"}' },
    });
    fireEvent.click(within(dialog).getByTestId("import-listing-text"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("market-search")).toHaveValue("");
    expect(rowNamed("imported")).toHaveAttribute("aria-selected", "true");
    expect(within(rowNamed("imported")).getByTestId("market-row-tag")).toHaveTextContent(
      "From file",
    );
    expect(calls("shakedex_import_listing")[0]![1]).toEqual({
      source: { kind: "text", json: '{"version":2,"name":"imported"}' },
    });
    expect(toasts()).toContain(".imported imported and verified");
  });

  it("imports a market link as a link", async () => {
    mock({
      network: "mainnet",
      importRow: () => Promise.resolve(makeMarketRow({ name: "linked" })),
    });
    settings({ shakedex_experimental: "true" });
    renderPage();
    const dialog = await openImport();
    fireEvent.change(within(dialog).getByTestId("listing-paste"), {
      target: { value: "https://market.learnhns.com/listing/linked" },
    });
    fireEvent.click(within(dialog).getByTestId("import-listing-link"));
    await screen.findAllByText(".linked");
    expect(within(rowNamed("linked")).getByTestId("market-row-tag")).toHaveTextContent("From link");
    expect(calls("shakedex_import_listing")[0]![1]).toEqual({
      source: { kind: "link", url: "https://market.learnhns.com/listing/linked" },
    });
  });

  it("refuses a link off LearnHNS and text that is no listing file, without asking the node", async () => {
    mock({ network: "mainnet" });
    renderPage();
    const dialog = await openImport();
    const paste = within(dialog).getByTestId("listing-paste");
    fireEvent.change(paste, { target: { value: "https://example.com/listing/x" } });
    fireEvent.click(within(dialog).getByTestId("import-listing-link"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(IMPORT_BAD_LINK);
    fireEvent.change(paste, { target: { value: "not json" } });
    fireEvent.click(within(dialog).getByTestId("import-listing-text"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(IMPORT_BAD_FILE);
    expect(calls("shakedex_import_listing")).toHaveLength(0);
  });

  it("off mainnet disables link import with the backend's reason", async () => {
    mock({ network: "regtest" });
    renderPage();
    const dialog = await openImport();
    fireEvent.change(within(dialog).getByTestId("listing-paste"), {
      target: { value: "https://market.learnhns.com/listing/x" },
    });
    expect(within(dialog).getByTestId("import-listing-link")).toBeDisabled();
    expect(within(dialog).getByText(MARKET_MAINNET_ONLY)).toBeInTheDocument();
  });

  it("says why an imported listing cannot be bought, and keeps it out of the list", async () => {
    mock({
      importRow: () =>
        Promise.resolve(
          makeMarketRow({
            name: "late",
            verdict: { verdict: "hidden", kind: "expiresBeforeFinalize" },
          }),
        ),
    });
    renderPage();
    const dialog = await openImport();
    fireEvent.change(within(dialog).getByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(within(dialog).getByTestId("import-listing-text"));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The name expires before a purchase could be finalized.",
    );
    expect(screen.queryByText(".late")).toBeNull();
  });

  it("gives an SPV import the backend's unverified verdict", async () => {
    mock({
      importRow: () =>
        Promise.resolve(
          makeMarketRow({ name: "spv", verdict: { verdict: "hidden", kind: "unverified" } }),
        ),
    });
    renderPage();
    const dialog = await openImport();
    fireEvent.change(within(dialog).getByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(within(dialog).getByTestId("import-listing-text"));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Not verified — needs a full or remote node.",
    );
  });

  it("imports a chosen file by its path", async () => {
    openMock.mockResolvedValue("/tmp/listing.json");
    mock({ importRow: () => Promise.resolve(makeMarketRow({ name: "fromfile" })) });
    renderPage();
    const dialog = await openImport();
    fireEvent.click(within(dialog).getByTestId("import-listing-file"));
    await screen.findAllByText(".fromfile");
    expect(calls("shakedex_import_listing")[0]![1]).toEqual({
      source: { kind: "file", path: "/tmp/listing.json" },
    });
  });

  it("stays open while the node checks it", async () => {
    mock({ importRow: () => new Promise(() => {}) });
    renderPage();
    const dialog = await openImport();
    fireEvent.change(within(dialog).getByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(within(dialog).getByTestId("import-listing-text"));
    expect(await within(dialog).findByText("Checking…")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});

describe("MarketPage: hotkeys", () => {
  it("moves the selection with j/k, focuses the search with /, and opens Import with i", async () => {
    mock({
      pages: [
        makeMarketPage({
          rows: [
            makeMarketRow({ name: "aaa", currentPrice: 1 }),
            makeMarketRow({ name: "bbb", currentPrice: 2 }),
          ],
        }),
      ],
    });
    renderPage();
    await screen.findAllByText(".bbb");
    expect(rows()[0]).toHaveAttribute("aria-selected", "true");
    act(() => dispatchAction("market:list:next"));
    expect(rows()[1]).toHaveAttribute("aria-selected", "true");
    act(() => dispatchAction("market:list:next"));
    expect(rows()[1]).toHaveAttribute("aria-selected", "true");
    act(() => dispatchAction("market:list:prev"));
    expect(rows()[0]).toHaveAttribute("aria-selected", "true");
    act(() => dispatchAction("market:focusSearch"));
    expect(screen.getByTestId("market-search")).toHaveFocus();
    act(() => dispatchAction("market:import"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});

// A row the backend never sends (rows are buyable or, in SPV, unverified),
// kept so a hidden verdict in `rows` still lands in the hidden list.
it("hides a row whose verdict is hidden even when it comes in `rows`", async () => {
  const sold: MarketRow = makeMarketRow({
    name: "sold",
    verdict: { verdict: "hidden", kind: "soldOrCancelled" },
  });
  mock({ pages: [makeMarketPage({ rows: [makeMarketRow(), sold] })] });
  renderPage();
  await screen.findAllByText(".dexreviews");
  expect(screen.queryByText(".sold")).toBeNull();
  expect(screen.getByTestId("hidden-toggle")).toHaveTextContent("1 sold");
});
