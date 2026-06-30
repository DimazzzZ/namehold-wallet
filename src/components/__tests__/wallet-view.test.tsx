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

import { WalletView } from "../WalletView";

const baseProfile = {
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
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

type Overrides = {
  profile?: Partial<typeof baseProfile>;
  profiles?: unknown[];
  unlocked?: boolean;
  canWrite?: boolean;
  draft?: unknown;
  spendableDoos?: number;
  confirmedDoos?: number;
};

function routeInvoke(o: Overrides = {}) {
  const profile = { ...baseProfile, ...(o.profile ?? {}) };
  const unlocked = o.unlocked ?? false;
  const canWrite = o.canWrite ?? false;
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve(o.profiles ?? [profile]);
      case "get_signer_session":
        return Promise.resolve({
          walletProfileId: unlocked ? profile.id : null,
          unlocked,
          unlockedUntilEpochMs: unlocked ? Date.now() + 60000 : 0,
        });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: unlocked,
          broadcasterAvailable: true,
          canWrite,
          reason: canWrite ? null : "Unlock your wallet to sign transactions.",
        });
      case "get_wallet_balances":
        return Promise.resolve({
          liquidDoos: o.spendableDoos ?? 5_000_000,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          totalDoos: o.spendableDoos ?? 5_000_000,
        });
      case "read_balance":
        return Promise.resolve({
          confirmed: o.confirmedDoos ?? 0,
          unconfirmed: 0,
          locked_confirmed: 0,
          locked_unconfirmed: 0,
        });
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "read_names":
        return Promise.resolve([
          { name: "example", state: "CLOSED", height: 100, renewal: 200, owner: { hash: "tx1", index: 0 }, stats: null },
        ]);
      case "build_send_hns_draft":
        return Promise.resolve(
          o.draft ?? {
            id: "d1",
            walletProfileId: profile.id,
            action: "send_hns",
            status: "draft",
            summary: {
              action: "send_hns",
              sendTotalDoos: 1_000_000,
              feeDoos: 1410,
              changeDoos: 3_998_590,
              inputTotalDoos: 5_000_000,
              numInputs: 1,
              recipientAddress: "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2",
              txid: null,
              warnings: [],
            },
            errorMessage: null,
            txid: null,
            createdAt: "2026-01-01",
          },
        );
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
});

describe("WalletView (non-custodial)", () => {
  it("shows a locked signer and an Unlock control, with NO secret inputs in the DOM", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false, canWrite: false }));
    const { container } = render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Signer locked/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Unlock/i })).toBeInTheDocument();

    // The core guarantee: React never renders a password/secret input field.
    expect(container.querySelector('input[type="password"]')).toBeNull();
    // And no mnemonic entry surface exists.
    expect(container.querySelector("textarea")).toBeNull();
  });

  it("the Unlock button delegates to the secure unlock command", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Unlock/i }));

    // Unlocking must go through the secure command (which prompts in the Rust
    // secure window) — never a React-side passphrase path.
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "unlock_local_signer");
      expect(call?.[1]).toEqual({ walletProfileId: "p1" });
    });
  });

  it("a no-passphrase wallet shows one-click unlock copy (no passphrase prompt mention)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { hasPassphrase: false }, unlocked: false }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Signer locked/i)).toBeInTheDocument();
    expect(screen.getByText(/no passphrase.*click Unlock/i)).toBeInTheDocument();
    expect(screen.queryByText(/Unlock with your passphrase/i)).toBeNull();
    expect(screen.getByRole("button", { name: /Unlock/i })).toBeInTheDocument();
  });

  it("a passphrase wallet still shows the secure-window unlock copy", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { hasPassphrase: true }, unlocked: false }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Unlock with your passphrase \(in a secure window\)/i)).toBeInTheDocument();
  });

  it("send dialog collects only address + amount (no passphrase field)", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: true, canWrite: true }));
    const { container } = render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));

    expect(screen.getByText(/Destination Address/i)).toBeInTheDocument();
    expect(screen.getByText(/Amount \(HNS\)/i)).toBeInTheDocument();
    // No passphrase/secret input anywhere in the send flow.
    expect(container.querySelector('input[type="password"]')).toBeNull();
  });

  it("building a draft shows a fee/change preview before broadcast", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: true, canWrite: true }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    fireEvent.change(screen.getByPlaceholderText(/rs1q/i), {
      target: { value: "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2" },
    });
    fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /Review/i }));

    await waitFor(() =>
      expect(screen.getByText(/Sign & Broadcast/i)).toBeInTheDocument(),
    );
    expect(screen.getByText(/Fee/i)).toBeInTheDocument();
    expect(screen.getByText(/Change/i)).toBeInTheDocument();
  });

  it("shows a 'connect & sync a node' hint when explorer balance > 0 but spendable is 0", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ unlocked: true, canWrite: true, spendableDoos: 0, confirmedDoos: 1_400_000 }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(await screen.findByTestId("needs-node-sync")).toBeInTheDocument();
    // Can't send with nothing synced, even though the signer/node are ready.
    expect(screen.getByRole("button", { name: /Send HNS/i })).toBeDisabled();
  });

  it("renders Owned Names from the cache-backed read_names command", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(await screen.findByText(/Owned Names/i)).toBeInTheDocument();
    expect(await screen.findByText(/\.example/)).toBeInTheDocument();
  });

  it("watch-only profiles hide spend + unlock controls", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { watchOnly: true, kind: "watch_only_xpub" } }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Watch-only/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Send HNS/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /Unlock/i })).toBeNull();
  });
});

