import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

import MarketPage from "../market/MarketPage";
import { useSettingsStore, DEFAULT_SETTINGS } from "../../stores/settings";

const noHidden = {
  soldOrCancelled: 0,
  failedVerification: 0,
  expiresBeforeFinalize: 0,
  notYetValid: 0,
  couldNotCheck: 0,
};

function row(overrides: Record<string, unknown> = {}) {
  return {
    listingJson: '{"name":"dexreviews"}',
    name: "dexreviews",
    verdict: {
      verdict: "buyable",
      currentStep: 0,
      nextStep: null,
      lockValue: 250_000_000,
      nameHeight: 1,
      expiryEnd: 100000,
      warnExpiry: false,
      mtp: 0,
      tip: 0,
    },
    kind: "buyNow",
    currentPrice: 250_000_000,
    nextPrice: null,
    nextValidInSecs: null,
    floorPrice: 250_000_000,
    steps: [],
    expiresAt: null,
    ...overrides,
  };
}

function page(overrides: Record<string, unknown> = {}) {
  return {
    rows: [row()],
    hidden: noHidden,
    verified: true,
    networkHasMarket: true,
    hiddenRows: [],
    page: 1,
    pageCount: 1,
    ...overrides,
  };
}

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

function writeCapability(broadcasterAvailable = true) {
  return {
    signerUnlocked: true,
    broadcasterAvailable,
    canWrite: broadcasterAvailable,
    reason: null,
  };
}

