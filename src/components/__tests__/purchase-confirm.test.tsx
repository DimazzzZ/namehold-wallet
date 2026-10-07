import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { PurchaseConfirm } from "../market/PurchaseConfirm";
import { useUiStore } from "../../stores/ui";
import { useSettingsStore, DEFAULT_SETTINGS } from "../../stores/settings";

const row = {
  listingJson: '{"name":"dexreviews"}',
  name: "dexreviews",
  verdict: { verdict: "buyable" },
  kind: "buyNow",
  currentPrice: 250_000_000,
  nextPrice: null,
  nextValidInSecs: null,
  floorPrice: 250_000_000,
  steps: [],
  expiresAt: null,
} as never;

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

function preview(over: Record<string, unknown> = {}) {
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
    warnExpiry: false,
    ...over,
  };
}

interface Opts {
  kind?: string;
  network?: string;
  canSend?: boolean;
  preview?: Record<string, unknown>;
  build?: () => Promise<unknown>;
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
      case "get_write_capability": {
        const canSend = o.canSend ?? true;
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: canSend,
          canWrite: canSend,
          reason: null,
        });
      }
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
        return Promise.resolve({ id: "draft-1" });
      case "broadcast_tx_draft":
        return o.broadcast ? o.broadcast() : Promise.resolve({ txid: "abc" });
      default:
        return Promise.resolve(null);
    }
  });
}

function renderIt(onClose = vi.fn()) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  render(<PurchaseConfirm open row={row} fromMarket onClose={onClose} />, { wrapper });
  return onClose;
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

