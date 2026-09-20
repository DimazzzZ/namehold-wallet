import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor } from "@testing-library/react";
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

/**
 * `reason` is what the capability rows carry; `writeReason` is what
 * `get_write_capability` reports. They default to the same string — a test that
 * needs to tell the two notices apart passes them separately.
 */
function route(
  canWrite: boolean,
  reason: string | null,
  signerUnlocked = true,
  writeReason: string | null = reason,
) {
  return (cmd: string) => {
    if (cmd === "get_name_action_capabilities") {
      return Promise.resolve({
        name: "examplename",
        phase: "CLOSED",
        taskState: "wonNeedsRegister",
        ownsName: true,
        nameIsRegistered: false,
        transferPending: false,
        hasBidCommitment: false,
        hasRevealCoin: false,
        hasOwnerCoin: true,
        canOpen: { allowed: false, reason: null },
        canBid: { allowed: false, reason: "Phase is CLOSED" },
        canReveal: { allowed: false, reason: "No commitment" },
        canRedeem: { allowed: false, reason: null },
        canRegister: { allowed: canWrite, reason: canWrite ? null : reason },
        canUpdate: { allowed: canWrite, reason: canWrite ? null : reason },
        canTransfer: { allowed: canWrite, reason: canWrite ? null : reason },
        canFinalize: { allowed: false, reason: null },
        canCancelTransfer: { allowed: false, reason: null },
        canRenew: { allowed: canWrite, reason: canWrite ? null : reason },
        canRevoke: { allowed: canWrite, reason: canWrite ? null : reason },
        nextActionKey: "REGISTER",
        nextActionLabel: "Register Name",
        nextActionReason: canWrite ? null : reason,
        countdownLabel: null,
        countdownBlocks: null,
        countdownHours: null,
      });
    }
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({
          walletProfileId: "p1",
          unlocked: signerUnlocked,
          unlockedUntilEpochMs: Date.now() + 60000,
        });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked,
          broadcasterAvailable: true,
          canWrite,
          reason: writeReason,
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
  it("states the reason once and offers no menu when the node can't write", async () => {
    invokeMock.mockImplementation(
      route(
        false,
        "Your local node is still syncing (40%). On-chain sends and transfers need a fully-synced node.",
      ),
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

    // A node that cannot write refuses every capability, so there is nothing
    // behind the advanced toggle and no toggle to open. The reason is already
    // stated twice — on the banner above and on the guided action — and a menu
    // of six buttons all carrying that same reason is the wall of dead
    // controls this modal no longer renders.
    expect(screen.queryByTestId("all-actions-toggle")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Reveal$/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Redeem$/i })).toBeNull();
    // No meaningful "Bid" action either, for a name whose auction is over.
    expect(screen.queryByRole("button", { name: /^Bid$/i })).toBeNull();
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    // Close stays available.
    expect(screen.getByRole("button", { name: /^Close$/i })).not.toBeDisabled();
  });

  it("states the locked-wallet reason once, with an Unlock button", async () => {
    // Regression: the write-capability reason was printed twice — once by the
    // modal-wide gate (which has an Unlock button) and again, bare, inside the
    // guided action panel. One notice, one button, and it must be actionable.
    const reason = "Unlock your wallet to sign transactions.";
    invokeMock.mockImplementation(route(false, "Needs a synced owner coin.", false, reason));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // Re-query on every tick: React swaps the banner's subtree once the
    // write-capability query resolves, so a node captured earlier goes stale.
    await waitFor(() =>
      expect(screen.getByTestId("name-actions-blocked")).toHaveTextContent(reason),
    );

    const occurrences = (document.body.textContent ?? "").split(reason).length - 1;
    expect(occurrences).toBe(1);
    expect(screen.getAllByTestId("unlock-now")).toHaveLength(1);
    expect(screen.getByTestId("name-actions-blocked")).toContainElement(
      screen.getByTestId("unlock-now"),
    );
  });

  it("enables actions once the node is write-capable", async () => {
    invokeMock.mockImplementation(route(true, null));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // No blocked banner.
    await waitForWritable();
    expect(screen.queryByTestId("name-actions-blocked")).toBeNull();
    // Register should be enabled since canWrite is true and canRegister is allowed.
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /^Register$/i })).not.toBeDisabled();
    });
  });
});

// Small helper: the write-capability query resolves async; wait for the
// blocked banner to disappear (i.e., canWrite=true has been applied).
async function waitForWritable() {
  const { waitFor } = await import("@testing-library/react");
  await waitFor(() => expect(screen.queryByTestId("name-actions-blocked")).toBeNull());
}
