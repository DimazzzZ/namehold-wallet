import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
  readText: vi.fn().mockResolvedValue(""),
}));
const openExternalMock = vi.fn();
vi.mock("../../lib/openExternal", async (orig) => ({
  ...(await orig<typeof import("../../lib/openExternal")>()),
  openExternal: (url: string) => openExternalMock(url),
}));

import ExchangePage from "../exchange/ExchangePage";
import { useUiStore } from "../../stores/ui";
import {
  EXCHANGE_NOT_ENABLED,
  EXCHANGE_SUBTITLE,
  NO_RELAY,
  OWN_OFFER,
  NOSTR_HINT,
  SOURCES_HINT,
  TAKE_ABOVE_MAX,
  TAKE_BELOW_MIN,
} from "../../lib/exchangeText";
import { HNS_AMOUNT_DECIMALS } from "../../lib/hnsAmount";
import { PRICE_DECIMALS, SATS_NOT_POSITIVE } from "../../lib/exchangeAmount";
import { tradeStatusText } from "../../lib/exchangeStatus";
import {
  EVERY_STATUS,
  T0,
  makeBtcBalance,
  makeMarketOffers,
  makeOffer,
  makeOverview,
  makeTrade,
} from "../../test/fixtures/exchange";
import type {
  BtcBalance,
  ExchangeOverview,
  MarketOffers,
  OfferRow,
  TradeRow,
} from "../../types/exchange";

const REFUSAL =
  "The exchange needs a local or remote node: the Explorer chain source cannot check a lock.";
const U64_MAX = 18_446_744_073_709_551_615;

interface World {
  overview?: ExchangeOverview;
  offers?: MarketOffers;
  myOffers?: OfferRow[];
  trades?: TradeRow[];
  balance?: BtcBalance;
  /** Overrides for single commands. */
  on?: Record<string, (args?: Record<string, unknown>) => Promise<unknown>>;
}

function mock(w: World = {}) {
  invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    const own = w.on?.[cmd];
    if (own) return own(args);
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([
          {
            id: "p1",
            label: "Primary",
            network: "regtest",
            receiveAddress: "rs1q",
            watchOnly: false,
            hasPassphrase: false,
            active: true,
            kind: "mnemonic_hot",
          },
        ]);
      case "exchange_overview":
        return Promise.resolve(w.overview ?? makeOverview());
      case "exchange_list_offers":
        return Promise.resolve(w.offers ?? makeMarketOffers());
      case "exchange_my_offers":
        return Promise.resolve(w.myOffers ?? []);
      case "exchange_list_trades":
        return Promise.resolve(w.trades ?? []);
      case "exchange_btc_balance":
        return Promise.resolve(w.balance ?? makeBtcBalance());
      default:
        return Promise.resolve(null);
    }
  });
}

function renderPage(entry = "/exchange") {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[entry]}>
        <ExchangePage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const toasts = () => useUiStore.getState().toastQueue.map((t) => t.message);
const refusalOf = (testId: string) => screen.getByTestId(testId).getAttribute("data-refusal");

beforeEach(() => {
  invokeMock.mockReset();
  openExternalMock.mockReset();
  useUiStore.setState({ toastQueue: [] });
});

describe("Exchange page: beta notice", () => {
  // Shown whatever the gate says: before the overview answers, when the
  // exchange is off, and when it is refused here.
  for (const [what, overview] of [
    ["enabled", makeOverview()],
    ["not enabled", makeOverview({ enabled: false })],
    ["refused", makeOverview({ available: false, refusal: "No node.", enabled: true })],
  ] as const) {
    it(`asks for small trades and for every problem as a GitHub issue (${what})`, async () => {
      mock({ overview });
      renderPage();
      const banner = screen.getByTestId("exchange-beta-banner");
      expect(banner).toHaveTextContent("The exchange is in beta.");
      expect(banner).toHaveTextContent("Trade small amounts first");
      expect(banner).toHaveTextContent("report every problem, even a small one, as a GitHub issue");
      await screen.findAllByTestId("offer-row");
    });
  }

  it("opens a new GitHub issue from the notice", async () => {
    mock();
    renderPage();
    fireEvent.click(screen.getByTestId("exchange-report-problem"));
    expect(openExternalMock).toHaveBeenCalledWith(
      "https://github.com/DimazzzZ/namehold-wallet/issues/new",
    );
    await screen.findAllByTestId("offer-row");
  });
});

