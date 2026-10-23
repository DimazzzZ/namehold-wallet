/**
 * Speed up (spec 2026-10-21): where the button is (R16) and what the form
 * says (R11, R16). Tauri rejections are `AppError` Display strings, as the
 * backend's `InvalidInput` sends them.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn(),
}));

import { ActivityView } from "../ActivityView";
import { PendingPurchases } from "../market/PendingPurchases";
import { deadlineSentence, deadlineText, rateText } from "../SpeedUp";
import { useUiStore } from "../../stores/ui";

const PARENT_TXID = "a".repeat(64);
const CHILD_TXID = "c".repeat(64);
const EXPIRES = 1_761_255_600;

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "rs1qexamplereceiveaddr",
  lastSyncedHeight: 10,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

/** A sent plain send, as `list_tx_drafts` returns it. */
function sendDraft(status: string, overrides: Record<string, unknown> = {}) {
  return {
    id: "parent1",
    walletProfileId: "p1",
    action: "send_hns",
    status,
    summary: {
      action: "send_hns",
      sendTotalDoos: 100_000,
      feeDoos: 140,
      changeDoos: 1_000_000,
      inputTotalDoos: 1_100_140,
      numInputs: 1,
      recipientAddress: "rs1qrecipient",
      txid: PARENT_TXID,
      warnings: [],
    },
    errorMessage: null,
    txid: status === "draft" ? null : PARENT_TXID,
    confirmationHeight: status === "confirmed" ? 121 : null,
    purchaseLostReason: null,
    createdAt: "2026-10-21 12:00:00",
    ...overrides,
  };
}

/** `speed_up.rs::SpeedUpSummary` as `preview_speed_up` returns it. */
function preview(overrides: Record<string, unknown> = {}) {
  return {
    action: "speed_up",
    parentTxid: PARENT_TXID,
    parentDraftId: "parent1",
    parentIsPurchase: false,
    txid: CHILD_TXID,
    sendTotalDoos: 0,
    feeDoos: 2_340,
    changeDoos: 997_660,
    inputTotalDoos: 1_000_000,
    numInputs: 1,
    recipientAddress: null,
    targetRatePerVbyte: 10,
    targetSource: "estimate",
    parentSize: 140,
    parentFeeDoos: 140,
    packageSize: 140,
    packageFeesDoos: 140,
    childSize: 108,
    packageRateBefore: 1000,
    packageRateAfter: 10_000,
    expiresAt: EXPIRES,
    warnings: [],
    ...overrides,
  };
}