const secondProfile = {
  ...baseProfile,
  id: "p2",
  label: "Trading",
  receiveAddress: "rs1qsecondaddr",
  active: false,
};

describe("WalletView multi-wallet management", () => {
  it("shows Add wallet + Manage entry points with an active profile", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByRole("button", { name: /\+ Add wallet/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Manage wallets/i })).toBeInTheDocument();
  });

  it("Manage opens a modal listing all wallets; Switch activates a non-active one", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profiles: [{ ...baseProfile, active: true }, secondProfile] }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Manage wallets/i }));

    // Both wallets listed in the dialog.
    expect(await screen.findByText("Trading")).toBeInTheDocument();
    // The non-active wallet (Trading) exposes a Switch action.
    fireEvent.click(screen.getByRole("button", { name: /^Switch$/i }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "set_active_wallet_profile");
      expect(call?.[1]).toEqual({ walletProfileId: "p2" });
    });
  });

  it("deleting the active wallet auto-switches to a remaining one", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    invokeMock.mockImplementation(
      routeInvoke({ profiles: [{ ...baseProfile, active: true }, secondProfile] }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Manage wallets/i }));
    await screen.findByText("Trading");

    // Delete the active wallet (the first Delete button = Primary's row).
    fireEvent.click(screen.getAllByRole("button", { name: /^Delete$/i })[0]!);
    await waitFor(() => {
      expect(invokeMock.mock.calls.find((c) => c[0] === "delete_wallet_profile")?.[1]).toEqual({
        walletProfileId: "p1",
      });
    });
    // …then re-activates the remaining wallet (p2).
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.find((c) => c[0] === "set_active_wallet_profile")?.[1],
      ).toEqual({ walletProfileId: "p2" });
    });
  });

  it("Add wallet → Create uses the entered label (not a hardcoded one)", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /\+ Add wallet/i }));

    // The add form opens on the chooser; pick "Create a new wallet".
    fireEvent.click(await screen.findByText(/Create a new wallet/i));
    const nameInput = screen.getByLabelText(/Wallet Name/i) as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: "Savings" } });
    fireEvent.click(screen.getByRole("button", { name: /Create in secure window/i }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "secure_create_wallet");
      expect(call?.[1]).toEqual({ label: "Savings", network: "mainnet" });
    });
  });

  it("no active profile renders the add-wallet chooser", async () => {
    invokeMock.mockImplementation(routeInvoke({ profiles: [] }));
    render(<WalletView />, { wrapper: wrapper() });

    expect(await screen.findByText(/Import your wallet/i)).toBeInTheDocument();
    expect(screen.getByText(/Create a new wallet/i)).toBeInTheDocument();
    expect(screen.getByText(/Watch-only/i)).toBeInTheDocument();
  });
});