describe("Exchange page: header and gate (R17)", () => {
  it("shows the title and the subtitle", async () => {
    mock();
    renderPage();
    expect(screen.getByText("Exchange")).toBeInTheDocument();
    expect(screen.getByText(EXCHANGE_SUBTITLE)).toBeInTheDocument();
    await screen.findAllByTestId("offer-row");
  });

  it("shows the backend's refusal and disables every action with that sentence, hiding none", async () => {
    mock({
      overview: makeOverview({ available: false, refusal: REFUSAL, enabled: true }),
      myOffers: [makeOffer({ id: "mine-1", own: true })],
      trades: [makeTrade()],
    });
    renderPage();
    expect(await screen.findByTestId("exchange-gate-banner")).toHaveTextContent(REFUSAL);
    await screen.findAllByTestId("offer-row");
    for (const id of [
      "exchange-refresh",
      "exchange-create-offer",
      "exchange-settings",
      "btc-withdraw-open",
      "take-offer-sell-1",
      "take-offer-buy-1",
      "take-offer-sell-own",
    ]) {
      expect(screen.getByTestId(id), id).toBeDisabled();
      expect(refusalOf(id), id).toBe(REFUSAL);
    }

    fireEvent.click(screen.getByRole("tab", { name: /My offers/ }));
    expect(await screen.findByTestId("withdraw-offer-mine-1")).toBeDisabled();
    expect(refusalOf("withdraw-offer-mine-1")).toBe(REFUSAL);

    fireEvent.click(screen.getByRole("tab", { name: /My trades/ }));
    expect(await screen.findByTestId("trade-commit-trade-1")).toBeDisabled();
    expect(refusalOf("trade-commit-trade-1")).toBe(REFUSAL);
    expect(refusalOf("trade-cancel-trade-1")).toBe(REFUSAL);
  });

  it("disables Enable with the refusal too when the exchange is unavailable and off", async () => {
    mock({ overview: makeOverview({ available: false, refusal: REFUSAL, enabled: false }) });
    renderPage();
    expect(await screen.findByTestId("exchange-enable")).toBeDisabled();
    expect(refusalOf("exchange-enable")).toBe(REFUSAL);
  });
});

describe("Exchange page: enabling", () => {
  it("asks to enable, gates the actions meanwhile, and enables on click", async () => {
    let enabled = false;
    mock({
      on: {
        exchange_overview: () => Promise.resolve(makeOverview({ enabled })),
        exchange_enable: () => {
          enabled = true;
          return Promise.resolve(makeOverview({ enabled: true }));
        },
      },
    });
    renderPage();
    const button = await screen.findByTestId("exchange-enable");
    expect(button).toBeEnabled();
    expect(screen.queryByTestId("exchange-gate-banner")).toBeNull();
    expect(refusalOf("exchange-create-offer")).toBe(EXCHANGE_NOT_ENABLED);
    expect(refusalOf("btc-withdraw-open")).toBe(EXCHANGE_NOT_ENABLED);
    // The account's data is not read before it exists.
    expect(calls("exchange_btc_balance")).toHaveLength(0);
    expect(calls("exchange_list_trades")).toHaveLength(0);

    fireEvent.click(button);
    await waitFor(() => expect(screen.queryByTestId("exchange-enable-banner")).toBeNull());
    expect(calls("exchange_enable")).toHaveLength(1);
    expect(toasts()).toContain("Exchange enabled");
    await waitFor(() => expect(screen.getByTestId("exchange-create-offer")).toBeEnabled());
    expect(await screen.findByTestId("btc-confirmed")).toHaveTextContent("0.01250000 BTC");
  });
});

