import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// F4 fix: client-side bid validation (0 < bid ≤ lockup, NaN guard) plus an
// always-visible forfeit warning, for both the guided BIDDING form and the
// duplicated "advanced" auction section form.

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
  network: "regtest",
  receiveAddress: "rs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function route() {
  return (cmd: string) => {
    if (cmd === "get_name_action_capabilities") {
      return Promise.resolve({
        name: "bidname",
        phase: "BIDDING",
        taskState: "readyToBid",
        ownsName: false,
        hasBidCommitment: false,
        hasRevealCoin: false,
        hasOwnerCoin: false,
        canOpen: { allowed: false, reason: "Phase is BIDDING" },
        canBid: { allowed: true, reason: null },
        canReveal: { allowed: false, reason: null },
        canRedeem: { allowed: false, reason: null },
        canRegister: { allowed: false, reason: null },
        canUpdate: { allowed: false, reason: null },
        canTransfer: { allowed: false, reason: null },
        canFinalize: { allowed: false, reason: null },
        canCancelTransfer: { allowed: false, reason: null },
        canRenew: { allowed: false, reason: null },
        canRevoke: { allowed: false, reason: null },
        nextActionKey: "BID",
        nextActionLabel: "Bid",
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
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
      case "get_write_capability":
        return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
      case "read_name_info":
        return Promise.resolve({
          name: "bidname",
          state: "BIDDING",
          registered: false,
          height: 100,
          renewal: 200,
          owner: null,
          value: null,
          highest: 5_000_000,
          stats: { blocksUntilReveal: 50, hoursUntilReveal: 8 },
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

describe("NameActionsModal — guided bid form validation (F4)", () => {
  it("shows the forfeit warning as soon as the bid form is active, before any input", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    const warning = await screen.findByTestId("bid-forfeit-warning");
    expect(warning).toHaveTextContent(/forfeited/i);
    expect(warning).toHaveTextContent(/\(0 HNS\)/);
  });

  it("updates the forfeit warning's lockup figure as the user types", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    const lockupInput = screen.getByLabelText(/Lockup \(HNS\)/i);
    fireEvent.change(lockupInput, { target: { value: "25" } });

    await waitFor(() => {
      expect(screen.getByTestId("bid-forfeit-warning")).toHaveTextContent(/\(25 HNS\)/);
    });
  });

  it("blocks submit and shows an error when bid exceeds lockup", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "20" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "10" } });

    await waitFor(() => {
      expect(screen.getByTestId("lockup-error")).toHaveTextContent(/lockup must be at least/i);
    });
    const bidButtons = screen.getAllByRole("button", { name: /^Bid$|Place bid/i });
    expect(bidButtons[0]).toBeDisabled();
  });

  // Note: an `<input type="number">` sanitizes non-numeric text to "" at the
  // DOM level (both real browsers and jsdom), so a literal "abc" never
  // reaches React state via a simulated change event — the NaN/non-numeric
  // guard itself is unit-tested directly against `validateBidInputs` in
  // `src/lib/auction.test.ts`. Here we exercise the DOM-reachable invalid
  // cases: empty, zero, negative, and bid > lockup.

  it("blocks submit and shows an error when the bid is zero", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "0" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "10" } });

    await waitFor(() => {
      expect(screen.getByTestId("bid-error")).toHaveTextContent(/greater than 0/i);
    });
    const bidButtons = screen.getAllByRole("button", { name: /^Bid$|Place bid/i });
    expect(bidButtons[0]).toBeDisabled();
  });

  it("enables submit once bid and lockup are both valid and bid <= lockup", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "10" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "12" } });

    await waitFor(() => {
      expect(screen.queryByTestId("bid-error")).toBeNull();
      expect(screen.queryByTestId("lockup-error")).toBeNull();
    });
    const submitBtn = screen.getByRole("button", { name: /Place a Bid|^Bid$/i });
    expect(submitBtn).not.toBeDisabled();
  });

  it("placing a valid bid sends the exact numeric doos values to the backend", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "10" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "12" } });

    const submitBtn = await screen.findByRole("button", { name: /Placing bid…|^Bid$/i });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "build_bid_draft");
      expect(call?.[1]).toMatchObject({ name: "bidname", bidValue: 10_000_000, lockup: 12_000_000 });
    });
  });
});

describe("NameActionsModal — advanced bid form validation (F4, duplicated form)", () => {
  async function openAdvanced() {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });
    await screen.findByText("Place a Bid");
    fireEvent.click(await screen.findByTestId("all-actions-toggle"));
    await screen.findByTestId("bid-forfeit-warning-advanced");
  }

  it("shows the forfeit warning in the advanced form too", async () => {
    await openAdvanced();
    expect(screen.getByTestId("bid-forfeit-warning-advanced")).toHaveTextContent(/forfeited/i);
  });

  it("blocks the advanced Bid button when bid > lockup", async () => {
    await openAdvanced();
    const bidInputs = screen.getAllByLabelText(/^Bid \(HNS\)/i);
    const lockupInputs = screen.getAllByLabelText(/Lockup \(HNS\)/i);
    // The advanced form's inputs are the second occurrence (guided form first).
    fireEvent.change(bidInputs[bidInputs.length - 1]!, { target: { value: "50" } });
    fireEvent.change(lockupInputs[lockupInputs.length - 1]!, { target: { value: "5" } });

    await waitFor(() => {
      expect(screen.getByTestId("lockup-error-advanced")).toHaveTextContent(/lockup must be at least/i);
    });
  });
});