function childDraft(status: string) {
  return {
    id: "child1",
    walletProfileId: "p1",
    action: "speed_up",
    status,
    summary: preview(),
    errorMessage: null,
    txid: CHILD_TXID,
    confirmationHeight: null,
    purchaseLostReason: null,
    createdAt: "2026-10-21 12:05:00",
  };
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const toasts = () => useUiStore.getState().toastQueue.map((t) => t.message);

interface World {
  drafts?: unknown[];
  check?: unknown;
  preview?: (args: Record<string, unknown>) => Promise<unknown>;
}

function routes(w: World) {
  return (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "list_tx_drafts":
        return Promise.resolve(w.drafts ?? []);
      case "read_action_history":
        return Promise.resolve([]);
      case "get_signer_session":
        return Promise.resolve({ unlocked: true, walletProfileId: "p1", expiresAtMs: null });
      case "speed_up_check":
        return Promise.resolve(w.check ?? { available: true, reason: null, expiresAt: EXPIRES });
      case "preview_speed_up":
        return w.preview ? w.preview(args ?? {}) : Promise.resolve(preview());
      case "build_speed_up_draft":
        return Promise.resolve(childDraft("draft"));
      case "sign_tx_draft":
        return Promise.resolve(childDraft("signed"));
      case "broadcast_tx_draft":
        return Promise.resolve({ draftId: "child1", txid: CHILD_TXID, status: "broadcasted" });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/activity"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  useUiStore.setState({ toastQueue: [] });
});

describe("Speed up in Activity (R16)", () => {
  it("a sent, unconfirmed row offers Speed up, checked against its own draft", async () => {
    invokeMock.mockImplementation(routes({ drafts: [sendDraft("broadcasted")] }));
    render(<ActivityView />, { wrapper: wrapper() });
    const button = await screen.findByTestId("speed-up-button");
    await waitFor(() => expect(button).toBeEnabled());
    expect(calls("speed_up_check")[0]![1]).toEqual({ draftId: "parent1" });
  });

  it("a refused row shows Speed up disabled with the backend's reason, never hidden", async () => {
    const reason = "This transaction has no change to spend, so it cannot be sped up.";
    invokeMock.mockImplementation(
      routes({
        drafts: [sendDraft("broadcasted")],
        check: { available: false, reason, expiresAt: null },
      }),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const button = await screen.findByTestId("speed-up-button");
    await waitFor(() => expect(button).toHaveAccessibleDescription(reason));
    expect(button).toBeDisabled();
  });

  it.each(["draft", "signed", "broadcast_pending", "confirmed", "dropped", "failed"])(
    "a %s row has no Speed up",
    async (status) => {
      invokeMock.mockImplementation(routes({ drafts: [sendDraft(status)] }));
      render(<ActivityView />, { wrapper: wrapper() });
      // The row is drawn (header + one row) before its absence is read.
      await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(2));
      expect(screen.queryByTestId("speed-up-button")).not.toBeInTheDocument();
    },
  );

  it("a child's row names what it speeds up and offers no Speed up; its parent names the child", async () => {
    invokeMock.mockImplementation(
      routes({
        drafts: [sendDraft("broadcasted"), childDraft("broadcasted")],
        check: {
          available: false,
          reason: "This transaction is already being sped up.",
          expiresAt: null,
        },
      }),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    expect(await screen.findByTestId("activity-speeds-up")).toHaveTextContent(
      `Speeds up ${PARENT_TXID.slice(0, 10)}…`,
    );
    expect(screen.getByTestId("activity-sped-up-by")).toHaveTextContent(
      `Sped up by ${CHILD_TXID.slice(0, 10)}…`,
    );
    // Only the parent's row has the button (disabled: already sped up).
    const buttons = await screen.findAllByTestId("speed-up-button");
    expect(buttons).toHaveLength(1);
    expect(calls("speed_up_check").every((c) => c[1].draftId === "parent1")).toBe(true);
  });
});

describe("the Speed up form (R11, R16)", () => {
  async function openForm(w: World = {}) {
    invokeMock.mockImplementation(routes({ drafts: [sendDraft("broadcasted")], ...w }));
    render(<ActivityView />, { wrapper: wrapper() });
    const button = await screen.findByTestId("speed-up-button");
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    return screen.findByTestId("speed-up-dialog");
  }

  it("shows the parent, the rates, the child's fee, the change left and the deadline", async () => {
    const dialog = await openForm();
    const numbers = await within(dialog).findByTestId("speed-up-numbers");
    expect(numbers).toHaveTextContent("140 vbytes, fee 0.000140 HNS");
    expect(numbers).toHaveTextContent("Fee rate now1.00 doos/vB");
    expect(numbers).toHaveTextContent("Target10 doos/vB (your node's estimate)");
    expect(numbers).toHaveTextContent("Speed-up fee0.002340 HNS");
    expect(numbers).toHaveTextContent("Fee rate with it10.00 doos/vB");
    expect(numbers).toHaveTextContent("Change left0.997660 HNS");
    expect(numbers).toHaveTextContent(`Mempool deadline${deadlineText(EXPIRES)}`);
    expect(within(dialog).getByTestId("speed-up-deadline")).toHaveTextContent(
      deadlineSentence(EXPIRES),
    );
    expect(dialog).toHaveTextContent("The fee goes to the network only; nobody else is paid.");
    expect(dialog).toHaveTextContent("The original keeps its transaction id.");
    expect(calls("preview_speed_up")[0]![1]).toEqual({ draftId: "parent1", feeRate: null });
  });

  it("typing a rate asks the node once, for the rate typed, not per keystroke", async () => {
    const dialog = await openForm();
    await waitFor(() => expect(calls("preview_speed_up")).toHaveLength(1));
    const input = within(dialog).getByTestId("speed-up-rate");
    fireEvent.change(input, { target: { value: "2" } });
    fireEvent.change(input, { target: { value: "25" } });
    fireEvent.change(input, { target: { value: "250" } });
    await waitFor(() =>
      expect(calls("preview_speed_up").some((c) => c[1].feeRate === 250)).toBe(true),
    );
    const rates = calls("preview_speed_up").map((c) => c[1].feeRate);
    expect(rates).toEqual([null, 250]);
  });

  it("a typed rate is previewed and built at that rate", async () => {
    const dialog = await openForm();
    fireEvent.change(within(dialog).getByTestId("speed-up-rate"), { target: { value: "25" } });
    await waitFor(() =>
      expect(calls("preview_speed_up").some((c) => c[1].feeRate === 25)).toBe(true),
    );
    const submit = within(dialog).getByTestId("speed-up-submit");
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(1));
    expect(calls("build_speed_up_draft")[0]![1]).toEqual({ draftId: "parent1", feeRate: 25 });
    expect(calls("sign_tx_draft")[0]![1]).toEqual({ draftId: "child1" });
    expect(calls("broadcast_tx_draft")[0]![1]).toEqual({ draftId: "child1" });
    await waitFor(() => expect(toasts()).toContain(`Speed up sent ${CHILD_TXID.slice(0, 12)}…`));
  });

  it("a refused preview reads as the backend's sentence and cannot be signed", async () => {
    const dialog = await openForm({
      preview: () =>
        Promise.reject(
          "Invalid input: Your transaction already pays this rate or more; choose a higher one.",
        ),
    });
    expect(await within(dialog).findByTestId("speed-up-refusal")).toHaveTextContent(
      /^Your transaction already pays this rate or more; choose a higher one\.$/,
    );
    expect(within(dialog).getByTestId("speed-up-submit")).toBeDisabled();
  });

  it("a declined confirmation discards the child, so its parent is not held", async () => {
    const dialog = await openForm();
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) =>
      cmd === "sign_tx_draft"
        ? Promise.reject("Confirmation declined")
        : routes({ drafts: [sendDraft("broadcasted")] })(cmd, args),
    );
    const submit = await within(dialog).findByTestId("speed-up-submit");
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("delete_tx_draft")).toHaveLength(1));
    expect(calls("delete_tx_draft")[0]![1]).toEqual({ draftId: "child1" });
    expect(calls("broadcast_tx_draft")).toHaveLength(0);
  });

  it("a purchase's form says the price and the purchase are unchanged", async () => {
    invokeMock.mockImplementation(
      routes({
        drafts: [sendDraft("broadcasted", { action: "shakedex_purchase" })],
        preview: () => Promise.resolve(preview({ parentIsPurchase: true })),
      }),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const button = await screen.findByTestId("speed-up-button");
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    const dialog = await screen.findByTestId("speed-up-dialog");
    expect(dialog).toHaveTextContent(
      "nobody else is paid, and the seller's price and the purchase itself are unchanged.",
    );
  });

  it("writes rates as the backend's confirmation does", () => {
    expect(rateText(1000)).toBe("1.00 doos/vB");
    expect(rateText(10_000)).toBe("10.00 doos/vB");
    expect(rateText(12_345)).toBe("12.34 doos/vB");
  });
});

