import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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

import { NamePageAt } from "../../test/namePage";

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
        nameIsRegistered: false,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
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
      // Submitting a bid runs build → sign → broadcast (see `run()` in
      // the name page / useExecuteDraft). The default `null` fall-through
      // used to make `run()` dereference `draft.id` on null and leak an
      // unhandled rejection into the suite. Stub the whole pipeline so the
      // happy path resolves cleanly; the tests still only assert on the
      // `build_bid_draft` args, not on the broadcast result.
      case "build_bid_draft":
        return Promise.resolve({ id: "draft-1" });
      case "sign_tx_draft":
        return Promise.resolve({ id: "draft-1" });
      case "broadcast_tx_draft":
        return Promise.resolve({ txid: "abcdef0123456789" });
      case "delete_tx_draft":
        return Promise.resolve(undefined);
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
        <>{children}</>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("Name page — guided bid form validation (F4)", () => {
  it("shows the forfeit warning as soon as the bid form is active, before any input", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    const warning = await screen.findByTestId("bid-forfeit-warning");
    expect(warning).toHaveTextContent(/forfeited/i);
    expect(warning).toHaveTextContent(/\(0 HNS\)/);
  });

  it("updates the forfeit warning's lockup figure as the user types", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    const lockupInput = screen.getByLabelText(/Lockup \(HNS\)/i);
    fireEvent.change(lockupInput, { target: { value: "25" } });

    await waitFor(() => {
      expect(screen.getByTestId("bid-forfeit-warning")).toHaveTextContent(/\(25 HNS\)/);
    });
  });

  it("the forfeit warning shows the lockup as read, and no figure for refused text", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    const lockupInput = screen.getByLabelText(/Lockup \(HNS\)/i);
    const warning = () => screen.getByTestId("bid-forfeit-warning");

    fireEvent.change(lockupInput, { target: { value: "1e3" } });
    await waitFor(() => expect(warning()).toHaveTextContent("the entire lockup is forfeited."));
    expect(warning()).not.toHaveTextContent(/1e3/);

    fireEvent.change(lockupInput, { target: { value: "25.50" } });
    await waitFor(() => expect(warning()).toHaveTextContent(/\(25\.5 HNS\) is forfeited\./));
  });

  it("blocks submit and shows an error when bid exceeds lockup", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "20" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "10" } });

    await waitFor(() => {
      expect(screen.getByTestId("lockup-error")).toHaveTextContent(/lockup must be at least/i);
    });
    const bidButtons = screen.getAllByRole("button", { name: /^Bid$|Place bid/i });
    expect(bidButtons[0]).toBeDisabled();
  });

  // The amount fields are text fields (inputMode "decimal"), so whatever is
  // typed reaches the parser and gets its sentence; `validateBidInputs` and
  // `parseHnsAmount` are unit-tested in `src/lib`.

  it("says what is wrong with a bid that is not plain digits", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "abc" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "10" } });

    await waitFor(() => {
      expect(screen.getByTestId("bid-error")).toHaveTextContent(/in digits like 12\.5/);
    });
    expect(screen.getAllByRole("button", { name: /^Bid$|Place bid/i })[0]).toBeDisabled();
  });

  it("refuses a 7th decimal in the bid instead of rounding it, and builds nothing", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), {
      target: { value: "10.0000005" },
    });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "12" } });

    await waitFor(() => {
      expect(screen.getByTestId("bid-error")).toHaveTextContent("HNS has at most 6 decimals");
    });
    const submit = screen.getAllByRole("button", { name: /^Bid$|Place bid/i })[0]!;
    expect(submit).toBeDisabled();
    fireEvent.click(submit);
    expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("build_bid_draft");
  });

  it("refuses a 7th decimal in the lockup instead of rounding it", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "10" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), {
      target: { value: "12.0000001" },
    });

    await waitFor(() => {
      expect(screen.getByTestId("lockup-error")).toHaveTextContent("HNS has at most 6 decimals");
    });
    expect(screen.getAllByRole("button", { name: /^Bid$|Place bid/i })[0]).toBeDisabled();
  });

  it("bids 1234.567891 HNS as exactly 1,234,567,891 doos", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), {
      target: { value: "1234.567891" },
    });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), {
      target: { value: "2000.000001" },
    });
    fireEvent.click(await screen.findByRole("button", { name: /Placing bid…|^Bid$/i }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "build_bid_draft");
      expect(call?.[1]).toMatchObject({
        name: "bidname",
        bidValue: 1_234_567_891,
        lockup: 2_000_000_001,
      });
    });
  });

  it("blocks submit and shows an error when the bid is zero", async () => {
    invokeMock.mockImplementation(route());
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

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
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

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
    render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });

    await screen.findByText("Place a Bid");
    fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "10" } });
    fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "12" } });

    const submitBtn = await screen.findByRole("button", { name: /Placing bid…|^Bid$/i });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "build_bid_draft");
      expect(call?.[1]).toMatchObject({
        name: "bidname",
        bidValue: 10_000_000,
        lockup: 12_000_000,
      });
    });
  });
});

// The "advanced bid form" — a second copy of the bid inputs behind the
// all-actions toggle — was removed in db0bad1, which de-duplicated the form and
// hid the toggle in BIDDING. The two tests that lived here pinned that removed
// UI and had been failing since; `BidForm`'s `variant: "advanced"` is now
// unreachable, so nothing renders `bid-forfeit-warning-advanced` or
// `lockup-error-advanced`. The guided describe above already covers the same
// validation: the forfeit warning, and the bid > lockup block.
