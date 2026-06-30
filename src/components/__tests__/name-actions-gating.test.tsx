import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
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

    // The blocked banner shows the precise reason (once write-capability loads)…
    expect(await screen.findByText(/still syncing \(40%\)/i)).toBeInTheDocument();
    expect(screen.getByTestId("name-actions-blocked")).toBeInTheDocument();
    // …and the spend actions are disabled, so no confusing backend error fires.
    expect(screen.getByRole("button", { name: /^Transfer$/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Finalize$/i })).toBeDisabled();
    // Close stays available.
    expect(screen.getByRole("button", { name: /^Close$/i })).not.toBeDisabled();
  });

  it("enables actions once the node is write-capable", async () => {
    invokeMock.mockImplementation(route(true, null));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // No blocked banner; Transfer becomes available once a recipient is entered.
    await waitForWritable();
    expect(screen.queryByTestId("name-actions-blocked")).toBeNull();
    // Transfer needs a recipient; Finalize (no input) should be enabled now.
    expect(screen.getByRole("button", { name: /^Finalize$/i })).not.toBeDisabled();
  });
});

// Small helper: the write-capability query resolves async; wait for the
// blocked banner to disappear (i.e., canWrite=true has been applied).
async function waitForWritable() {
  const { waitFor } = await import("@testing-library/react");
  await waitFor(() => expect(screen.queryByTestId("name-actions-blocked")).toBeNull());
}
