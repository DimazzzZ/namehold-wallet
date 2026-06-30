import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// Auction-UX behaviours added after the live auction GUI test:
//   * a broadcast tx visibly settles Pending → Confirmed (and Not confirmed);
//   * "Locked in Auctions" balance is surfaced when a bid lockup exists;
//   * a reveal-required alert appears for names in the REVEAL phase;
//   * the name modal shows the live phase + countdown and the DNS row editor
//     serializes to the record array the build_*_draft commands expect.

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

import { WalletView } from "../WalletView";
import { NameActionsModal } from "../NameActionsModal";

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

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/wallet"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("WalletView — auction UX", () => {
  function routeWallet(opts: {
    drafts?: unknown[];
    names?: unknown[];
    lockupDoos?: number;
  }) {
    return (cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "get_wallet_balances":
          return Promise.resolve({
            liquidDoos: 5_000_000,
            nameControlDoos: 0,
            nameLockupDoos: opts.lockupDoos ?? 0,
            totalDoos: 5_000_000 + (opts.lockupDoos ?? 0),
          });
        case "read_balance":
          return Promise.resolve({ confirmed: 0, unconfirmed: 0, locked_confirmed: 0, locked_unconfirmed: 0 });
        case "list_tx_drafts":
          return Promise.resolve(opts.drafts ?? []);
        case "read_names":
          return Promise.resolve(opts.names ?? []);
        default:
          return Promise.resolve(null);
      }
    };
  }

  const draft = (over: Record<string, unknown>) => ({
    id: "d1",
    walletProfileId: "p1",
    action: "send_hns",
    status: "broadcasted",
    summary: { action: "send_hns", sendTotalDoos: 1_000_000, feeDoos: 1410 },
    errorMessage: null,
    txid: "abcdef0123456789",
    confirmationHeight: null,
    createdAt: "2026-01-01",
    ...over,
  });

  it("renders Pending / Confirmed / Not confirmed for tx statuses", async () => {
    invokeMock.mockImplementation(
      routeWallet({
        drafts: [
          draft({ id: "a", status: "confirmed", confirmationHeight: 437 }),
          draft({ id: "b", status: "broadcasted" }),
          draft({ id: "c", status: "dropped", errorMessage: "never confirmed" }),
        ],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    expect(await screen.findByText(/Confirmed · #437/)).toBeInTheDocument();
    expect(screen.getByText("Pending")).toBeInTheDocument();
    expect(screen.getByText("Not confirmed")).toBeInTheDocument();
  });

  it("shows the Locked in Auctions balance only when a lockup exists", async () => {
    invokeMock.mockImplementation(routeWallet({ lockupDoos: 2_000_000 }));
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    const card = await screen.findByTestId("balance-locked-auctions");
    expect(card).toHaveTextContent("Locked in Auctions");
    expect(card).toHaveTextContent("2.000000");
  });

  it("hides the Locked in Auctions card when there is no lockup", async () => {
    invokeMock.mockImplementation(routeWallet({ lockupDoos: 0 }));
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    expect(screen.queryByTestId("balance-locked-auctions")).toBeNull();
  });

  it("raises a reveal-required alert for names in the REVEAL phase", async () => {
    invokeMock.mockImplementation(
      routeWallet({
        names: [{ name: "examplename", state: "REVEAL", height: 1, renewal: 2, owner: { hash: "t", index: 0 }, stats: null }],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    const alert = await screen.findByTestId("reveal-alert");
    expect(alert).toHaveTextContent(/Action required: reveal/i);
    expect(alert).toHaveTextContent(".examplename");
  });
});

describe("NameActionsModal — phase header + DNS editor", () => {
  function routeModal(captured: { records?: unknown }) {
    return (cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "read_name_info":
          return Promise.resolve({
            name: "cuatesttld",
            state: "CLOSED",
            height: 100,
            renewal: 200,
            owner: { hash: "t", index: 0 },
            value: 1_000_000,
            highest: 2_000_000,
            stats: { blocksUntilExpire: 100 },
          });
        case "build_register_draft":
          captured.records = args?.records;
          return Promise.resolve({ id: "reg1", status: "draft" });
        case "sign_tx_draft":
          return Promise.resolve({ id: "reg1", status: "signed" });
        case "broadcast_tx_draft":
          return Promise.resolve({ draftId: "reg1", txid: "f".repeat(64), status: "broadcasted" });
        default:
          return Promise.resolve(null);
      }
    };
  }

  it("shows the live phase badge and the recommended action", async () => {
    invokeMock.mockImplementation(routeModal({}));
    render(<NameActionsModal name="cuatesttld" open onClose={() => {}} />, { wrapper: wrapper() });

    // Badge starts "Available" then settles to the live phase once read_name_info resolves.
    expect(await screen.findByText("Closed")).toBeInTheDocument();
    expect(within(await screen.findByTestId("name-phase")).getByText("Closed")).toBeInTheDocument();
    // CLOSED → recommend Register.
    expect(await screen.findByTestId("name-recommended")).toHaveTextContent(/Register/i);
  });

  it("serializes the DNS row editor into the records array on Register", async () => {
    const captured: { records?: unknown } = {};
    invokeMock.mockImplementation(routeModal(captured));
    render(<NameActionsModal name="cuatesttld" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByTestId("dns-rows");
    // Default first row is a TXT — fill its value, then Register.
    fireEvent.change(screen.getByLabelText("record value"), {
      target: { value: "cua-agent-verified" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Register$/i }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).toContain("build_register_draft");
    });
    expect(captured.records).toEqual([{ type: "TXT", txt: ["cua-agent-verified"] }]);
  });
});
