import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { ListingDetail } from "../market/ListingDetail";
import { buildList, type Entry, type Origin } from "../market/marketModel";
import { useUiStore } from "../../stores/ui";
import { makeAuctionRow, makeMarketPage, makeMarketRow } from "../../test/fixtures/listings";
import type { MarketRow, PurchasePreview } from "../../types";

const UNPUBLISHED =
  "not signed by the seller and not the market's published fee — anyone could have added it";

function profile(kind = "mnemonic_hot", network = "regtest") {
  return {
    id: "p1",
    label: "Primary",
    network,
    receiveAddress: "rs1q",
    watchOnly: false,
    hasPassphrase: false,
    active: true,
    kind,
  };
}

function preview(over: Partial<PurchasePreview> = {}): PurchasePreview {
  return {
    name: "dexreviews",
    priceDoos: 250_000_000,
    marketFee: {
      valueDoos: 2_500_000,
      percentText: "1.00%",
      published: true,
      payable: true,
      warning: null,
    },
    networkFeeDoos: 10_000,
    totalDoos: 252_510_000,
    finalizeWait: "after a finalize, 288 blocks (about 2 days) after the purchase is mined",
    lockupBlocks: 288,
    wallClockTiming: true,
    warnExpiry: false,
    ...over,
  };
}

interface Opts {
  kind?: string;
  network?: string;
  preview?: Partial<PurchasePreview>;
  build?: () => Promise<unknown>;
  sign?: () => Promise<unknown>;
  broadcast?: () => Promise<unknown>;
  failPreviewAfterBuild?: boolean;
}

function setup(o: Opts = {}) {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile(o.kind, o.network)]);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: 0 });
      case "shakedex_preview_purchase":
        if (
          o.failPreviewAfterBuild &&
          invokeMock.mock.calls.some((c) => c[0] === "shakedex_build_purchase_draft")
        )
          return Promise.reject("node unreachable");
        return Promise.resolve(preview(o.preview));
      case "shakedex_build_purchase_draft":
        return o.build ? o.build() : Promise.resolve({ id: "draft-1" });
      case "sign_tx_draft":
        return o.sign ? o.sign() : Promise.resolve({ id: "draft-1" });
      case "broadcast_tx_draft":
        return o.broadcast ? o.broadcast() : Promise.resolve({ txid: "abc" });
      default:
        return Promise.resolve(null);
    }
  });
}

function entryOf(row: MarketRow, origin: Origin = "market"): Entry {
  const { entries } = buildList({
    imported: origin === "market" ? [] : [{ row, origin }],
    pages: origin === "market" ? [makeMarketPage({ rows: [row] })] : [],
    rechecked: new Map(),
  });
  return entries[0]!;
}

interface RenderOpts {
  row?: MarketRow;
  origin?: Origin;
  entry?: Entry;
  refusal?: string | null;
  gated?: boolean;
  availableDoos?: number | null;
  qc?: QueryClient;
  /** Render inside a scrolling page area this tall (clientHeight, padding 24). */
  scrollport?: number;
}

