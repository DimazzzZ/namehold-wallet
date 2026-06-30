import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// The Namebase transfer modal must let the user choose ANY HNS destination, with
// their own wallet pre-filled as the default — and flag when the destination is
// outside their wallet (irreversible withdrawal).

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NamebaseDashboard } from "../NamebaseDashboard";

const WALLET = "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8";
const THIRD_PARTY = "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "mainnet",
  accountXpub: "xpubFAKE",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: WALLET,
  lastSyncedHeight: 10,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function routeInvoke(cmd: string) {
  switch (cmd) {
    case "list_wallet_profiles":
      return Promise.resolve([profile]);
    case "get_namebase_status":
      return Promise.resolve({
        connected: true,
        has_cookie: true,
        account: { balance: { hns: 100, btc: 0 }, pendingHns: 0, has2fa: false, withdrawalFeeHns: 1, minimums: { hns: 1 } },
      });
    case "fetch_namebase_domains":
      return Promise.resolve({
        domains: [
          { name: "exampletld", owner_id: "o1", owned_since: "2024-01-01", auto_renew_active: false, status: "active" },
        ],
      });
    case "fetch_namebase_staked":
      return Promise.resolve({ stakedDomains: [] });
    case "namebase_transfer_domain":
      return Promise.resolve();
    default:
      return Promise.resolve(null);
  }
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

async function openTransferModal() {
  render(<NamebaseDashboard />, { wrapper: wrapper() });
  // Wait for the domain row, then click its Transfer button.
  await screen.findByText(/exampletld/);
  const transferBtn = screen.getAllByRole("button", { name: /^Transfer$/ })[0]!;
  fireEvent.click(transferBtn);
  await screen.findByText(/from Namebase to an HNS address/i);
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeInvoke);
});

describe("Namebase transfer destination", () => {
  it("defaults the destination to the user's wallet and transfers there", async () => {
    await openTransferModal();

    const input = screen.getByPlaceholderText(/hs1/) as HTMLInputElement;
    expect(input.value).toBe(WALLET);
    // Sending to self → no out-of-wallet warning.
    expect(screen.queryByText(/outside this wallet/i)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Confirm Transfer/i }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "namebase_transfer_domain");
      expect(call?.[1]).toEqual({ name: "exampletld", address: WALLET });
    });
  });

  it("allows a third-party destination and warns it is outside the wallet", async () => {
    await openTransferModal();

    const input = screen.getByPlaceholderText(/hs1/) as HTMLInputElement;
    fireEvent.change(input, { target: { value: THIRD_PARTY } });

    // Warning appears once the destination differs from the wallet.
    expect(screen.getByText(/outside this wallet/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Confirm Transfer/i }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "namebase_transfer_domain");
      expect(call?.[1]).toEqual({ name: "exampletld", address: THIRD_PARTY });
    });
  });

  it("disables Confirm when the destination is cleared", async () => {
    await openTransferModal();
    const input = screen.getByPlaceholderText(/hs1/) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "   " } });
    expect(screen.getByRole("button", { name: /Confirm Transfer/i })).toBeDisabled();
  });

  it("resets the destination to the wallet when the transfer modal is reopened", async () => {
    // Guards against a stale third-party address leaking from one transfer into
    // the next — which would send a domain to the wrong place.
    await openTransferModal();
    const input = () => screen.getByPlaceholderText(/hs1/) as HTMLInputElement;
    fireEvent.change(input(), { target: { value: THIRD_PARTY } });
    expect(input().value).toBe(THIRD_PARTY);

    // Cancel, then reopen the transfer modal.
    fireEvent.click(screen.getByRole("button", { name: /^Cancel$/i }));
    fireEvent.click(screen.getAllByRole("button", { name: /^Transfer$/ })[0]!);
    await screen.findByText(/from Namebase to an HNS address/i);

    await waitFor(() => expect(input().value).toBe(WALLET));
  });
});

describe("Namebase Withdraw HNS", () => {
  async function openWithdraw() {
    render(<NamebaseDashboard />, { wrapper: wrapper() });
    // Wait for the connected account panel, then open the withdraw modal.
    fireEvent.click(await screen.findByRole("button", { name: /Withdraw HNS/i }));
    await screen.findByText(/Withdraw HNS from your Namebase balance/i);
  }

  it("defaults the destination to the wallet and sends the amount in doos", async () => {
    await openWithdraw();
    const dest = screen.getByPlaceholderText(/hs1/) as HTMLInputElement;
    expect(dest.value).toBe(WALLET);

    fireEvent.change(screen.getByPlaceholderText("0.0"), { target: { value: "2" } });
    // Breakdown: recipient gets 2, fee 1, total debited 3.
    expect(screen.getByText("Total debited")).toBeInTheDocument();
    expect(screen.getByText("3 HNS")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Withdraw$/i }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "namebase_withdraw_hns");
      // The user enters the NET (2); we send the GROSS (net + 1 fee = "3").
      expect(call?.[1]).toEqual({ address: WALLET, amount: "3" });
    });
  });

  it("blocks Withdraw with no amount and warns on a third-party destination", async () => {
    await openWithdraw();
    // No amount yet → confirm disabled.
    expect(screen.getByRole("button", { name: /^Withdraw$/i })).toBeDisabled();

    fireEvent.change(screen.getByPlaceholderText("0.0"), { target: { value: "2" } });
    fireEvent.change(screen.getByPlaceholderText(/hs1/), { target: { value: THIRD_PARTY } });
    expect(screen.getByText(/outside this wallet/i)).toBeInTheDocument();
  });

  it("blocks an amount exceeding the available balance", async () => {
    await openWithdraw();
    fireEvent.change(screen.getByPlaceholderText("0.0"), { target: { value: "999" } }); // > 100 avail
    expect(screen.getByRole("button", { name: /^Withdraw$/i })).toBeDisabled();
  });

  it("blocks a gross amount below the Namebase minimum", async () => {
    // Override the account so the minimum (5 HNS) exceeds net+fee, exercising the
    // min-amount gate (not the balance gate).
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_namebase_status") {
        return Promise.resolve({
          connected: true,
          has_cookie: true,
          account: {
            balance: { hns: 100, btc: 0 },
            pendingHns: 0,
            has2fa: false,
            withdrawalFeeHns: 1,
            minimums: { hns: 5 },
          },
        });
      }
      return routeInvoke(cmd);
    });

    await openWithdraw();
    // net 1 + fee 1 = gross 2, which is below the 5 HNS minimum → blocked.
    fireEvent.change(screen.getByPlaceholderText("0.0"), { target: { value: "1" } });
    expect(screen.getByRole("button", { name: /^Withdraw$/i })).toBeDisabled();
  });
});