beforeEach(() => {
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("PurchaseConfirm", () => {
  it("shows price, market fee with percent, network fee and total", async () => {
    setup();
    renderIt();
    expect(await screen.findByText("Price (to seller)")).toBeInTheDocument();
    expect(screen.getByTestId("purchase-market-fee")).toHaveTextContent("2.500000 HNS (1.00%)");
    expect(screen.getByText("Network fee")).toBeInTheDocument();
    expect(screen.getByText("Total")).toBeInTheDocument();
    expect(
      screen.getByText(
        "The name becomes yours after a finalize, 288 blocks (about 2 days) after the purchase is mined.",
      ),
    ).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("purchase-market-fee-pay")).toBeChecked());
  });

  it("an unpublished market fee is unchecked with a warning", async () => {
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
  });

  it("toggling the fee re-fetches the preview", async () => {
    setup();
    renderIt();
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    fireEvent.click(box);
    await waitFor(() =>
      expect(
        calls("shakedex_preview_purchase").some(
          (c) => (c[1] as { payMarketFee: boolean }).payMarketFee === false,
        ),
      ).toBe(true),
    );
  });

  it("confirm builds a fresh draft and runs it", async () => {
    setup();
    const onClose = renderIt();
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    const order = invokeMock.mock.calls.map((c) => c[0]);
    expect(order).toContain("shakedex_build_purchase_draft");
    expect(order.indexOf("sign_tx_draft")).toBeGreaterThan(
      order.indexOf("shakedex_build_purchase_draft"),
    );
    expect(order).toContain("broadcast_tx_draft");
    expect(useUiStore.getState().toastMessage).toContain("Purchase sent");
  });

  it("states the backend's wait for the profile network, as the secure window does", async () => {
    setup({
      preview: { finalizeWait: "after a finalize, 10 blocks after the purchase is mined" },
    });
    renderIt();
    expect(
      await screen.findByText(
        "The name becomes yours after a finalize, 10 blocks after the purchase is mined.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/days/)).toBeNull();
  });

  it("an expiry warning is shown", async () => {
    setup({ preview: { warnExpiry: true } });
    renderIt();
    const warning = await screen.findByText(/finalize it in time/);
    // The name expires, not the listing (whose own expiry is "Listed until").
    expect(warning).toHaveTextContent(/^This name expires soon/);
    expect(warning).toHaveTextContent(/the name is lost/);
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
    expect(fee).toHaveTextContent("4.350000 HNS");
    expect(fee).not.toHaveTextContent("%");
  });

  it("ledger profile cannot buy", async () => {
    setup({ kind: "ledger_hardware" });
    renderIt();
    expect(await screen.findByText(/recovery-phrase wallet/)).toBeInTheDocument();
    expect(screen.getByTestId("purchase-buy")).toBeDisabled();
  });

  it("on mainnet Buy stays disabled with the backend's reason until Shakedex is enabled", async () => {
    useSettingsStore.setState({
      settings: { ...DEFAULT_SETTINGS, shakedex_experimental: "false" },
      loaded: true,
    });
    setup({ network: "mainnet" });
    renderIt();
    expect(
      await screen.findByText(
        "Shakedex purchases on mainnet are experimental: enable them in Settings",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("purchase-buy")).toBeDisabled();
    useSettingsStore.setState({ settings: null, loaded: false });
  });

  it("a node that cannot send leaves Buy disabled with the backend's reason", async () => {
    setup({ canSend: false });
    renderIt();
    expect(
      await screen.findByText("Shakedex needs a local node, or a remote node with sending allowed"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("purchase-buy")).toBeDisabled();
  });

  it("an unpayable fee has a disabled, unchecked checkbox", async () => {
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
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
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
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    const before = calls("shakedex_preview_purchase").length;
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
    expect(await screen.findByText(/market fee changed/)).toBeInTheDocument();
    await waitFor(() => expect(calls("shakedex_preview_purchase").length).toBeGreaterThan(before));
    expect(calls("sign_tx_draft")).toHaveLength(0);
  });

  it("build gets the previewed fee when ticked and null when not", async () => {
    setup();
    renderIt();
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
    await waitFor(() => expect(calls("shakedex_build_purchase_draft")).toHaveLength(1));
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({
      acceptedMarketFeeDoos: 2_500_000,
    });
  });

  it("build gets null when the fee box is unticked", async () => {
    setup();
    renderIt();
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    fireEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
    await waitFor(() => expect(calls("shakedex_build_purchase_draft")).toHaveLength(1));
    expect(calls("shakedex_build_purchase_draft")[0]![1]).toMatchObject({
      acceptedMarketFeeDoos: null,
    });
  });

  it("an xpriv profile cannot buy", async () => {
    setup({ kind: "xpriv_hot" });
    renderIt();
    expect(await screen.findByText(/recovery-phrase wallet/)).toBeInTheDocument();
    expect(screen.getByTestId("purchase-buy")).toBeDisabled();
  });

  it("Buy stays disabled when the re-fetch after a refused build fails", async () => {
    setup({
      failPreviewAfterBuild: true,
      build: () =>
        Promise.reject("the market fee changed since you reviewed it — review the purchase again"),
    });
    renderIt();
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
    expect(await screen.findByText(/market fee changed/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText(/Could not price/)).toBeInTheDocument());
    expect(screen.getByTestId("purchase-buy")).toBeDisabled();
  });

  // The backend discards a purchase it refuses before sending; the dialog
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
      const onClose = renderIt();
      const box = await screen.findByTestId("purchase-market-fee-pay");
      await waitFor(() => expect(box).toBeChecked());
      const before = calls("shakedex_preview_purchase").length;
      await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
      fireEvent.click(screen.getByTestId("purchase-buy"));
      expect(await screen.findByText(shown)).toBeInTheDocument();
      await waitFor(() =>
        expect(calls("shakedex_preview_purchase").length).toBeGreaterThan(before),
      );
      expect(calls("delete_tx_draft")).toHaveLength(0);
      expect(onClose).not.toHaveBeenCalled();
    },
  );

  it("an ordinary broadcast failure keeps the draft", async () => {
    setup({ broadcast: () => Promise.reject("TX rejected: bad-txns-inputs-missingorspent") });
    renderIt();
    const box = await screen.findByTestId("purchase-market-fee-pay");
    await waitFor(() => expect(box).toBeChecked());
    await waitFor(() => expect(screen.getByTestId("purchase-buy")).toBeEnabled());
    fireEvent.click(screen.getByTestId("purchase-buy"));
    expect(await screen.findByText(/bad-txns/)).toBeInTheDocument();
    expect(calls("delete_tx_draft")).toHaveLength(0);
  });
});
