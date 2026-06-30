import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// Issue 6: each wallet must show its OWN balance (no cross-wallet bleed), keep it
// across navigation, and update ONLY on Refresh — never auto-refetch.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";

const mkProfile = (id: string, label: string, active: boolean) => ({
  id,
  label,
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: `rs1q${id}`,
  lastSyncedHeight: 1,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active,
});

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

// A stateful mock: tracks the active profile and per-wallet liquid balances, so
// the queries behave like the real per-profile backend.
function makeBackend(initial: { A: number; B: number }) {
  const state = { active: "A", liquid: { ...initial } as Record<string, number> };
  const impl = (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([
          mkProfile("A", "Wallet A", state.active === "A"),
          mkProfile("B", "Wallet B", state.active === "B"),
        ]);
      case "set_active_wallet_profile":
        state.active = String(args?.walletProfileId);
        return Promise.resolve(mkProfile(state.active, `Wallet ${state.active}`, true));
      case "get_wallet_balances": {
        const id = String(args?.walletProfileId ?? state.active);
        return Promise.resolve({
          liquidDoos: state.liquid[id] ?? 0,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          totalDoos: state.liquid[id] ?? 0,
        });
      }
      case "read_balance":
        return Promise.resolve({ confirmed: 0, unconfirmed: 0, locked_confirmed: 0, locked_unconfirmed: 0 });
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: null, unlocked: false, unlockedUntilEpochMs: 0 });
      case "get_write_capability":
        return Promise.resolve({ signerUnlocked: false, broadcasterAvailable: false, canWrite: false, reason: null });
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "read_names":
        return Promise.resolve([]);
      case "sync_wallet_state":
        return Promise.resolve({ nodeReachable: false });
      case "discover_owned_names":
        return Promise.resolve({ discovered: 0 });
      default:
        return Promise.resolve(null);
    }
  };
  return { state, impl };
}

const HNS = 1_000_000; // doos per HNS

beforeEach(() => invokeMock.mockReset());

describe("Per-wallet balance persistence (Issue 6)", () => {
  it("shows each wallet's own spendable balance, with no cross-wallet bleed on switch", async () => {
    const { impl } = makeBackend({ A: 100 * HNS, B: 200 * HNS });
    invokeMock.mockImplementation(impl);
    render(<WalletView />, { wrapper: wrapper() });

    // Wallet A's own balance.
    expect(await screen.findByText("100.000000")).toBeInTheDocument();
    expect(screen.queryByText("200.000000")).toBeNull();

    // Switch to Wallet B → shows B's balance, not A's.
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "B" } });
    expect(await screen.findByText("200.000000")).toBeInTheDocument();
    expect(screen.queryByText("100.000000")).toBeNull();

    // get_wallet_balances was queried per-profile (A then B).
    const balCalls = invokeMock.mock.calls.filter((c) => c[0] === "get_wallet_balances");
    expect(balCalls.some((c) => (c[1] as { walletProfileId?: string })?.walletProfileId === "A")).toBe(true);
    expect(balCalls.some((c) => (c[1] as { walletProfileId?: string })?.walletProfileId === "B")).toBe(true);
  });

  it("does not auto-refetch — the balance changes only after Refresh", async () => {
    const backend = makeBackend({ A: 100 * HNS, B: 200 * HNS });
    invokeMock.mockImplementation(backend.impl);
    render(<WalletView />, { wrapper: wrapper() });

    expect(await screen.findByText("100.000000")).toBeInTheDocument();
    const callsForA = () =>
      invokeMock.mock.calls.filter(
        (c) => c[0] === "get_wallet_balances" && (c[1] as { walletProfileId?: string })?.walletProfileId === "A",
      ).length;
    const initialCalls = callsForA();

    // The backend value changes, but with no Refresh the UI must NOT update.
    backend.state.liquid.A = 150 * HNS;
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.getByText("100.000000")).toBeInTheDocument();
    expect(callsForA()).toBe(initialCalls); // no auto-refetch

    // Refresh → the balance query is invalidated and picks up the new value.
    fireEvent.click(screen.getByRole("button", { name: /Refresh/i }));
    expect(await screen.findByText("150.000000")).toBeInTheDocument();
    await waitFor(() => expect(callsForA()).toBeGreaterThan(initialCalls));
  });
});