describe("Exchange page: order book", () => {
  it("splits offers by side and tags the user's own as not takeable", async () => {
    mock();
    renderPage();
    await screen.findAllByTestId("offer-row");
    const sell = screen.getByTestId("order-book-sell");
    const buy = screen.getByTestId("order-book-buy");
    const ids = (el: HTMLElement) =>
      within(el)
        .getAllByTestId("offer-row")
        .map((r) => r.getAttribute("data-offer-id"));
    expect(ids(sell)).toEqual(["sell-1", "sell-own"]);
    expect(ids(buy)).toEqual(["buy-1"]);
    expect(screen.getAllByTestId("offer-own")).toHaveLength(1);
    expect(screen.getByTestId("take-offer-sell-own")).toBeDisabled();
    expect(refusalOf("take-offer-sell-own")).toBe(OWN_OFFER);
    expect(screen.getByTestId("take-offer-sell-1")).toBeEnabled();
    const first = within(sell).getAllByTestId("offer-row")[0]!;
    expect(within(first).getByTestId("offer-price")).toHaveTextContent("250");
    expect(within(first).getByTestId("offer-range")).toHaveTextContent("100 – 2,500 HNS");
    expect(within(first).getByTestId("offer-btc")).toHaveTextContent(
      "≈ 0.00025000 – 0.00625000 BTC",
    );
  });

  it("shows the relay's error line", async () => {
    mock({ offers: makeMarketOffers({ offers: [], error: "The relay did not answer" }) });
    renderPage();
    expect(await screen.findByTestId("order-book-error")).toHaveTextContent(
      "The relay did not answer",
    );
  });

  it("renders u64-sized offer numbers without throwing", async () => {
    mock({
      offers: makeMarketOffers({
        offers: [
          makeOffer({
            id: "huge",
            price: "18446744073709551615.999",
            minHns: "1",
            maxHns: "18446744073709.551615",
            minSats: U64_MAX,
            maxSats: U64_MAX,
            expiresAt: U64_MAX,
          }),
        ],
      }),
    });
    renderPage();
    const row = await screen.findByTestId("offer-row");
    expect(within(row).getByTestId("offer-btc")).toHaveTextContent("—");
    expect(within(row).getByTestId("offer-price")).toHaveTextContent(
      "18,446,744,073,709,551,615.999",
    );
  });

  it("says when no relay is set and disables every offer action with that sentence", async () => {
    mock({
      overview: makeOverview({ relayUrl: "", meetingPoint: "none" }),
      myOffers: [makeOffer({ id: "mine-1" })],
    });
    renderPage();
    expect(await screen.findByTestId("order-book-no-relay")).toHaveTextContent(NO_RELAY);
    expect(calls("exchange_list_offers")).toHaveLength(0);
    expect(screen.getByTestId("exchange-create-offer")).toBeDisabled();
    expect(refusalOf("exchange-create-offer")).toBe(NO_RELAY);
    // Settings stays usable: it is how a relay gets set.
    expect(screen.getByTestId("exchange-settings")).toBeEnabled();
    fireEvent.click(screen.getByRole("tab", { name: /My offers/ }));
    expect(await screen.findByTestId("withdraw-offer-mine-1")).toBeDisabled();
    expect(refusalOf("withdraw-offer-mine-1")).toBe(NO_RELAY);
  });
});

describe("Exchange page: Take dialog", () => {
  async function openTake(id = "sell-1") {
    fireEvent.click(await screen.findByTestId(`take-offer-${id}`));
    return screen.getByTestId("take-amount");
  }
  const type = (el: HTMLElement, value: string) => fireEvent.change(el, { target: { value } });

  it("refuses an amount outside the offer's limits or past 6 decimals", async () => {
    mock();
    renderPage();
    const field = await openTake();
    type(field, "99.999999");
    expect(screen.getByTestId("take-amount-error")).toHaveTextContent(TAKE_BELOW_MIN("100"));
    expect(screen.getByTestId("take-submit")).toBeDisabled();
    type(field, "2500.000001");
    expect(screen.getByTestId("take-amount-error")).toHaveTextContent(TAKE_ABOVE_MAX("2500"));
    type(field, "1.0000001");
    expect(screen.getByTestId("take-amount-error")).toHaveTextContent(HNS_AMOUNT_DECIMALS);
    type(field, "");
    expect(screen.queryByTestId("take-amount-error")).toBeNull();
    expect(screen.getByTestId("take-submit")).toBeDisabled();
  });

  it("shows the BTC exactly as floor(doos × msat / 1e9) and takes the offer", async () => {
    mock({
      offers: makeMarketOffers({
        offers: [makeOffer({ id: "odd", price: "333.333", minHns: "0.000001", maxHns: "5000" })],
      }),
      on: { exchange_take_offer: () => Promise.resolve(makeTrade()) },
    });
    renderPage();
    const field = await openTake("odd");
    // 1234.567891 HNS × 333.333 sats/HNS = 411,522.140 sats → 411,522.
    type(field, "1234.567891");
    expect(screen.getByTestId("take-btc")).toHaveTextContent(
      "You pay 0.00411522 BTC (411,522 sats).",
    );
    fireEvent.click(screen.getByTestId("take-submit"));
    await waitFor(() => expect(calls("exchange_take_offer")).toHaveLength(1));
    expect(calls("exchange_take_offer")[0]![1]).toEqual({
      offerId: "odd",
      hnsAmount: "1234.567891",
    });
    await waitFor(() => expect(screen.queryByTestId("take-amount")).toBeNull());
    expect(toasts()).toContain("Trade started · it is under My trades");
  });

  it("says You receive on a Buy HNS offer", async () => {
    mock();
    renderPage();
    const field = await openTake("buy-1");
    type(field, "100");
    expect(screen.getByTestId("take-btc")).toHaveTextContent(
      "You receive 0.00024050 BTC (24,050 sats).",
    );
  });

  it("computes at a u64-max-ish price and amount without throwing", async () => {
    mock({
      offers: makeMarketOffers({
        offers: [
          makeOffer({
            id: "huge",
            price: "18446744073709551615.999",
            minHns: "0.000001",
            maxHns: "18446744073709.551615",
            minSats: U64_MAX,
            maxSats: U64_MAX,
          }),
        ],
      }),
    });
    renderPage();
    const field = await openTake("huge");
    // Every HNS there will ever be, at the u64-max price.
    type(field, "2040000000");
    const doos = 2_040_000_000_000_000n;
    const msat = 18_446_744_073_709_551_615_999n;
    const sats = (doos * msat) / 1_000_000_000n;
    const grouped = sats.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    expect(screen.getByTestId("take-btc")).toHaveTextContent(`(${grouped} sats)`);
    // Past the money supply: the field's own sentence, no throw.
    type(field, "18446744073709.551615");
    expect(screen.getByTestId("take-amount-error")).toHaveTextContent(
      "That is more than the 2.04 billion HNS that will ever exist",
    );
  });

  it("shows the backend's refusal under the form", async () => {
    mock({
      on: { exchange_take_offer: () => Promise.reject("The offer is no longer on the relay") },
    });
    renderPage();
    type(await openTake(), "100");
    fireEvent.click(screen.getByTestId("take-submit"));
    expect(await screen.findByTestId("take-error")).toHaveTextContent(
      "The offer is no longer on the relay",
    );
  });
});