function mockMarket(p: unknown, kind = "mnemonic_hot", canSend = true, network = "regtest") {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "list_wallet_profiles") return Promise.resolve([profile(kind, network)]);
    if (cmd === "get_write_capability") return Promise.resolve(writeCapability(canSend));
    if (cmd === "shakedex_list_market") return Promise.resolve(p);
    if (cmd === "shakedex_import_listing") return Promise.resolve(row({ name: "imported" }));
    return Promise.resolve(null);
  });
}

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/market"]}>
        <MarketPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("MarketPage", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("shows buyable rows in a canonical table and a hidden counter", async () => {
    mockMarket(
      page({
        hidden: { ...noHidden, soldOrCancelled: 28, failedVerification: 4 },
        hiddenRows: [{ name: "gone", reason: { kind: "soldOrCancelled" } }],
      }),
    );
    renderPage();
    expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
    // One line per listing, its status in words.
    expect(screen.getAllByTestId("market-row")).toHaveLength(1);
    expect(screen.getByTestId("market-status")).toHaveTextContent("Ready to buy");
    expect(
      screen.getByText("Hidden 32: 28 already sold or cancelled, 4 failed verification"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("market-buy")).toBeEnabled();
  });

  it("pages through a market longer than one page", async () => {
    invokeMock.mockImplementation((cmd: string, args?: { page?: number }) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile()]);
      if (cmd === "get_write_capability") return Promise.resolve(writeCapability());
      if (cmd === "shakedex_list_market") {
        const n = args?.page ?? 1;
        return Promise.resolve(page({ page: n, pageCount: 3, rows: [row({ name: `name${n}` })] }));
      }
      return Promise.resolve(null);
    });
    renderPage();
    expect(await screen.findByText(".name1")).toBeInTheDocument();
    expect(screen.getByTestId("market-page-of")).toHaveTextContent("Page 1 of 3");
    expect(screen.getByTestId("market-prev")).toBeDisabled();

    fireEvent.click(screen.getByTestId("market-next"));
    expect(await screen.findByText(".name2")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("shakedex_list_market", { page: 2 });
    expect(screen.getByTestId("market-page-of")).toHaveTextContent("Page 2 of 3");

    fireEvent.click(screen.getByTestId("market-next"));
    expect(await screen.findByText(".name3")).toBeInTheDocument();
    expect(screen.getByTestId("market-next")).toBeDisabled();

    fireEvent.click(screen.getByTestId("market-prev"));
    expect(await screen.findByText(".name2")).toBeInTheDocument();
  });

  it("shows that the market is being checked while the first page loads", async () => {
    let answer: (p: unknown) => void = () => {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile()]);
      if (cmd === "get_write_capability") return Promise.resolve(writeCapability());
      if (cmd === "shakedex_list_market") return new Promise((resolve) => (answer = resolve));
      return Promise.resolve(null);
    });
    renderPage();
    expect(await screen.findByTestId("market-loading")).toHaveTextContent(
      "Checking listings against your node",
    );
    expect(screen.queryByTestId("market-page-of")).toBeNull();
    expect(screen.queryByText(/No buyable listings/)).toBeNull();
    answer(page({ pageCount: 2 }));
    expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
    expect(screen.queryByTestId("market-loading")).toBeNull();
  });

  it("while another page loads, says so and holds the pager", async () => {
    let answerPage2: (p: unknown) => void = () => {};
    invokeMock.mockImplementation((cmd: string, args?: { page?: number }) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile()]);
      if (cmd === "get_write_capability") return Promise.resolve(writeCapability());
      if (cmd === "shakedex_list_market") {
        const n = args?.page ?? 1;
        if (n === 2) return new Promise((resolve) => (answerPage2 = resolve));
        return Promise.resolve(page({ page: n, pageCount: 3, rows: [row({ name: `name${n}` })] }));
      }
      return Promise.resolve(null);
    });
    renderPage();
    expect(await screen.findByText(".name1")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("market-next"));
    await waitFor(() =>
      expect(screen.getByTestId("market-page-of")).toHaveTextContent("Loading page 2…"),
    );
    expect(screen.getByTestId("market-next")).toBeDisabled();
    expect(screen.getByTestId("market-prev")).toBeDisabled();
    answerPage2(page({ page: 2, pageCount: 3, rows: [row({ name: "name2" })] }));
    expect(await screen.findByText(".name2")).toBeInTheDocument();
    expect(screen.getByTestId("market-page-of")).toHaveTextContent("Page 2 of 3");
  });

  it("says a page without buyable listings is only this page", async () => {
    mockMarket(page({ rows: [], page: 1, pageCount: 2 }));
    renderPage();
    expect(await screen.findByText("No buyable listings on this page.")).toBeInTheDocument();
    expect(screen.getByTestId("market-next")).toBeEnabled();
  });

  it("does not call a page empty of buyable listings when some could not be checked", async () => {
    mockMarket(
      page({
        rows: [],
        hidden: { ...noHidden, couldNotCheck: 3 },
        hiddenRows: [],
      }),
    );
    renderPage();
    expect(
      await screen.findByText(
        "No listing right now could be shown as buyable; 3 could not be checked against your node.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("No buyable listings right now.")).toBeNull();
  });

  it("names every kind of hidden listing in the counter", async () => {
    mockMarket(
      page({
        hidden: {
          soldOrCancelled: 1,
          failedVerification: 2,
          expiresBeforeFinalize: 3,
          notYetValid: 4,
          couldNotCheck: 5,
        },
      }),
    );
    renderPage();
    expect(
      await screen.findByText(
        "Hidden 15: 1 already sold or cancelled, 2 failed verification, 3 expire before they can be finalized, 4 not valid yet, 5 could not be checked",
      ),
    ).toBeInTheDocument();
  });

  it("says why the market could not be loaded", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile()]);
      if (cmd === "shakedex_list_market")
        return Promise.reject("LearnHNS Market request failed: HTTP 502");
      return Promise.resolve(null);
    });
    renderPage();
    expect(await screen.findByText("Could not load the market")).toBeInTheDocument();
    expect(screen.getByText("LearnHNS Market request failed: HTTP 502")).toBeInTheDocument();
  });

  it("shows no pager for a single page", async () => {
    mockMarket(page());
    renderPage();
    expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
    expect(screen.queryByTestId("market-next")).not.toBeInTheDocument();
  });

  it("expands the hidden counter into names and reasons without Buy", async () => {
    mockMarket(
      page({
        rows: [],
        hidden: { ...noHidden, soldOrCancelled: 1, failedVerification: 1 },
        hiddenRows: [
          { name: "gone", reason: { kind: "soldOrCancelled" } },
          { name: "bad", reason: { kind: "failedVerification", reason: "wrong lock" } },
        ],
      }),
    );
    renderPage();
    const hidden = await screen.findByTestId("hidden-summary");
    expect(hidden).toHaveTextContent("Hidden 2");
    fireEvent.click(hidden);
    const items = screen.getAllByTestId("hidden-row");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent(".gone");
    expect(items[0]).toHaveTextContent("Already sold or cancelled");
    expect(items[1]).toHaveTextContent("Failed verification: wrong lock");
    expect(screen.queryByTestId("market-buy")).not.toBeInTheDocument();
  });

  it("names a hidden listing called unknown, and marks one with no name", async () => {
    mockMarket(
      page({
        rows: [],
        hidden: { ...noHidden, failedVerification: 2 },
        hiddenRows: [
          { name: "unknown", reason: { kind: "failedVerification", reason: "wrong lock" } },
          { name: null, reason: { kind: "failedVerification", reason: "missing field" } },
        ],
      }),
    );
    renderPage();
    fireEvent.click(await screen.findByTestId("hidden-summary"));
    const items = screen.getAllByTestId("hidden-row");
    expect(items[0]).toHaveTextContent(".unknown");
    expect(items[1]).toHaveTextContent("Unnamed listing");
  });

  it("in SPV mode shows rows as not verified and without Buy", async () => {
    // The backend's SPV page: every row unverified, without a price
    // (`list_market_unverified_in_spv`).
    mockMarket(
      page({
        verified: false,
        rows: [
          row({
            verdict: { verdict: "hidden", kind: "unverified" },
            currentPrice: null,
          }),
        ],
      }),
    );
    renderPage();
    expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
    expect(
      screen.getAllByText("Not verified — needs a full or remote node").length,
    ).toBeGreaterThan(0);
    expect(screen.queryByTestId("market-buy")).not.toBeInTheDocument();
  });

  it.each(["ledger_hardware", "watch_only_xpub", "xpriv_hot"])(
    "a %s profile sees Buy disabled with the recovery-phrase reason",
    async (kind) => {
      mockMarket(page(), kind);
      renderPage();
      expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
      expect(
        await screen.findByText("Shakedex works with a recovery-phrase wallet for now"),
      ).toBeInTheDocument();
      const buy = screen.getByTestId("market-buy");
      expect(buy).toBeDisabled();
      // The disabled button says why itself, as Finalize does.
      fireEvent.mouseEnter(buy.parentElement!);
      expect(await screen.findByRole("tooltip")).toHaveTextContent(
        "Shakedex works with a recovery-phrase wallet for now",
      );
    },
  );

  it("on mainnet Buy stays disabled with the backend's reason until Shakedex is enabled", async () => {
    useSettingsStore.setState({
      settings: { ...DEFAULT_SETTINGS, shakedex_experimental: "false" },
      loaded: true,
    });
    mockMarket(page(), "mnemonic_hot", true, "mainnet");
    const { unmount } = renderPage();
    expect(
      await screen.findByText(
        "Shakedex purchases on mainnet are experimental: enable them in Settings",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("market-buy")).toBeDisabled();
    unmount();

    useSettingsStore.setState({
      settings: { ...DEFAULT_SETTINGS, shakedex_experimental: "true" },
      loaded: true,
    });
    renderPage();
    await waitFor(() => expect(screen.getByTestId("market-buy")).toBeEnabled());
    useSettingsStore.setState({ settings: null, loaded: false });
  });

  it("a node that cannot send leaves Buy disabled with the backend's reason", async () => {
    mockMarket(page(), "mnemonic_hot", false);
    renderPage();
    expect(
      await screen.findByText("Shakedex needs a local node, or a remote node with sending allowed"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("market-buy")).toBeDisabled();
  });

  it("a software profile sees no software-wallet notice", async () => {
    mockMarket(page());
    renderPage();
    await waitFor(() => expect(screen.getByTestId("market-buy")).toBeEnabled());
    expect(screen.queryByText(/recovery-phrase wallet for now/)).toBeNull();
  });

  it("shows the seller's listing expiry as information only", async () => {
    // 1815232480 = 2027-07-10 15:14:40 UTC, the live LearnHNS fixture's expiresAt.
    mockMarket(page({ rows: [row({ expiresAt: 1815232480 })] }));
    renderPage();
    expect(
      await screen.findByText("Listed until July 10, 2027 · stays buyable until sold or cancelled"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("market-buy")).toBeEnabled();
  });

  it("a listing whose expiresAt no date can hold still renders", async () => {
    mockMarket(page({ rows: [row({ expiresAt: 18446744073709552000 })] }));
    renderPage();
    expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
    expect(screen.queryByText(/Listed until/)).toBeNull();
  });

  it("a price at the money supply renders to the last dollarydoo", async () => {
    // The parser bounds a step's price plus fee to MAX_MONEY (R2), which is
    // below 2^53: every price the Market can show is exact as a JS number.
    const maxMoney = 2_040_000_000_000_000;
    mockMarket(
      page({
        rows: [
          row({
            kind: "reverseAuction",
            currentPrice: maxMoney - 1,
            nextPrice: null,
            floorPrice: maxMoney,
            steps: [{ price: maxMoney - 1, lockTime: 1_783_696_480, validInSecs: 0 }],
          }),
        ],
      }),
    );
    renderPage();
    expect(await screen.findAllByText(/2,039,999,999\.999999 HNS/)).not.toHaveLength(0);
    expect(screen.getByText(/2,040,000,000\.000000/)).toBeInTheDocument();
  });

  it("a listing priced at the u64 maximum still renders", async () => {
    // The parser refuses such a file (R2), so the backend never sends one;
    // should one arrive anyway, rendering must not throw and take the page
    // down.
    const max = 18446744073709552000;
    mockMarket(
      page({
        rows: [
          row({
            currentPrice: max,
            nextPrice: max,
            floorPrice: max,
            steps: [{ price: max, lockTime: max, validInSecs: max }],
          }),
        ],
      }),
    );
    renderPage();
    expect(await screen.findByText(".dexreviews")).toBeInTheDocument();
  });

  it("off mainnet explains there is no market and offers import", async () => {
    mockMarket(page({ rows: [], networkHasMarket: false }));
    renderPage();
    const notice = await screen.findByText(/no LearnHNS Market for this network/);
    expect(notice).not.toHaveTextContent(/link/);
    expect(screen.getByTestId("listing-paste")).toBeInTheDocument();
  });

  it("off mainnet disables link import with the backend's reason", async () => {
    mockMarket(page({ rows: [], networkHasMarket: false }), "mnemonic_hot", true, "regtest");
    renderPage();
    await screen.findByText(/no LearnHNS Market for this network/);
    fireEvent.change(screen.getByTestId("listing-paste"), {
      target: { value: "https://market.learnhns.com/listing/dexreviews" },
    });
    expect(screen.getByTestId("import-listing-link")).toBeDisabled();
    expect(screen.getByText("LearnHNS Market lists mainnet names only")).toBeInTheDocument();
  });

  it("on mainnet link import stays available", async () => {
    mockMarket(page(), "mnemonic_hot", true, "mainnet");
    renderPage();
    await screen.findByText(".dexreviews");
    fireEvent.change(screen.getByTestId("listing-paste"), {
      target: { value: "https://market.learnhns.com/listing/dexreviews" },
    });
    expect(screen.getByTestId("import-listing-link")).toBeEnabled();
    expect(screen.queryByText("LearnHNS Market lists mainnet names only")).not.toBeInTheDocument();
  });

  it("reverse auction shows now, next and floor", async () => {
    mockMarket(
      page({
        rows: [
          row({
            kind: "reverseAuction",
            currentPrice: 900e6,
            nextPrice: 800e6,
            nextValidInSecs: 21600,
            floorPrice: 600e6,
          }),
        ],
      }),
    );
    renderPage();
    expect(await screen.findByText(/900\.000000 HNS/)).toBeInTheDocument();
    expect(screen.getByText(/Next 800\.000000 HNS in ~6 h/)).toBeInTheDocument();
    expect(screen.getByText(/floor 600\.000000/)).toBeInTheDocument();
  });

  it("imports a pasted listing", async () => {
    mockMarket(page());
    renderPage();
    const json = '{"name":"imported"}';
    fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: json } });
    fireEvent.click(screen.getByTestId("import-listing-text"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("shakedex_import_listing", {
        source: { kind: "text", json },
      }),
    );
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(screen.getByText("From file")).toBeInTheDocument();
  });

  it("a market link pasted where a listing file goes is imported as a link", async () => {
    mockMarket(page({ rows: [] }), "mnemonic_hot", true, "mainnet");
    renderPage();
    const link = "https://market.learnhns.com/listing/enstransfer/proof.json";
    fireEvent.change(await screen.findByTestId("listing-paste"), {
      target: { value: `  ${link}\n` },
    });
    // The field sees a link: its button imports a link.
    fireEvent.click(screen.getByTestId("import-listing-link"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("shakedex_import_listing", {
        source: { kind: "link", url: link },
      }),
    );
  });

  it("gives an SPV import the backend's unverified verdict and no Buy", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "shakedex_list_market")
        return Promise.resolve(page({ verified: false, rows: [] }));
      if (cmd === "shakedex_import_listing")
        return Promise.resolve(
          row({
            name: "imported",
            verdict: { verdict: "hidden", kind: "unverified" },
          }),
        );
      return Promise.resolve(null);
    });
    renderPage();
    fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("import-listing-text"));
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(screen.getByText("Not verified — needs a full or remote node")).toBeInTheDocument();
    expect(screen.queryByTestId("market-buy")).not.toBeInTheDocument();
  });

  it.each([
    ["still loading", () => new Promise(() => {})],
    ["unavailable", () => Promise.reject("LearnHNS Market request failed: HTTP 502")],
  ])(
    "a buyable import can be bought while the market is %s",
    async (_label, market: () => Promise<unknown>) => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "list_wallet_profiles") return Promise.resolve([profile()]);
        if (cmd === "get_write_capability") return Promise.resolve(writeCapability());
        if (cmd === "shakedex_list_market") return market();
        if (cmd === "shakedex_import_listing") return Promise.resolve(row({ name: "imported" }));
        return Promise.resolve(null);
      });
      renderPage();
      fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: "{}" } });
      fireEvent.click(screen.getByTestId("import-listing-text"));
      expect(await screen.findByText(".imported")).toBeInTheDocument();
      expect(screen.getByTestId("market-buy")).toBeEnabled();
      expect(screen.queryByText("Not verified — needs a full or remote node")).toBeNull();
    },
  );

  it("badges a LearnHNS link import and buys it as from the market", async () => {
    mockMarket(page({ rows: [] }));
    renderPage();
    fireEvent.change(await screen.findByTestId("listing-paste"), {
      target: { value: "https://market.learnhns.com/listing/imported" },
    });
    fireEvent.click(screen.getByTestId("import-listing-link"));
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(screen.getByText("From LearnHNS link")).toBeInTheDocument();
    expect(screen.queryByText("From file")).toBeNull();
    fireEvent.click(screen.getByTestId("market-buy"));
    await waitFor(() =>
      expect(
        invokeMock.mock.calls.some(
          (c) =>
            c[0] === "shakedex_preview_purchase" &&
            (c[1] as { fromMarket: boolean }).fromMarket === true,
        ),
      ).toBe(true),
    );
  });

  it("buys a pasted import as not from the market", async () => {
    mockMarket(page({ rows: [] }));
    renderPage();
    fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("import-listing-text"));
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(screen.getByText("From file")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("market-buy"));
    await waitFor(() =>
      expect(
        invokeMock.mock.calls.some(
          (c) =>
            c[0] === "shakedex_preview_purchase" &&
            (c[1] as { fromMarket: boolean }).fromMarket === false,
        ),
      ).toBe(true),
    );
  });

  it("shows why a full node could not check an import, not that a node is needed", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "shakedex_list_market") return Promise.resolve(page({ rows: [] }));
      if (cmd === "shakedex_import_listing")
        return Promise.resolve(
          row({
            name: "imported",
            verdict: {
              verdict: "hidden",
              kind: "couldNotCheck",
              reason: "node did not report the name's height",
            },
          }),
        );
      return Promise.resolve(null);
    });
    renderPage();
    fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("import-listing-text"));
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(
      screen.getByText("Could not be checked: node did not report the name's height"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Not verified — needs a full or remote node")).toBeNull();
    expect(screen.queryByTestId("market-buy")).not.toBeInTheDocument();
  });

  it("shows why an import whose name expires before finalize cannot be bought, without Buy", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "shakedex_list_market") return Promise.resolve(page({ rows: [] }));
      if (cmd === "shakedex_import_listing")
        return Promise.resolve(
          row({ name: "imported", verdict: { verdict: "hidden", kind: "expiresBeforeFinalize" } }),
        );
      return Promise.resolve(null);
    });
    renderPage();
    fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("import-listing-text"));
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(
      screen.getByText("The name expires before a purchase could be finalized."),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("market-buy")).not.toBeInTheDocument();
  });

  it("shows an unverified import as not verified without Buy", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "shakedex_list_market") return Promise.resolve(page({ rows: [] }));
      if (cmd === "shakedex_import_listing")
        return Promise.resolve(
          row({
            name: "imported",
            verdict: { verdict: "hidden", kind: "unverified" },
          }),
        );
      return Promise.resolve(null);
    });
    renderPage();
    fireEvent.change(await screen.findByTestId("listing-paste"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("import-listing-text"));
    expect(await screen.findByText(".imported")).toBeInTheDocument();
    expect(screen.getByText("Not verified — needs a full or remote node")).toBeInTheDocument();
    expect(screen.queryByTestId("market-buy")).not.toBeInTheDocument();
  });

  it("lists every price step of a reverse auction", async () => {
    mockMarket(
      page({
        rows: [
          row({
            kind: "reverseAuction",
            verdict: { ...row().verdict, mtp: 1 },
            currentPrice: 900e6,
            nextPrice: 800e6,
            nextValidInSecs: 21600,
            floorPrice: 600e6,
            steps: [
              { price: 900e6, lockTime: 0, validInSecs: 0 },
              { price: 800e6, lockTime: 21600, validInSecs: 21504 },
              { price: 600e6, lockTime: 86400, validInSecs: 86016 },
            ],
          }),
        ],
      }),
    );
    renderPage();
    const toggle = await screen.findByTestId("price-steps-toggle");
    expect(toggle).toHaveTextContent("All 3 price steps");
    fireEvent.click(toggle);
    const items = screen.getByTestId("price-steps").querySelectorAll("li");
    expect(items).toHaveLength(3);
    expect(items[0]).toHaveTextContent("now");
    expect(items[1]).toHaveTextContent("800.000000 HNS — in ~6 h");
    expect(items[2]).toHaveTextContent("in ~24 h");
  });

  it("shows each step's wait as the backend gives it, a second short of valid included", async () => {
    // The 512-second rounding (R3) is the backend's: `market_row` gives each
    // step's wait, so the page never computes validity itself.
    for (const [validInSecs, label] of [
      [1, "in ~1 min"],
      [0, "now"],
    ] as const) {
      invokeMock.mockReset();
      mockMarket(
        page({
          rows: [
            row({
              kind: "reverseAuction",
              steps: [
                { price: 900e6, lockTime: 0, validInSecs: 0 },
                { price: 800e6, lockTime: 1535, validInSecs },
              ],
            }),
          ],
        }),
      );
      const { unmount } = renderPage();
      const toggle = await screen.findByTestId("price-steps-toggle");
      expect(toggle).toHaveTextContent("All 2 price steps");
      fireEvent.click(toggle);
      const items = screen.getByTestId("price-steps").querySelectorAll("li");
      expect(items[1]).toHaveTextContent(`800.000000 HNS — ${label}`);
      unmount();
    }
  });
});