describe("Speed up on the Market's Your purchases (R16)", () => {
  function purchase(state: "unconfirmed" | "awaitingFinalize") {
    return {
      name: "nameholdtest",
      shakedex: {
        state,
        // Not counted yet: the column falls through to the Speed up slot.
        blocksRemaining: null,
        purchaseId: "pur1",
        purchaseDraftId: "pdraft1",
        priceDoos: 5_000_000,
        lockupBlocks: 288,
        wallClockTiming: false,
      },
    };
  }

  function renderCard(state: "unconfirmed" | "awaitingFinalize") {
    invokeMock.mockImplementation(routes({}));
    render(
      <PendingPurchases
        purchases={[purchase(state)]}
        finalizeRefusal={null}
        finalizingId={null}
        primary
        onFinalize={() => {}}
      />,
      { wrapper: wrapper() },
    );
  }

  it("an Unconfirmed purchase offers Speed up on its purchase's draft", async () => {
    renderCard("unconfirmed");
    const button = await screen.findByTestId("speed-up-button");
    await waitFor(() => expect(button).toBeEnabled());
    expect(calls("speed_up_check")[0]![1]).toEqual({ draftId: "pdraft1" });
    fireEvent.click(button);
    const dialog = await screen.findByTestId("speed-up-dialog");
    expect(dialog).toHaveTextContent("the seller's price and the purchase itself are unchanged");
  });

  it("a purchase awaiting finalize has no Speed up", async () => {
    renderCard("awaitingFinalize");
    await screen.findByTestId("pending-purchase");
    expect(screen.queryByTestId("speed-up-button")).not.toBeInTheDocument();
  });
});