describe("Exchange page: Create offer dialog", () => {
  const type = (id: string, value: string) =>
    fireEvent.change(screen.getByTestId(id), { target: { value } });

  it("validates price and limits with a sentence under each field", async () => {
    mock();
    renderPage();
    await screen.findAllByTestId("offer-row");
    fireEvent.click(screen.getByTestId("exchange-create-offer"));
    type("offer-price-input", "250.1234");
    expect(screen.getByTestId("offer-price-error")).toHaveTextContent(PRICE_DECIMALS);
    type("offer-price-input", "0");
    expect(screen.getByTestId("offer-price-error")).toHaveTextContent(
      "Enter a price greater than 0",
    );
    type("offer-price-input", "250");
    expect(screen.queryByTestId("offer-price-error")).toBeNull();
    type("offer-min-input", "500");
    type("offer-max-input", "100");
    expect(screen.getByTestId("offer-min-error")).toHaveTextContent(
      "The smallest trade cannot be larger than the largest",
    );
    expect(screen.getByTestId("offer-submit")).toBeDisabled();
    type("offer-max-input", "0");
    expect(screen.getByTestId("offer-max-error")).toHaveTextContent(
      "Enter an amount greater than 0",
    );
  });

  it("publishes the offer with normalized digits and awaits the secure window", async () => {
    let resolve: (v: unknown) => void = () => {};
    mock({
      on: {
        exchange_create_offer: () => new Promise((r) => (resolve = r)),
      },
    });
    renderPage();
    await screen.findAllByTestId("offer-row");
    fireEvent.click(screen.getByTestId("exchange-create-offer"));
    fireEvent.click(screen.getByTestId("offer-side-buy"));
    type("offer-price-input", "250.500");
    type("offer-min-input", "100.0");
    type("offer-max-input", "2500");
    fireEvent.click(screen.getByTestId("offer-submit"));
    await waitFor(() => expect(calls("exchange_create_offer")).toHaveLength(1));
    expect(calls("exchange_create_offer")[0]![1]).toEqual({
      side: "buy",
      price: "250.5",
      minHns: "100",
      maxHns: "2500",
    });
    expect(screen.getByTestId("offer-submit")).toHaveTextContent("Confirm in the secure window…");
    resolve(makeOffer({ id: "mine-1", own: true, side: "buy" }));
    await waitFor(() => expect(screen.queryByTestId("offer-submit")).toBeNull());
    expect(toasts()).toContain("Offer published");
  });
});

