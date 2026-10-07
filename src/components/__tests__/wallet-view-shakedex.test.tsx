import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
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
import { dispatchAction } from "../../lib/actionBus";
import { useUiStore } from "../../stores/ui";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";
import type { ShakedexNameState, WalletProfileSummary } from "../../types";

const DNS_HINT = "This name still carries the seller's DNS records. Update them.";

function purchaseRow(shakedex: ShakedexNameState) {
  return { name: "dexreviews", state: "CLOSED", height: 100, renewal: 200, stats: null, shakedex };
}

function route(names: unknown[], profile: WalletProfileSummary = makeProfile(), canSend = true) {
  const session = makeSession({ unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve(session);
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: canSend,
          canWrite: canSend,
          reason: null,
        });
      case "get_wallet_balances":
        return Promise.resolve({
          liquidDoos: 5_000_000,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          immatureDoos: 0,
          immatureInBlocks: null,
          totalDoos: 5_000_000,
        });
      case "read_balance":
        return Promise.resolve({
          confirmed: 0,
          unconfirmed: 0,
          locked_confirmed: 0,
          locked_unconfirmed: 0,
        });
      case "read_names":
        return Promise.resolve(names);
      case "list_tx_drafts":
      case "read_action_history":
        return Promise.resolve([]);
      case "shakedex_build_purchase_finalize_draft":
        return Promise.resolve({
          id: "fd1",
          walletProfileId: profile.id,
          action: "purchase_finalize",
          status: "draft",
          summary: {
            action: "purchase_finalize",
            sendTotalDoos: 0,
            feeDoos: 1000,
            changeDoos: 0,
            inputTotalDoos: 1000,
            numInputs: 1,
            recipientAddress: null,
            txid: null,
            warnings: [DNS_HINT],
          },
          errorMessage: null,
          txid: null,
          confirmationHeight: null,
          createdAt: "2026-01-01",
        });
      case "sign_tx_draft":
        return Promise.resolve({});
      case "broadcast_tx_draft":
        return Promise.resolve({ draftId: "fd1", txid: "ab".repeat(32), status: "broadcast" });
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

beforeEach(() => {
  invokeMock.mockReset();
  useUiStore.setState({ toastQueue: [] });
  Element.prototype.scrollIntoView = vi.fn();
});

describe("WalletView purchases", () => {
  it("shows Unconfirmed purchase", async () => {
    invokeMock.mockImplementation(
      route([purchaseRow({ state: "unconfirmed", blocksRemaining: null, purchaseId: "p1" })]),
    );
    render(<WalletView />, { wrapper: wrapper() });
    expect(await screen.findByText("Unconfirmed purchase")).toBeInTheDocument();
    expect(screen.queryByTestId("owned-name-finalize")).toBeNull();
  });

  it("shows Awaiting finalize with blocks and no Finalize yet", async () => {
    invokeMock.mockImplementation(
      route([purchaseRow({ state: "awaitingFinalize", blocksRemaining: 214, purchaseId: "p1" })]),
    );
    render(<WalletView />, { wrapper: wrapper() });
    expect(await screen.findByText("Awaiting finalize · 214 blocks")).toBeInTheDocument();
    expect(screen.queryByTestId("owned-name-finalize")).toBeNull();
  });

  it("offers Finalize when ready and runs it, then shows the DNS hint", async () => {
    invokeMock.mockImplementation(
      route([purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" })]),
    );
    render(<WalletView />, { wrapper: wrapper() });
    expect(await screen.findByText("Ready to finalize")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("owned-name-finalize"));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "shakedex_build_purchase_finalize_draft",
      );
      expect(call?.[1]).toEqual({ purchaseId: "p1", feeRate: null });
      expect(invokeMock.mock.calls.some((c) => c[0] === "sign_tx_draft")).toBe(true);
      expect(invokeMock.mock.calls.some((c) => c[0] === "broadcast_tx_draft")).toBe(true);
    });
    await waitFor(() => {
      const messages = useUiStore.getState().toastQueue.map((t) => t.message);
      expect(messages.some((m) => m.startsWith("Finalize broadcast"))).toBe(true);
      expect(messages).toContain(DNS_HINT);
    });
  });

  it("discards the draft when signing is cancelled", async () => {
    const base = route([
      purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" }),
    ]);
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "sign_tx_draft" ? Promise.reject("cancelled") : base(cmd),
    );
    render(<WalletView />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("owned-name-finalize"));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "delete_tx_draft");
      expect(call?.[1]).toEqual({ draftId: "fd1" });
    });
    expect(invokeMock.mock.calls.some((c) => c[0] === "broadcast_tx_draft")).toBe(false);
  });

  it("purchase rows have no Manage button or batch checkbox", async () => {
    invokeMock.mockImplementation(
      route([purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" })]),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Ready to finalize");
    expect(screen.queryByTestId("owned-name-manage")).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Select .*dexreviews/ })).toBeNull();
    // Purchase rows are kept out of the capabilities batch as well.
    await waitFor(() => {
      const caps = invokeMock.mock.calls.filter((c) => c[0] === "get_names_action_capabilities");
      for (const c of caps) expect(JSON.stringify(c[1])).not.toContain("dexreviews");
    });
  });

  it("watch-only profiles get no Finalize", async () => {
    invokeMock.mockImplementation(
      route(
        [purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" })],
        makeProfile({ watchOnly: true }),
      ),
    );
    render(<WalletView />, { wrapper: wrapper() });
    expect(await screen.findByText("Ready to finalize")).toBeInTheDocument();
    expect(screen.queryByTestId("owned-name-finalize")).toBeNull();
  });

  it.each([
    [
      "a ledger profile",
      makeProfile({ kind: "ledger_hardware" }),
      true,
      "Shakedex works with a recovery-phrase wallet for now",
    ],
    [
      "a node that cannot send",
      makeProfile(),
      false,
      "Shakedex needs a local node, or a remote node with sending allowed",
    ],
  ])("%s sees Finalize disabled with the backend's reason", async (_, profile, canSend, reason) => {
    invokeMock.mockImplementation(
      route(
        [purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" })],
        profile,
        canSend,
      ),
    );
    render(<WalletView />, { wrapper: wrapper() });
    const button = await screen.findByTestId("owned-name-finalize");
    await waitFor(() => expect(button).toBeDisabled());
    fireEvent.mouseEnter(button.parentElement!);
    expect(await screen.findByText(reason)).toBeInTheDocument();
    fireEvent.click(button);
    expect(
      invokeMock.mock.calls.some((c) => c[0] === "shakedex_build_purchase_finalize_draft"),
    ).toBe(false);
  });

  it("the name of a purchase row opens no dialog", async () => {
    invokeMock.mockImplementation(
      route([purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" })]),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Ready to finalize");
    expect(screen.queryByTestId("owned-name-info-link")).toBeNull();
    const label = screen.getByTestId("owned-name-purchase-label");
    expect(label).toHaveTextContent(".dexreviews");
    fireEvent.click(label);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keyboard open on a selected purchase row opens no dialog", async () => {
    invokeMock.mockImplementation(
      route([purchaseRow({ state: "awaitingFinalize", blocksRemaining: 0, purchaseId: "p1" })]),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Ready to finalize");
    act(() => dispatchAction("wallet:list:next"));
    act(() => dispatchAction("wallet:list:open"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
