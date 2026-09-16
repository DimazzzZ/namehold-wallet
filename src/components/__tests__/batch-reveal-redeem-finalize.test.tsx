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
 * Build a NameActionCapabilities row with per-action toggles. Each of
 * `canReveal` / `canRedeem` / `canFinalize` gates the matching batch button.
 */
function cap(name: string, opts: { reveal?: boolean; redeem?: boolean; finalize?: boolean } = {}) {
  const yes = { allowed: true, reason: null };
  const no = { allowed: false, reason: "not allowed" };
  return {
    name,
    phase: "CLOSED",
    height: 100,
    renewalBlock: null,
    canOpen: no,
    canBid: no,
    canReveal: opts.reveal ? yes : no,
    canRedeem: opts.redeem ? yes : no,
    canRegister: no,
    canRenew: yes,
    canUpdate: yes,
    canTransfer: no,
    canFinalize: opts.finalize ? yes : no,
    canRevoke: no,
  };
}

interface Overrides {
  caps?: unknown[];
}

function draftFor(action: string) {
  return {
    id: `draft-${action}-001`,
    walletProfileId: profile.id,
    action: `batch-${action}`,
    status: "draft",
    summary: {
      action: `batch-${action}`,
      sendTotalDoos: 0,
      feeDoos: 2620,
      changeDoos: 0,
      inputTotalDoos: 10_002_620,
      numInputs: 2,
      recipientAddress: null,
      txid: null,
      warnings: [],
      nameList: ["alpha", "bravo"],
    },
    errorMessage: null,
    txid: null,
    createdAt: "2026-01-01",
  };
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
        return Promise.resolve(
          o.caps ?? [
            cap("alpha", { reveal: true, redeem: true, finalize: true }),
            cap("bravo", { reveal: true, redeem: true, finalize: true }),
          ],
        );
      case "build_batch_reveal_draft":
        return Promise.resolve(draftFor("reveal"));
      case "build_batch_redeem_draft":
        return Promise.resolve(draftFor("redeem"));
      case "build_batch_finalize_draft":
        return Promise.resolve(draftFor("finalize"));
      case "sign_tx_draft":
        return Promise.resolve({ id: "draft-x-001" });
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
  // boxes[0] is the header select-all checkbox; one click selects every
  // filtered name (alpha + bravo).
  fireEvent.click(boxes[0]!);
  await screen.findByTestId("batch-reveal-btn");
}

beforeEach(() => {
  invokeMock.mockReset();
  Element.prototype.scrollIntoView = vi.fn();
});

describe("WalletView — batch reveal/redeem/finalize", () => {
  it.each([
    ["reveal", "batch-reveal-btn", "build_batch_reveal_draft"],
    ["redeem", "batch-redeem-btn", "build_batch_redeem_draft"],
    ["finalize", "batch-finalize-btn", "build_batch_finalize_draft"],
  ])(
    "clicking %s invokes %s with { names, feeRate: undefined }",
    async (_action, testid, command) => {
      invokeMock.mockImplementation(routeInvoke());
      render(<WalletView />, { wrapper: wrapper() });
      await screen.findByText("Primary");
      await selectBothNames();

      fireEvent.click(screen.getByTestId(testid));

      await waitFor(() => {
        const call = invokeMock.mock.calls.find((c) => c[0] === command);
        expect(call).toBeTruthy();
        expect(call?.[1]).toEqual({ names: ["alpha", "bravo"], feeRate: undefined });
      });
    },
  );

  it.each([
    ["reveal", "batch-reveal-btn", { reveal: false, redeem: true, finalize: true }],
    ["redeem", "batch-redeem-btn", { reveal: true, redeem: false, finalize: true }],
    ["finalize", "batch-finalize-btn", { reveal: true, redeem: true, finalize: false }],
  ])(
    "%s button is disabled when any selected name isn't eligible",
    async (_action, testid, flags) => {
      invokeMock.mockImplementation(
        routeInvoke({
          caps: [cap("alpha", { reveal: true, redeem: true, finalize: true }), cap("bravo", flags)],
        }),
      );
      render(<WalletView />, { wrapper: wrapper() });
      await screen.findByText("Primary");
      await selectBothNames();

      expect(screen.getByTestId(testid)).toBeDisabled();
    },
  );

  it("after building a reveal draft, confirming signs then broadcasts", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    await selectBothNames();

    fireEvent.click(screen.getByTestId("batch-reveal-btn"));

    const confirmBtn = await screen.findByRole("button", { name: /confirm/i });
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(invokeMock.mock.calls.some((c) => c[0] === "sign_tx_draft")).toBe(true);
      expect(invokeMock.mock.calls.some((c) => c[0] === "broadcast_tx_draft")).toBe(true);
    });
  });
});