describe("Exchange page: My offers", () => {
  it("shows live and paused offers and withdraws one", async () => {
    mock({
      myOffers: [
        makeOffer({ id: "mine-1", own: true, live: true }),
        makeOffer({ id: "mine-2", own: true, live: false, side: "buy" }),
      ],
    });
    renderPage("/exchange?tab=offers");
    expect(await screen.findByTestId("my-offer-live")).toHaveTextContent("Live");
    expect(screen.getByTestId("my-offer-paused")).toHaveTextContent("Paused");
    fireEvent.click(screen.getByTestId("withdraw-offer-mine-2"));
    await waitFor(() => expect(calls("exchange_withdraw_offer")).toHaveLength(1));
    expect(calls("exchange_withdraw_offer")[0]![1]).toEqual({ offerId: "mine-2" });
  });
});

describe("Exchange page: My trades", () => {
  const statusRows = EVERY_STATUS.flatMap((status) =>
    (["buyer", "seller"] as const).map((role) => [status, role] as const),
  );

  it.each(statusRows)("renders the headline of %j as %s", async (status, role) => {
    const trade = makeTrade({ role, status });
    mock({ trades: [trade] });
    renderPage("/exchange?tab=trades");
    const card = await screen.findByTestId("trade-card");
    const expected = tradeStatusText(trade);
    expect(within(card).getByTestId("trade-headline")).toHaveTextContent(expected.headline);
    expect(within(card).getByTestId("trade-sentence")).toHaveTextContent(expected.sentence);
  });

  it("highlights a trade awaiting the user's commit, and Confirm and sign calls exchange_commit", async () => {
    mock({
      trades: [
        makeTrade({ tradeId: "other", role: "seller", updatedAt: T0 + 50 }),
        makeTrade({ tradeId: "mine", role: "buyer" }),
      ],
      on: { exchange_commit: () => Promise.resolve(makeTrade({ tradeId: "mine" })) },
    });
    renderPage("/exchange?tab=trades");
    const cards = await screen.findAllByTestId("trade-card");
    // The one waiting for the user comes first.
    expect(cards.map((c) => c.getAttribute("data-trade-id"))).toEqual(["mine", "other"]);
    expect(cards[0]).toHaveAttribute("data-needs-commit", "true");
    expect(within(cards[0]!).getByTestId("trade-commit-deadline")).toHaveTextContent(
      "Needs your confirmation before Nov 7, 2026, 13:00 UTC",
    );
    expect(within(cards[1]!).queryByTestId("trade-commit-other")).toBeNull();
    expect(screen.getByRole("tab", { name: /My trades/ })).toHaveTextContent("1");

    fireEvent.click(screen.getByTestId("trade-commit-mine"));
    await waitFor(() => expect(calls("exchange_commit")).toHaveLength(1));
    expect(calls("exchange_commit")[0]![1]).toEqual({ tradeId: "mine" });
    await waitFor(() => expect(toasts()).toContain("Confirmed and signed"));
  });

  it("disables the Commit while the terms are not agreed", async () => {
    mock({ trades: [makeTrade({ negotiating: true, termsId: null, timeline: null })] });
    renderPage("/exchange?tab=trades");
    expect(await screen.findByTestId("trade-commit-trade-1")).toBeDisabled();
    expect(refusalOf("trade-commit-trade-1")).toBe("The terms are not agreed yet.");
    expect(screen.getByTestId("trade-negotiating")).toBeInTheDocument();
  });

  it("offers Cancel only while negotiating or before the user's commit", async () => {
    mock({
      trades: [
        makeTrade({ tradeId: "b-before", role: "buyer" }),
        makeTrade({
          tradeId: "b-after",
          role: "buyer",
          status: { kind: "awaitingBtcLock", confirmations: 0, needed: 2 },
        }),
        makeTrade({
          tradeId: "s-before",
          role: "seller",
          status: { kind: "awaitingBtcLock", confirmations: 1, needed: 2 },
        }),
        makeTrade({
          tradeId: "s-after",
          role: "seller",
          status: { kind: "awaitingHnsLock", confirmations: 1, needed: 6 },
        }),
        makeTrade({
          tradeId: "nego",
          role: "seller",
          negotiating: true,
          status: { kind: "awaitingHnsLock", confirmations: 0, needed: 6 },
        }),
      ],
      on: { exchange_cancel_trade: () => Promise.resolve(makeTrade({ tradeId: "s-before" })) },
    });
    renderPage("/exchange?tab=trades");
    await screen.findAllByTestId("trade-card");
    expect(screen.getByTestId("trade-cancel-b-before")).toBeEnabled();
    expect(screen.queryByTestId("trade-cancel-b-after")).toBeNull();
    expect(screen.getByTestId("trade-cancel-s-before")).toBeEnabled();
    expect(screen.queryByTestId("trade-cancel-s-after")).toBeNull();
    expect(screen.getByTestId("trade-cancel-nego")).toBeEnabled();
    fireEvent.click(screen.getByTestId("trade-cancel-s-before"));
    await waitFor(() => expect(calls("exchange_cancel_trade")).toHaveLength(1));
    expect(calls("exchange_cancel_trade")[0]![1]).toEqual({ tradeId: "s-before" });
  });

  it("shows the timeline, the locks, the watchtower, a decline and the last error", async () => {
    mock({
      trades: [
        makeTrade({
          status: { kind: "awaitingHnsLock", confirmations: 2, needed: 6 },
          btcLock: { txid: "11".repeat(32), vout: 1 },
          watchtower: "registered",
          declined: "the price moved",
          lastError: "The relay did not answer",
          timeline: {
            buyerFundDeadline: T0 + 3600,
            sellerFundDeadline: T0 + 4 * 3600,
            revealDeadline: T0 + 9 * 3600,
            hnsRefundTime: U64_MAX,
            btcRefundTime: T0 + 24 * 3600,
          },
        }),
      ],
    });
    renderPage("/exchange?tab=trades");
    await screen.findByTestId("trade-card");
    expect(screen.getByTestId("trade-buyerFundDeadline")).toHaveTextContent(
      "Nov 7, 2026, 13:00 UTC",
    );
    expect(screen.getByTestId("trade-btcRefundTime")).toHaveTextContent("Nov 8, 2026, 12:00 UTC");
    // A time no Date can hold shows a dash.
    expect(screen.getByTestId("trade-hnsRefundTime")).toHaveTextContent("—");
    expect(screen.getByTestId("trade-btc-lock")).toHaveTextContent(":1");
    expect(screen.getByTestId("trade-hns-lock")).toHaveTextContent("Not yet");
    expect(screen.getByTestId("trade-watchtower")).toHaveTextContent(
      "Registered with the watchtower",
    );
    expect(screen.getByTestId("trade-declined")).toHaveTextContent(
      "The other side declined: the price moved",
    );
    expect(screen.getByTestId("trade-last-error")).toHaveTextContent("The relay did not answer");
  });

  it("Refresh runs exchange_refresh and shows its answer", async () => {
    mock({
      trades: [],
      on: {
        exchange_refresh: () =>
          Promise.resolve([makeTrade({ tradeId: "fresh", status: { kind: "completed" } })]),
      },
    });
    renderPage("/exchange?tab=trades");
    expect(await screen.findByTestId("my-trades-empty")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("exchange-refresh")).toBeEnabled());
    fireEvent.click(screen.getByTestId("exchange-refresh"));
    expect(await screen.findByText("Trade complete")).toBeInTheDocument();
    expect(calls("exchange_refresh")).toHaveLength(1);
  });
});

