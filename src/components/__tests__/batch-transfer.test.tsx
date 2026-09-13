import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));
vi.mock("../../lib/clipboard", () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";

const RECIPIENT = "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE000000000000",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "rs1qexamplereceiveaddr",
  lastSyncedHeight: 10,
  lastSyncedAt: null as string | null,
  lastExplorerSyncAt: null as string | null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

const names = [
  {
    name: "alpha",
    state: "CLOSED",
    height: 100,
    renewal: 200,
    owner: { hash: "t1", index: 0 },
    stats: null,
  },
  {
    name: "bravo",
    state: "CLOSED",
    height: 101,
    renewal: 201,
    owner: { hash: "t2", index: 0 },
    stats: null,
  },
];

/**
 * Build a NameActionCapabilities row. `canTransfer` toggles whether the
 * batch-transfer button treats this name as eligible.
 */
function cap(name: string, canTransfer: boolean, reason: string | null = null) {
  const yes = { allowed: true, reason: null };
  const no = { allowed: false, reason: reason ?? "not allowed" };
  return {
    name,
    phase: "CLOSED",
    height: 100,
    renewalBlock: null,
    canOpen: no,
    canBid: no,
    canReveal: no,
    canRedeem: no,
    canRegister: no,
    canRenew: yes,
    canUpdate: yes,
    canTransfer: canTransfer ? yes : no,
    canFinalize: no,
    canRevoke: no,
  };
}

interface Overrides {
  caps?: unknown[];
  buildResult?: unknown;
  buildError?: unknown;
}

function routeInvoke(o: Overrides = {}) {
  return (cmd: string, _args?: unknown) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({
          walletProfileId: profile.id,
          unlocked: true,
          unlockedUntilEpochMs: Date.now() + 60000,
        });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: true,
          canWrite: true,
          reason: null,
        });
      case "get_wallet_balances":
        return Promise.resolve({
          liquidDoos: 5_000_000,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          totalDoos: 5_000_000,
        });
      case "read_balance":
        return Promise.resolve({
          confirmed: 0,
          unconfirmed: 0,
          locked_confirmed: 0,
          locked_unconfirmed: 0,
        });
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "read_action_history":
        return Promise.resolve([]);
      case "refresh_tx_confirmations":
        return Promise.resolve(null);
      case "read_renewals":
        return Promise.resolve({
          walletProfileId: profile.id,
          currentHeight: null,
          heightSource: "unknown",
          expiringSoonThresholdDays: 30,
          names: [],
        });
      case "read_names":
        return Promise.resolve(names);
      case "get_names_action_capabilities":
        return Promise.resolve(o.caps ?? [cap("alpha", true), cap("bravo", true)]);
      case "build_batch_transfer_draft":
        if (o.buildError) return Promise.reject(o.buildError);
        return Promise.resolve(
          o.buildResult ?? {
            id: "draft-batch-transfer-001",
            walletProfileId: profile.id,
            action: "batch-transfer",
            status: "draft",
            summary: {
              action: "batch-transfer",
              sendTotalDoos: 10_000_000,
              feeDoos: 2620,
              changeDoos: 0,
              inputTotalDoos: 10_002_620,
              numInputs: 3,
              recipientAddress: RECIPIENT,
              txid: null,
              warnings: [],
              nameList: ["alpha", "bravo"],
            },
            errorMessage: null,
            txid: null,
            createdAt: "2026-01-01",
          },
        );
      case "sign_tx_draft":
        return Promise.resolve({ id: "draft-batch-transfer-001" });
      case "broadcast_tx_draft":
        return Promise.resolve({ txid: "deadbeef0011223344", status: "ok" });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/wallet"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

async function selectBothNames() {
  await screen.findByText(/\.alpha/);
  await screen.findByText(/\.bravo/);
  const boxes = document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
  // boxes[0] is the header select-all checkbox; a single click selects every
  // filtered name (alpha + bravo). boxes[1]/[2] are the per-row checkboxes.
  fireEvent.click(boxes[0]!);
  await screen.findByTestId("batch-transfer-btn");
}

beforeEach(() => {
  invokeMock.mockReset();
  Element.prototype.scrollIntoView = vi.fn();
});

describe("WalletView — batch transfer", () => {
  it("Transfer Selected is disabled when the recipient input is empty", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    await selectBothNames();

    const btn = screen.getByTestId("batch-transfer-btn");
    expect(btn).toBeDisabled();

    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    expect(btn).not.toBeDisabled();
  });

  it("Transfer Selected is disabled when any selected name isn't transferable", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ caps: [cap("alpha", true), cap("bravo", false, "already mid-transfer")] }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    await selectBothNames();

    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    expect(screen.getByTestId("batch-transfer-btn")).toBeDisabled();
  });

  it("clicking Transfer invokes build_batch_transfer_draft with { names, recipient, feeRate: undefined }", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    await selectBothNames();

    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    fireEvent.click(screen.getByTestId("batch-transfer-btn"));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "build_batch_transfer_draft");
      expect(call).toBeTruthy();
      expect(call?.[1]).toEqual({
        names: ["alpha", "bravo"],
        recipient: RECIPIENT,
        feeRate: undefined,
      });
    });
  });

  it("after build, confirming the modal signs then broadcasts and shows the recipient", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    await selectBothNames();

    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    fireEvent.click(screen.getByTestId("batch-transfer-btn"));

    await waitFor(() => expect(screen.getByText(/Confirm batch transfer/i)).toBeInTheDocument());
    const recipientRow = screen.getByTestId("batch-transfer-recipient");
    expect(recipientRow).toHaveTextContent(RECIPIENT);

    fireEvent.click(screen.getByRole("button", { name: /^Confirm$/i }));

    await waitFor(() => {
      const signCall = invokeMock.mock.calls.find((c) => c[0] === "sign_tx_draft");
      const bcastCall = invokeMock.mock.calls.find((c) => c[0] === "broadcast_tx_draft");
      expect(signCall?.[1]).toEqual({ draftId: "draft-batch-transfer-001" });
      expect(bcastCall?.[1]).toEqual({ draftId: "draft-batch-transfer-001" });
    });
  });

  it("a build rejection surfaces a humanized toast (no [object Object])", async () => {
    invokeMock.mockImplementation(routeInvoke({ buildError: new Error("insufficient funds") }));
    const uiStore = await import("../../stores/ui");
    uiStore.useUiStore.getState().clearToast();

    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    await selectBothNames();

    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    fireEvent.click(screen.getByTestId("batch-transfer-btn"));

    await waitFor(() => {
      const queue = uiStore.useUiStore.getState().toastQueue;
      expect(queue.length).toBeGreaterThan(0);
      const msg = queue.map((t: { message: string }) => t.message).join(" | ");
      expect(msg).toMatch(/Batch transfer failed/i);
      expect(msg).not.toMatch(/\[object Object\]/);
    });
  });
});
