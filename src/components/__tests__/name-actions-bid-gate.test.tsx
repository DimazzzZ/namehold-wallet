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

/**
 * Regression (bid de-duplication): the bid + lockup inputs live in exactly
 * ONE place — the guided panel. The advanced auction section no longer
 * renders a second bid form (it used to, which duplicated the guided form
 * one-for-one during BIDDING and implied Open would also bid during OPENING).
 * So: during OPENING the advanced section exposes only Open/Reveal/Redeem and
 * NO bid inputs; during BIDDING the guided panel renders the sole bid form.
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
        nameIsRegistered: false,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
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
  it("offers no advanced section at all during OPENING, so no bid can be invited", async () => {
    invokeMock.mockImplementation(route("OPENING", false));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });
    await screen.findByTestId("name-phase");

    // The auction is already open, so the manual Open fallback is refused and
    // nothing else applies to a name this wallet does not own. A toggle onto a
    // disabled "Open" is the dead control the section states exist to remove —
    // which makes "no Bid / Lockup input here" unconditional.
    expect(screen.queryByTestId("all-actions-toggle")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Lockup (HNS)")).not.toBeInTheDocument();
    expect(screen.queryByTestId("bid-gate-placeholder")).not.toBeInTheDocument();
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

  it("hides the Show-all-actions toggle in BIDDING when every advanced action is disabled", async () => {
    // Already bid: canBid + Open/Reveal/Redeem are all caps-disabled, so the
    // advanced section would only reveal an all-disabled menu — suppress it.
    invokeMock.mockImplementation(route("BIDDING", false));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    // Wait for capabilities to resolve so the phase-only fallback can't leave
    // the toggle transiently visible.
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_name_action_capabilities", expect.anything());
    });
    await waitFor(() => {
      expect(screen.queryByTestId("all-actions-toggle")).not.toBeInTheDocument();
    });
  });
});