describe("Exchange page: BTC account", () => {
  it("shows the balances and the receive address", async () => {
    mock({ balance: makeBtcBalance() });
    renderPage();
    expect(await screen.findByTestId("btc-confirmed")).toHaveTextContent(
      "0.01250000 BTC1,250,000 sats",
    );
    expect(screen.getByTestId("btc-unconfirmed")).toHaveTextContent("20,000 sats");
    expect(screen.getByTestId("btc-locked")).toHaveTextContent("300,000 sats");
    expect(screen.getByTestId("btc-receive-address")).toHaveTextContent(
      "bcrt1qexchangereceive0000000000000000000000",
    );
    expect(screen.getByTestId("btc-receive-copy")).toBeEnabled();
    expect(screen.queryByTestId("btc-source-error")).toBeNull();
    expect(screen.getByText(/^Checked /)).toBeInTheDocument();
  });

  // The command answers a failed source with 0, 0 and the error (QA Q2): the
  // card must not show an empty account.
  it("shows amounts the source did not give as not read, never as zero", async () => {
    mock({
      balance: makeBtcBalance({
        confirmedSats: 0,
        unconfirmedSats: 0,
        error: "Bitcoin source http://127.0.0.1:18443/: error sending request",
      }),
    });
    renderPage();
    expect(await screen.findByTestId("btc-confirmed")).toHaveTextContent("Not read");
    expect(screen.getByTestId("btc-confirmed")).not.toHaveTextContent("0 sats");
    expect(screen.getByTestId("btc-unconfirmed")).toHaveTextContent("Not read");
    // Locked in trades comes from the stored trades, not from the source.
    expect(screen.getByTestId("btc-locked")).toHaveTextContent("300,000 sats");
    expect(screen.getByTestId("btc-source-error")).toHaveTextContent("error sending request");
    expect(screen.getByText(/^Last check failed /)).toBeInTheDocument();
    expect(screen.queryByText(/^Checked /)).toBeNull();
    expect(screen.getByTestId("btc-receive-address")).toHaveTextContent("bcrt1q");
  });

  it("shows a dash for a balance past 2^53 instead of a wrong number", async () => {
    mock({ balance: makeBtcBalance({ confirmedSats: U64_MAX }) });
    renderPage();
    expect(await screen.findByTestId("btc-confirmed")).toHaveTextContent("—");
  });

  it("withdraws an amount or everything", async () => {
    mock({ on: { exchange_btc_withdraw: () => Promise.resolve({ txid: "ab".repeat(32) }) } });
    renderPage();
    await screen.findByTestId("btc-confirmed");
    fireEvent.click(screen.getByTestId("btc-withdraw-open"));
    // No address yet: the click says so.
    fireEvent.click(screen.getByTestId("btc-withdraw-submit"));
    expect(screen.getByTestId("btc-address-error")).toHaveTextContent("Enter a Bitcoin address");
    fireEvent.change(screen.getByTestId("btc-withdraw-address"), {
      target: { value: " bcrt1qdest " },
    });
    fireEvent.change(screen.getByTestId("btc-withdraw-amount"), { target: { value: "1.5" } });
    expect(screen.getByTestId("btc-amount-error")).toHaveTextContent(SATS_NOT_POSITIVE);
    fireEvent.change(screen.getByTestId("btc-withdraw-amount"), { target: { value: "2000000" } });
    expect(screen.getByTestId("btc-amount-error")).toHaveTextContent(
      "That is more than your confirmed balance of 1,250,000 sats",
    );
    expect(screen.getByTestId("btc-withdraw-submit")).toBeDisabled();
    fireEvent.change(screen.getByTestId("btc-withdraw-amount"), { target: { value: "50000" } });
    fireEvent.click(screen.getByTestId("btc-withdraw-submit"));
    await waitFor(() => expect(calls("exchange_btc_withdraw")).toHaveLength(1));
    expect(calls("exchange_btc_withdraw")[0]![1]).toEqual({
      toAddress: "bcrt1qdest",
      amountSats: 50_000,
    });
    await waitFor(() => expect(screen.queryByTestId("btc-withdraw-address")).toBeNull());

    fireEvent.click(screen.getByTestId("btc-withdraw-open"));
    fireEvent.change(screen.getByTestId("btc-withdraw-address"), {
      target: { value: "bcrt1qdest" },
    });
    fireEvent.click(screen.getByTestId("btc-withdraw-everything"));
    expect(screen.getByTestId("btc-withdraw-amount")).toBeDisabled();
    fireEvent.click(screen.getByTestId("btc-withdraw-submit"));
    await waitFor(() => expect(calls("exchange_btc_withdraw")).toHaveLength(2));
    expect(calls("exchange_btc_withdraw")[1]![1]).toEqual({
      toAddress: "bcrt1qdest",
      amountSats: null,
    });
  });
});

