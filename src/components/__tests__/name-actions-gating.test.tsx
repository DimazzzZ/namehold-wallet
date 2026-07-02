import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NameActionsModal } from "../NameActionsModal";

const profile = {
  id: "p1",
  label: "Primary",
  network: "mainnet",
  receiveAddress: "hs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function route(canWrite: boolean, reason: string | null) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: canWrite,
          canWrite,
          reason,
        });
      case "read_name_info":
        return Promise.resolve({
          name: "examplename",
          state: "CLOSED",
          registered: true,
          height: 5040,
          renewal: 329999,
          owner: { hash: "deadbeef", address: "hs1qwallet" },
          value: 100000,
          highest: 100000,
          stats: { openPeriodStart: 5000, biddingPeriodEnd: 5040, revealPeriodEnd: 5540 },
        });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("NameActionsModal — node-readiness gating", () => {
  it("blocks every name action with the reason when the node can't write", async () => {
    invokeMock.mockImplementation(
      route(false, "Your local node is still syncing (40%). On-chain sends and transfers need a fully-synced node."),
    );
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // Wait for write-capability gating to settle, then assert on the banner itself.
    const blocked = await screen.findByTestId("name-actions-blocked");
    expect(blocked).toHaveTextContent(/name actions unavailable/i);

    // The mocked name is CLOSED/owned, so the guided action is Register and it
    // should be disabled.
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /^Register$/i })).toBeDisabled();
    });

    // Advanced actions are behind a toggle — open them to verify gating on actions
    // that are always present in the auction section for the current modal contract.
    fireEvent.click(screen.getByTestId("all-actions-toggle"));
    expect(screen.getAllByRole("button", { name: /^Open$/i }).at(-1)).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Reveal$/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Redeem$/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Bid$/i })).toBeDisabled();
    // Close stays available.
    expect(screen.getByRole("button", { name: /^Close$/i })).not.toBeDisabled();
  });

  it("enables actions once the node is write-capable", async () => {
    invokeMock.mockImplementation(route(true, null));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // No blocked banner.
    await waitForWritable();
    expect(screen.queryByTestId("name-actions-blocked")).toBeNull();
    // Advanced actions are behind a toggle — open them to verify.
    fireEvent.click(screen.getByTestId("all-actions-toggle"));
    // In the current modal contract, always-available auction actions should be enabled.
    expect(screen.getAllByRole("button", { name: /^Open$/i }).at(-1)).not.toBeDisabled();
    expect(screen.getByRole("button", { name: /^Reveal$/i })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: /^Redeem$/i })).not.toBeDisabled();
  });
});

// Small helper: the write-capability query resolves async; wait for the
// blocked banner to disappear (i.e., canWrite=true has been applied).
async function waitForWritable() {
  const { waitFor } = await import("@testing-library/react");
  await waitFor(() => expect(screen.queryByTestId("name-actions-blocked")).toBeNull());
}
