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

/**
 * Regression (BidGate): the advanced auction section rendered the Bid /
 * Lockup inputs in every phase, so during OPENING they sat right next to the
 * "Open" button — implying Open would also submit a bid. The gate hides the
 * inputs unless `canBid.allowed`, showing a countdown-aware placeholder
 * instead. In BIDDING (canBid allowed) the inputs come back.
 */

const profile = {
  id: "p1",
  label: "Primary",
  network: "regtest",
  receiveAddress: "hs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

/** Route factory keyed on auction phase + whether bidding is allowed. */
function route(phase: "OPENING" | "BIDDING", canBidAllowed: boolean) {
  return (cmd: string) => {
    if (cmd === "get_name_action_capabilities") {
      return Promise.resolve({
        name: "examplename",
        phase,
        taskState: "none",
        ownsName: false,
        hasBidCommitment: canBidAllowed ? false : phase === "BIDDING",
        hasBidCoin: false,
        hasRevealCoin: false,
        hasOwnerCoin: false,
        revealTxid: null,
        bidValueDoos: null,
        canOpen: { allowed: false, reason: null },
        canBid: {
          allowed: canBidAllowed,
          reason: canBidAllowed ? null : phase === "OPENING" ? "Auction is opening" : "Already bid",
        },
        canReveal: { allowed: false, reason: null },
        canRedeem: { allowed: false, reason: null },
        canRegister: { allowed: false, reason: null },
        canUpdate: { allowed: false, reason: null },
        canTransfer: { allowed: false, reason: null },
        canFinalize: { allowed: false, reason: null },
        canCancelTransfer: { allowed: false, reason: null },
        canRenew: { allowed: false, reason: null },
        canRevoke: { allowed: false, reason: null },
        nextActionKey: null,
        nextActionLabel: null,
        nextActionReason: null,
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
      case "read_name_info":
        return Promise.resolve({
          name: "examplename",
          state: phase,
          registered: false,
          height: 100,
          renewal: null,
          owner: null,
          value: null,
          highest: null,
          // OPENING → blocksUntilBidding drives "Bidding opens in 12 blocks".
          // BIDDING → blocksUntilReveal (not asserted here).
          stats: {
            blocksUntilBidding: 12,
            hoursUntilBidding: 2,
            blocksUntilReveal: 6,
            hoursUntilReveal: 1,
          },
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

describe("NameActionsModal — BidGate hides inputs off the bidding phase", () => {
  it("hides Bid/Lockup inputs during OPENING and shows a countdown placeholder", async () => {
    invokeMock.mockImplementation(route("OPENING", false));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    // Open the advanced section where the bid inputs historically lived.
    const toggle = await screen.findByTestId("all-actions-toggle");
    fireEvent.click(toggle);

    // The placeholder appears instead of the inputs …
    const placeholder = await screen.findByTestId("bid-gate-placeholder");
    expect(placeholder).toHaveTextContent(/Bidding opens in 12 blocks \(~2h\)/i);
    // … and there is NO Bid / Lockup input to invite a bid next to "Open".
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Lockup (HNS)")).not.toBeInTheDocument();
  });

  it("shows the Bid/Lockup inputs during BIDDING when canBid is allowed", async () => {
    invokeMock.mockImplementation(route("BIDDING", true));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    // The guided BIDDING panel renders the inputs directly (no toggle needed).
    await waitFor(() => {
      expect(screen.getAllByLabelText("Bid (HNS)").length).toBeGreaterThan(0);
    });
    expect(screen.queryByTestId("bid-gate-placeholder")).not.toBeInTheDocument();
  });
});