describe("Exchange page: settings dialog", () => {
  const type = (id: string, value: string) =>
    fireEvent.change(screen.getByTestId(id), { target: { value } });

  async function openSettings(overview: ExchangeOverview) {
    mock({
      overview,
      on: { exchange_update_settings: () => Promise.resolve(overview) },
    });
    renderPage();
    await waitFor(() => expect(screen.getByTestId("exchange-settings")).toBeEnabled());
    fireEvent.click(screen.getByTestId("exchange-settings"));
  }

  it("opens with the stored values and the sources hint", async () => {
    await openSettings(makeOverview({ watchtowerUrl: "https://tower.example" }));
    expect(screen.getByTestId("settings-relay")).toHaveValue("http://127.0.0.1:8787");
    expect(screen.getByTestId("settings-watchtower")).toHaveValue("https://tower.example");
    expect(screen.getByTestId("settings-sources")).toHaveValue(
      "https://mempool.example/api\nhttps://blockstream.example/api",
    );
    expect(screen.getByTestId("settings-sources-hint")).toHaveTextContent(SOURCES_HINT);
    expect(screen.getByTestId("settings-auth")).toHaveAttribute("type", "password");
    expect(screen.queryByTestId("settings-auth-clear")).toBeNull();
  });

  it("validates the URLs, the sources and the login", async () => {
    await openSettings(makeOverview());
    type("settings-relay", "relay.example");
    expect(screen.getByTestId("settings-relay-error")).toHaveTextContent(
      "Enter a URL that starts with http:// or https://",
    );
    type("settings-relay", "https://relay.example");
    type("settings-watchtower", "ftp://tower");
    expect(screen.getByTestId("settings-watchtower-error")).toBeInTheDocument();
    type("settings-watchtower", "");
    type("settings-sources", "https://mempool.space/api\nmempool.space");
    expect(screen.getByTestId("settings-sources-error")).toHaveTextContent('"mempool.space"');
    type("settings-sources", "  \n");
    expect(screen.getByTestId("settings-sources-error")).toHaveTextContent(
      "Enter at least one Bitcoin source",
    );
    type("settings-sources", "bitcoind+http://127.0.0.1:18443");
    type("settings-auth", "nocolon");
    expect(screen.getByTestId("settings-auth-error")).toHaveTextContent(
      "Enter the RPC login as user:password",
    );
    expect(screen.getByTestId("settings-save")).toBeDisabled();
    type("settings-auth", "user:secret");
    expect(screen.getByTestId("settings-save")).toBeEnabled();
  });

  it("saves the trimmed values, an empty watchtower and a new login", async () => {
    await openSettings(makeOverview());
    type("settings-relay", " https://relay.example ");
    type("settings-sources", "https://mempool.space/api\n\n bitcoind+http://127.0.0.1:18443 \n");
    type("settings-auth", "user:secret");
    fireEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => expect(calls("exchange_update_settings")).toHaveLength(1));
    expect(calls("exchange_update_settings")[0]![1]).toEqual({
      relayUrl: "https://relay.example",
      watchtowerUrl: "",
      btcSources: ["https://mempool.space/api", "bitcoind+http://127.0.0.1:18443"],
      bitcoindAuth: "user:secret",
      nostrRelays: [],
    });
    await waitFor(() => expect(screen.queryByTestId("settings-save")).toBeNull());
    expect(toasts()).toContain("Exchange settings saved");
  });

  it("shows the default Nostr relays, checks each line and saves the list", async () => {
    await openSettings(
      makeOverview({
        meetingPoint: "nostr",
        relayUrl: "",
        defaultNostrRelays: ["wss://relay.one", "wss://relay.two"],
      }),
    );
    expect(screen.getByTestId("settings-nostr")).toHaveValue("");
    expect(screen.getByTestId("settings-nostr")).toHaveAttribute(
      "placeholder",
      "wss://relay.one\nwss://relay.two",
    );
    expect(screen.getByTestId("settings-nostr-hint")).toHaveTextContent(NOSTR_HINT);
    type("settings-nostr", "wss://mine.example\nhttps://not-a-relay");
    expect(screen.getByTestId("settings-nostr-error")).toHaveTextContent('"https://not-a-relay"');
    expect(screen.getByTestId("settings-save")).toBeDisabled();
    type("settings-nostr", " wss://mine.example \n\nwss://other.example");
    fireEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => expect(calls("exchange_update_settings")).toHaveLength(1));
    expect(calls("exchange_update_settings")[0]![1]).toMatchObject({
      nostrRelays: ["wss://mine.example", "wss://other.example"],
      relayUrl: "",
    });
  });

  it("keeps a stored login when the field is left empty, and Clear sends an empty one", async () => {
    await openSettings(makeOverview({ bitcoindAuthSet: true }));
    expect(screen.getByTestId("settings-auth")).toHaveAttribute("placeholder", "set");
    expect(screen.getByTestId("settings-auth-state")).toHaveTextContent(
      "A user:password is set. Leave the field empty to keep it.",
    );
    fireEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => expect(calls("exchange_update_settings")).toHaveLength(1));
    expect(calls("exchange_update_settings")[0]![1]).toMatchObject({ bitcoindAuth: null });

    await waitFor(() => expect(screen.queryByTestId("settings-save")).toBeNull());
    fireEvent.click(screen.getByTestId("exchange-settings"));
    fireEvent.click(screen.getByTestId("settings-auth-clear"));
    expect(screen.getByTestId("settings-auth")).toBeDisabled();
    expect(screen.getByTestId("settings-auth-state")).toHaveTextContent(
      "The stored user:password is removed when you save.",
    );
    fireEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() => expect(calls("exchange_update_settings")).toHaveLength(2));
    expect(calls("exchange_update_settings")[1]![1]).toMatchObject({ bitcoindAuth: "" });
  });

  it("is disabled with the refusal when the exchange is unavailable", async () => {
    mock({ overview: makeOverview({ available: false, refusal: REFUSAL }) });
    renderPage();
    await screen.findByTestId("exchange-gate-banner");
    expect(screen.getByTestId("exchange-settings")).toBeDisabled();
    expect(refusalOf("exchange-settings")).toBe(REFUSAL);
  });
});