function renderIt(o: RenderOpts = {}) {
  const onBought = vi.fn();
  const onTurnOn = vi.fn();
  const onRetry = vi.fn();
  const entry = o.entry ?? entryOf(o.row ?? makeMarketRow(), o.origin);
  function Harness() {
    const [pay, setPay] = useState<boolean | null>(null);
    const [sent, setSent] = useState(false);
    return (
      <ListingDetail
        entry={entry}
        sent={sent}
        refusal={o.refusal ?? null}
        gated={o.gated ?? false}
        onTurnOn={onTurnOn}
        availableDoos={o.availableDoos === undefined ? 10_000_000_000 : o.availableDoos}
        pay={pay}
        onPay={setPay}
        checking={false}
        onRetry={onRetry}
        onBought={(e) => {
          setSent(true);
          onBought(e);
        }}
        primaryRef={{ current: null }}
      />
    );
  }
  const qc = o.qc ?? new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const port = o.scrollport;
  render(
    <QueryClientProvider client={qc}>
      {port === undefined ? (
        <Harness />
      ) : (
        <div
          style={{ overflowY: "auto", padding: 24 }}
          // jsdom lays nothing out: give the page area its height.
          ref={(el) => {
            if (el) Object.defineProperty(el, "clientHeight", { value: port, configurable: true });
          }}
        >
          <Harness />
        </div>
      )}
    </QueryClientProvider>,
  );
  return { onBought, onTurnOn, onRetry };
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const buy = () => screen.getByTestId("purchase-buy");

async function ready() {
  const box = await screen.findByTestId("purchase-market-fee-pay");
  await waitFor(() => expect(box).toBeChecked());
  await waitFor(() => expect(buy()).toBeEnabled());
  return box;
}

beforeEach(() => {
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("ListingDetail: the breakdown", () => {
  it("shows the price, the LearnHNS fee with its share, the network fee and the total, to 6 decimals", async () => {
    setup();
    renderIt();
    await ready();
    const rows = screen.getByTestId("purchase-rows");
    expect(rows).toHaveTextContent("Price to seller250.000000 HNS");
    expect(rows).toHaveTextContent("LearnHNS fee (1.00%)");
    expect(screen.getByTestId("purchase-market-fee")).toHaveTextContent("2.500000 HNS");
    expect(rows).toHaveTextContent("Network fee0.010000 HNS");
    expect(screen.getByTestId("purchase-total")).toHaveTextContent("252.510000 HNS");
    // The commit button names the name and the short total.
    expect(buy()).toHaveTextContent("Buy .dexreviews for 252.51 HNS");
  });

  it("names the wait in How it works by the network's lockup, as the design does", async () => {
    setup();
    renderIt();
    const steps = await screen.findByTestId("detail-steps");
    await within(steps).findByText("Wait ~2 days");
    expect(steps).toHaveTextContent(
      "288 blocks while the swap settles. Track it in Your purchases.",
    );
    expect(screen.queryByText("How it works")).toBeNull();
  });

  it("counts the wait in blocks where blocks are mined on demand (regtest)", async () => {
    setup({ preview: { lockupBlocks: 10, wallClockTiming: false } });
    renderIt();
    const steps = await screen.findByTestId("detail-steps");
    await within(steps).findByText("Wait 10 blocks");
    expect(steps).toHaveTextContent(
      "The swap settles once 10 more blocks are mined. Track it in Your purchases.",
    );
    expect(steps).not.toHaveTextContent("~");
  });

  it("an unpublished market fee is unticked with a warning", async () => {
    setup({
      preview: {
        marketFee: {
          valueDoos: 2_500_000,
          percentText: "1.00%",
          published: false,
          payable: true,
          warning: UNPUBLISHED,
        },
      },
    });
    renderIt();
    expect(await screen.findByText(/not signed by the seller/)).toBeInTheDocument();
    expect(screen.getByTestId("purchase-market-fee-pay")).not.toBeChecked();
    expect(screen.getByTestId("purchase-market-fee-pay")).toBeEnabled();
    expect(screen.getByText(/Optional. Without it LearnHNS isn't paid/)).toBeInTheDocument();
  });

  it("locks the fee for a listing from a file, and prices it as not from the market", async () => {
    setup({ preview: { marketFee: null } });
    renderIt({ origin: "file" });
    expect(
      await screen.findByText(
        "This listing didn't come through LearnHNS, so there's no market fee.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("purchase-market-fee-pay")).toBeDisabled();
    expect(screen.getByTestId("purchase-market-fee")).toHaveTextContent("—");
    await waitFor(() => expect(calls("shakedex_preview_purchase").length).toBeGreaterThan(0));
    expect(calls("shakedex_preview_purchase")[0]![1]).toMatchObject({ fromMarket: false });
  });

  it("never pays a fee a file's listing carries, whatever the preview says", async () => {
    setup({
      preview: {
        marketFee: {
          valueDoos: 2_500_000,
          percentText: "1.00%",
          published: false,
          payable: true,
          warning: UNPUBLISHED,
        },
      },
    });
    renderIt({ origin: "file" });
    await waitFor(() => expect(buy()).toBeEnabled());
    expect(screen.getByTestId("purchase-market-fee-pay")).toBeDisabled();
    expect(screen.getByTestId("purchase-market-fee-pay")).not.toBeChecked();
    expect(screen.getByTestId("purchase-market-fee")).toHaveTextContent("—");
    fireEvent.click(buy());
    await waitFor(() => expect(calls("shakedex_build_purchase_draft")).toHaveLength(1));
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({
      acceptedMarketFeeDoos: null,
      fromMarket: false,
    });
  });

  it("prices a LearnHNS link as from the market", async () => {
    setup();
    renderIt({ origin: "link" });
    await ready();
    expect(calls("shakedex_preview_purchase")[0]![1]).toMatchObject({ fromMarket: true });
  });

  it("toggling the fee re-fetches the preview", async () => {
    setup();
    renderIt();
    const box = await ready();
    fireEvent.click(box);
    await waitFor(() =>
      expect(
        calls("shakedex_preview_purchase").some(
          (c) => (c[1] as { payMarketFee: boolean }).payMarketFee === false,
        ),
      ).toBe(true),
    );
  });

  it("a market fee with no meaningful percentage is shown without one", async () => {
    setup({
      preview: {
        marketFee: {
          valueDoos: 4_350_000,
          percentText: null,
          published: true,
          payable: true,
          warning: null,
        },
      },
    });
    renderIt();
    const fee = await screen.findByTestId("purchase-market-fee");
    await waitFor(() => expect(fee).toHaveTextContent("4.350000 HNS"));
    expect(screen.getByText("LearnHNS fee")).toBeInTheDocument();
  });

  it("an unpayable fee has a disabled, unticked box and is not paid", async () => {
    setup({
      preview: {
        marketFee: {
          valueDoos: 100,
          percentText: "0.01%",
          published: false,
          payable: false,
          warning: "the market fee is below the dust limit, so it is not paid",
        },
        totalDoos: 250_010_000,
      },
    });
    renderIt();
    expect(await screen.findByText(/below the dust limit/)).toBeInTheDocument();
    expect(screen.getByTestId("purchase-market-fee-pay")).toBeDisabled();
    expect(screen.getByTestId("purchase-market-fee-pay")).not.toBeChecked();
    await waitFor(() => expect(buy()).toBeEnabled());
    fireEvent.click(buy());
    await waitFor(() => expect(calls("shakedex_build_purchase_draft")).toHaveLength(1));
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({
      acceptedMarketFeeDoos: null,
    });
  });

  it("warns when the name expires soon after it can be finalized", async () => {
    setup({ preview: { warnExpiry: true } });
    renderIt();
    const checks = await screen.findByTestId("detail-checks");
    await waitFor(() => expect(checks).toHaveTextContent(/soon after you can finalize/));
    expect(checks).toHaveTextContent(/Finalize and renew in time or the name is lost/);
  });

  it("shows a reverse auction's steps from now on and how it drops", async () => {
    setup();
    renderIt({ row: makeAuctionRow() });
    expect(
      await screen.findByText("Drops to 1,100 HNS in ~6 h, down to a floor of 400 HNS in ~3 d."),
    ).toBeInTheDocument();
    const steps = screen.getByTestId("price-steps");
    expect(steps).toHaveTextContent("Now");
    expect(steps).toHaveTextContent("~3 d");
    expect(steps.querySelectorAll("li")).toHaveLength(4);
  });

  it("lists a file's steps by when they lower the price, not by where the file puts them", async () => {
    setup();
    const step = (price: number, validInSecs: number) => ({
      price,
      lockTime: 1_700_000_000 + validInSecs,
      validInSecs,
    });
    // The current step (cheapest valid) is second in the file; an expensive
    // step written last never comes into play, nor does another valid one.
    renderIt({
      row: makeAuctionRow({
        currentPrice: 1_000_000_000,
        steps: [
          step(900_000_000, 21_600),
          step(1_000_000_000, 0),
          step(1_500_000_000, 0),
          step(2_000_000_000, 86_400),
        ],
        verdict: { ...makeAuctionRow().verdict, currentStep: 1 } as MarketRow["verdict"],
      }),
    });
    const items = [...(await screen.findByTestId("price-steps")).querySelectorAll("li")];
    expect(items.map((li) => li.textContent)).toEqual(["Now1,000 HNS", "~6 h900 HNS"]);
  });

  it("refuses a fee rate that is not a whole number, and sends a valid one in sats/byte", async () => {
    setup();
    renderIt();
    await ready();
    fireEvent.click(screen.getByTestId("fee-rate-toggle"));
    fireEvent.change(screen.getByTestId("fee-rate-input"), { target: { value: "1.5" } });
    expect(screen.getByTestId("fee-rate-error")).toHaveTextContent("Whole number only");
    expect(buy()).toBeDisabled();
    expect(screen.getByTestId("detail-action-reason")).toHaveTextContent("Whole number only");
    fireEvent.change(screen.getByTestId("fee-rate-input"), { target: { value: "5000" } });
    await waitFor(() => expect(buy()).toBeEnabled());
    fireEvent.click(buy());
    await waitFor(() => expect(calls("shakedex_build_purchase_draft")).toHaveLength(1));
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({ feeRate: 5 });
  });
});

describe("ListingDetail: the button (§3.5)", () => {
  it("keeps Buy in sight: the panel fits the page area, scrolls inside, its action pinned", async () => {
    setup();
    renderIt({ scrollport: 700 });
    await ready();
    const panel = screen.getByTestId("listing-detail");
    // The page area's height less its padding, 24 above and below.
    await waitFor(() => expect(panel.style.maxHeight).toBe("652px"));
    expect(panel).toHaveClass("overflow-y-auto");
    expect(screen.getByTestId("detail-action")).toHaveClass("sticky", "bottom-0", "bg-white");

    // Lower on the page, under the banner and Your purchases, before it
    // sticks: it ends at the page area's foot all the same.
    panel.getBoundingClientRect = () => ({ top: 300 }) as DOMRect;
    fireEvent.scroll(panel.parentElement!);
    await waitFor(() => expect(panel.style.maxHeight).toBe("376px"));
  });

  it("turns buying on first when mainnet buying is off", async () => {
    setup({ network: "mainnet" });
    const { onTurnOn } = renderIt({ gated: true });
    const button = await screen.findByRole("button", { name: "Turn on buying" });
    expect(screen.getByTestId("detail-action-reason")).toHaveTextContent(
      "Buying on mainnet is off, so this button turns it on first. Nothing is bought until you press Buy.",
    );
    expect(button).toHaveAttribute("aria-describedby", "detail-action-reason");
    fireEvent.click(button);
    expect(onTurnOn).toHaveBeenCalled();
    expect(calls("shakedex_build_purchase_draft")).toHaveLength(0);
  });

  it("never shows a second filled button beside the gate banner's", async () => {
    setup({ network: "mainnet", kind: "watch_only_xpub" });
    renderIt({ gated: true, refusal: "Shakedex works with a recovery-phrase wallet for now" });
    expect(await screen.findByTestId("detail-action-reason")).toHaveTextContent(
      "Shakedex works with a recovery-phrase wallet for now.",
    );
    expect(buy()).toBeDisabled();
    expect(buy().className).not.toContain("bg-blue-600");
  });

  it("says how much is missing when the total is more than the balance", async () => {
    setup();
    renderIt({ availableDoos: 100_000_000 });
    await waitFor(() =>
      expect(screen.getByTestId("detail-action-reason")).toHaveTextContent(
        "You have 100 HNS available, 152.51 HNS short of the total.",
      ),
    );
    expect(buy()).toBeDisabled();
    expect(buy()).toHaveTextContent("Buy for 252.51 HNS");
  });

  it.each([
    ["a Ledger profile", "Shakedex works with a recovery-phrase wallet for now"],
    [
      "a node that cannot send",
      "Shakedex needs a local node, or a remote node with sending allowed",
    ],
  ])("refuses %s with the backend's sentence", async (_label, refusal) => {
    setup();
    renderIt({ refusal, gated: true });
    expect(await screen.findByTestId("detail-action-reason")).toHaveTextContent(`${refusal}.`);
    expect(buy()).toBeDisabled();
    expect(buy()).toHaveTextContent("Buy .dexreviews");
  });

  it("refuses a listing of this device's own, without pricing it (R33)", async () => {
    setup();
    renderIt({ row: makeMarketRow({ own: true }) });
    expect(await screen.findByTestId("detail-action-reason")).toHaveTextContent(
      /^This is your own listing: cancel it instead \(My listings\)/,
    );
    expect(buy()).toBeDisabled();
    expect(calls("shakedex_preview_purchase")).toHaveLength(0);
  });

  it("keeps Buy disabled with the reason when the preview fails", async () => {
    setup({
      failPreviewAfterBuild: true,
      build: () =>
        Promise.reject("the market fee changed since you reviewed it — review the purchase again"),
    });
    renderIt();
    await ready();
    fireEvent.click(buy());
    expect(await screen.findByText(/market fee changed/)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("detail-action-reason")).toHaveTextContent(/node unreachable/),
    );
    expect(buy()).toBeDisabled();
  });

  it("Buy waits for the preview the fee box asked for", async () => {
    setup();
    const base = invokeMock.getMockImplementation()!;
    // The preview without the market fee never answers: the rows on screen
    // are the old ones, kept while the new figures load.
    invokeMock.mockImplementation((cmd: string, args?: { payMarketFee?: boolean }) =>
      cmd === "shakedex_preview_purchase" && args?.payMarketFee === false
        ? new Promise(() => {})
        : base(cmd, args),
    );
    renderIt();
    const box = await ready();
    fireEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    expect(screen.getByTestId("purchase-market-fee")).toHaveTextContent("2.500000 HNS");
    await waitFor(() => expect(buy()).toBeDisabled());
    expect(screen.getByTestId("detail-action-reason")).toHaveTextContent("Working out the total…");
  });
});

describe("ListingDetail: buying", () => {
  it("builds a fresh draft, runs it, and shows the purchase as sent", async () => {
    setup();
    const { onBought } = renderIt();
    await ready();
    fireEvent.click(buy());
    expect(await screen.findByTestId("detail-sent")).toHaveTextContent(
      ".dexreviews is locked to you.",
    );
    expect(onBought).toHaveBeenCalledTimes(1);
    const order = invokeMock.mock.calls.map((c) => c[0]);
    expect(order.indexOf("sign_tx_draft")).toBeGreaterThan(
      order.indexOf("shakedex_build_purchase_draft"),
    );
    expect(order).toContain("broadcast_tx_draft");
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({
      acceptedMarketFeeDoos: 2_500_000,
      fromMarket: true,
    });
    // The checks stay; the breakdown and the button go.
    expect(screen.getByTestId("detail-checks")).toBeInTheDocument();
    expect(screen.queryByTestId("purchase-buy")).toBeNull();
  });

  it("build gets null when the fee box is unticked", async () => {
    setup();
    renderIt();
    const box = await ready();
    fireEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    await waitFor(() => expect(buy()).toBeEnabled());
    fireEvent.click(buy());
    await waitFor(() => expect(calls("shakedex_build_purchase_draft")).toHaveLength(1));
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({
      acceptedMarketFeeDoos: null,
    });
  });

  it("a refused build shows the message and re-fetches the preview", async () => {
    setup({
      build: () =>
        Promise.reject("the market fee changed since you reviewed it — review the purchase again"),
    });
    renderIt();
    await ready();
    const before = calls("shakedex_preview_purchase").length;
    fireEvent.click(buy());
    expect(await screen.findByTestId("purchase-error")).toHaveTextContent(/market fee changed/);
    await waitFor(() => expect(calls("shakedex_preview_purchase").length).toBeGreaterThan(before));
    expect(calls("sign_tx_draft")).toHaveLength(0);
  });

  // The backend discards a purchase it refuses before sending; the panel
  // never tells refusals apart by their text.
  it.each([
    ["the price changed — review the purchase again", /price changed/],
    [
      "node did not report median time, so the price could not be re-checked; the purchase was not sent",
      /median time/,
    ],
  ])(
    "a broadcast refusal (%s) is shown, leaves the draft to the backend and re-fetches the preview",
    async (message, shown) => {
      setup({ broadcast: () => Promise.reject(message) });
      const { onBought } = renderIt();
      await ready();
      const before = calls("shakedex_preview_purchase").length;
      fireEvent.click(buy());
      expect(await screen.findByText(shown)).toBeInTheDocument();
      await waitFor(() =>
        expect(calls("shakedex_preview_purchase").length).toBeGreaterThan(before),
      );
      expect(calls("delete_tx_draft")).toHaveLength(0);
      expect(onBought).not.toHaveBeenCalled();
    },
  );

  it("an ordinary broadcast failure keeps the draft", async () => {
    setup({ broadcast: () => Promise.reject("TX rejected: bad-txns-inputs-missingorspent") });
    renderIt();
    await ready();
    fireEvent.click(buy());
    expect(await screen.findByText(/bad-txns/)).toBeInTheDocument();
    expect(calls("delete_tx_draft")).toHaveLength(0);
  });

  // A failed send may have deleted the purchase draft and its record (a
  // refusal before sending, a cancelled window) or kept it broadcast_pending:
  // Owned Names and the market page read both, so they are refreshed now.
  it.each([
    [
      "a broadcast failure",
      { broadcast: () => Promise.reject("TX rejected: bad-txns") },
      /bad-txns/,
    ],
    [
      "a refusal before broadcast",
      { sign: () => Promise.reject("cancelled by user") },
      /cancelled/,
    ],
  ])("%s refreshes Owned Names and the market", async (_label, opts, shown) => {
    setup(opts);
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const spy = vi.spyOn(qc, "invalidateQueries");
    renderIt({ qc });
    await ready();
    fireEvent.click(buy());
    expect(await screen.findByText(shown)).toBeInTheDocument();
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ queryKey: ["read"] }));
    expect(spy).toHaveBeenCalledWith({ queryKey: ["shakedex"] });
  });

  it("cancelling the secure window discards the prepared purchase", async () => {
    setup({ sign: () => Promise.reject("cancelled by user") });
    const { onBought } = renderIt();
    await ready();
    fireEvent.click(buy());
    expect(await screen.findByText(/cancelled by user/)).toBeInTheDocument();
    await waitFor(() => expect(calls("delete_tx_draft")).toHaveLength(1));
    expect(calls("delete_tx_draft")[0]![1]).toEqual({ draftId: "draft-1" });
    expect(calls("broadcast_tx_draft")).toHaveLength(0);
    expect(onBought).not.toHaveBeenCalled();
  });

  it("while a purchase runs, it says so and a second click builds nothing", async () => {
    let finish: (v: unknown) => void = () => {};
    setup({ build: () => new Promise((resolve) => (finish = resolve)) });
    const { onBought } = renderIt();
    await ready();
    fireEvent.click(buy());
    fireEvent.click(buy());
    await waitFor(() => expect(buy()).toHaveTextContent("Sending…"));
    expect(buy()).toBeDisabled();
    expect(calls("shakedex_build_purchase_draft")).toHaveLength(1);
    finish({ id: "draft-1" });
    await waitFor(() => expect(onBought).toHaveBeenCalledTimes(1));
  });
});

describe("ListingDetail: a listing the node could not check", () => {
  it("shows no price, the node's reason and Check again", async () => {
    setup();
    const { entries } = buildList({
      imported: [],
      pages: [
        makeMarketPage({
          rows: [],
          hiddenRows: [
            {
              name: "cooljobs",
              reason: { kind: "couldNotCheck", reason: "Node RPC error: operation timed out" },
              listingJson: "{}",
            },
          ],
        }),
      ],
      rechecked: new Map(),
    });
    const { onRetry } = renderIt({ entry: entries[0]! });
    expect(screen.getByText("Couldn't check this listing")).toBeInTheDocument();
    expect(screen.getByTestId("detail-unchecked")).toHaveTextContent(
      /Your node took too long to answer\..*won't show a price or let you buy until it's verified/,
    );
    expect(screen.queryByTestId("detail-price")).toBeNull();
    expect(screen.queryByTestId("purchase-buy")).toBeNull();
    fireEvent.click(screen.getByTestId("detail-check-again"));
    expect(onRetry).toHaveBeenCalled();
    expect(calls("shakedex_preview_purchase")).toHaveLength(0);
  });
});
