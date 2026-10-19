import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// Tests for the simplified name-acquisition flow:
//   * WalletView shows an "Auctions" link in the sidebar
//   * AuctionsView renders input and opens the modal
//   * NameActionsModal shows guided phase-based UI for AVAILABLE names
//   * NameActionsModal shows guided UI for BIDDING/REVEAL/CLOSED names
//   * Write-capability gating is preserved

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";
import { AuctionsView } from "../AuctionsView";
import { NameActionsModal } from "../NameActionsModal";
import { makeCapabilities, refused } from "../../test/fixtures/capabilities";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE",
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

function wrapper(initialEntries = ["/wallet"]) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={initialEntries}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("WalletView — Auctions link", () => {
  function routeWallet() {
    return (cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
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
        case "get_wallet_balances":
          return Promise.resolve({
            liquidDoos: 5_000_000,
            nameControlDoos: 0,
            nameLockupDoos: 0,
            immatureDoos: 0,
            immatureInBlocks: null,
            totalDoos: 5_000_000,
          });
        case "read_balance":
          return Promise.resolve({
            confirmed: 0,
            unconfirmed: 0,
            locked_confirmed: 0,
            locked_unconfirmed: 0,
          });
        case "list_tx_drafts":
          return Promise.resolve([]);
        case "read_names":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    };
  }

  it("renders the Auctions link in the sidebar", async () => {
    invokeMock.mockImplementation(routeWallet());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    expect(screen.getByText("Auctions")).toBeInTheDocument();
  });
});

describe("AuctionsView — name lookup and modal", () => {
  function routeAuctions() {
    return (cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
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
        default:
          return Promise.resolve(null);
      }
    };
  }

  it("renders the page header and input", async () => {
    invokeMock.mockImplementation(routeAuctions());
    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });

    expect(await screen.findByText("Auctions")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("example")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Look up/i })).toBeInTheDocument();
  });

  it("Look up button is disabled when input is empty", async () => {
    invokeMock.mockImplementation(routeAuctions());
    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });
    await screen.findByText("Auctions");

    const btn = screen.getByRole("button", { name: /Look up/i });
    expect(btn).toBeDisabled();
  });

  it("enables Look up after typing a name and opens the modal on click", async () => {
    invokeMock.mockImplementation(routeAuctions());
    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });
    await screen.findByText("Auctions");

    const input = screen.getByPlaceholderText("example");
    fireEvent.change(input, { target: { value: "myname" } });

    const btn = screen.getByRole("button", { name: /Look up/i });
    expect(btn).not.toBeDisabled();

    fireEvent.click(btn);

    // Modal should open — it will show the name in the header
    await waitFor(() => {
      expect(screen.getByText(/\.myname/)).toBeInTheDocument();
    });
  });

  it("Enter key in the input opens the modal", async () => {
    invokeMock.mockImplementation(routeAuctions());
    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });
    await screen.findByText("Auctions");

    const input = screen.getByPlaceholderText("example");
    fireEvent.change(input, { target: { value: "coolname" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText(/\.coolname/)).toBeInTheDocument();
    });
  });

  it("keeps typed characters verbatim during typing; normalizes only at submit", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
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
          return Promise.resolve({ name: "helloworld123", state: "AVAILABLE" });
        default:
          return Promise.resolve(null);
      }
    });
    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });
    await screen.findByText("Auctions");

    const input = screen.getByPlaceholderText("example") as HTMLInputElement;
    // During typing: no live cleanup — the input echoes the DOM value verbatim.
    fireEvent.change(input, { target: { value: "Hello World! 123" } });
    expect(input.value).toBe("Hello World! 123");

    // At submit: the pipeline lowercases, strips invalid chars, and would encode
    // to ACE (this input is ASCII so encoding is a no-op). Modal opens for the
    // normalized name.
    fireEvent.click(screen.getByRole("button", { name: /Look up/i }));
    await waitFor(() => {
      expect(screen.getByText(/\.helloworld123/)).toBeInTheDocument();
    });
    expect(invokeMock).toHaveBeenCalledWith("read_name_info", { name: "helloworld123" });
  });

  it("shows Unicode verbatim during typing; encodes to ACE only on Look up", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
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
          return Promise.resolve({ name: "xn--90ai7ab", state: "AVAILABLE" });
        default:
          return Promise.resolve(null);
      }
    });

    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });
    await screen.findByText("Auctions");

    const input = screen.getByPlaceholderText("example") as HTMLInputElement;
    // During typing: the input shows the raw Unicode the user typed (NOT the
    // punycode form — encoding on keystroke was the previous regression).
    fireEvent.change(input, { target: { value: "сбер" } });
    expect(input.value).toBe("сбер");
    expect(input.value).not.toMatch(/^xn--/);

    // Look up encodes and opens the modal with the ACE form.
    const btn = screen.getByRole("button", { name: /Look up/i });
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);
    await waitFor(() => {
      expect(screen.getByText(/\.xn--90ai7ab/)).toBeInTheDocument();
    });
    expect(invokeMock).toHaveBeenCalledWith("read_name_info", { name: "xn--90ai7ab" });
  });

  it("allows typing a Cyrillic word character-by-character without re-encoding mid-type", async () => {
    // Red-capable regression test for the reported bug: with the input controlled
    // by React state, if the onChange handler ACE-encodes on every keystroke, then
    // after typing `с` the state becomes "xn--q1a", the input shows "xn--q1a",
    // and the next keystroke appends to that punycode prefix instead of extending
    // "с". This test simulates sequential typing and asserts the input preserves
    // exactly what the user typed at every intermediate step.
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
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
          return Promise.resolve({ name: "xn--90ai7ab", state: "AVAILABLE" });
        default:
          return Promise.resolve(null);
      }
    });

    render(<AuctionsView />, { wrapper: wrapper(["/auctions"]) });
    await screen.findByText("Auctions");

    const input = screen.getByPlaceholderText("example") as HTMLInputElement;

    // Type each character sequentially. On a controlled input, the new value the
    // browser sends is `<current value> + <new char>`, so we always append to
    // whatever `input.value` currently is — which is precisely how the on-keystroke
    // encoding regression manifests.
    const word = "сбер";
    let expected = "";
    for (const ch of word) {
      expected += ch;
      fireEvent.change(input, { target: { value: input.value + ch } });
      expect(input.value).toBe(expected);
      expect(input.value).not.toMatch(/^xn--/);
    }

    // Sanity: after typing the whole word, the input still shows the Unicode.
    expect(input.value).toBe("сбер");

    // And Look up still encodes correctly to the ACE form.
    fireEvent.click(screen.getByRole("button", { name: /Look up/i }));
    await waitFor(() => {
      expect(screen.getByText(/\.xn--90ai7ab/)).toBeInTheDocument();
    });
    expect(invokeMock).toHaveBeenCalledWith("read_name_info", { name: "xn--90ai7ab" });
  });
});

describe("NameActionsModal — guided acquisition flow", () => {
  function routeModal(
    nameInfo: Record<string, unknown> | null,
    overrides: Record<string, unknown> = {},
  ) {
    return (cmd: string) => {
      if (cmd === "get_name_action_capabilities") {
        return Promise.resolve(overrides.capabilities ?? null);
      }
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
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
          return Promise.resolve(nameInfo);
        default:
          return Promise.resolve(null);
      }
    };
  }

  it("shows Open Auction for an AVAILABLE name", async () => {
    invokeMock.mockImplementation(
      routeModal({
        name: "newname",
        state: "AVAILABLE",
        height: null,
        renewal: null,
        owner: null,
        value: null,
        highest: null,
        stats: null,
      }),
    );
    render(<NameActionsModal name="newname" open onClose={() => {}} />, { wrapper: wrapper() });

    // Should show the guided phase header
    expect(await screen.findByText("Open Auction")).toBeInTheDocument();
    expect(screen.getByText("Open")).toBeInTheDocument();
  });

  /// Regression: while our own OPEN sits in the mempool the chain still calls
  /// the name AVAILABLE, so the panel rendered the AVAILABLE copy — "Start a
  /// Vickrey auction" plus an Open button disabled with "an auction is already
  /// opening" — underneath a heading that already read "Wait for Bidding". It
  /// told the user to do the thing they had just done.
  it("says the OPEN is waiting to be mined, with no Open button", async () => {
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "pendingopen",
          state: "AVAILABLE",
          height: null,
          renewal: null,
          owner: null,
          value: null,
          highest: null,
          stats: null,
        },
        {
          capabilities: {
            name: "pendingopen",
            phase: "AVAILABLE",
            taskState: "waitingForBidding",
            ownsName: false,
            nameIsRegistered: false,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            hasBidCommitment: false,
            hasBidCoin: false,
            hasRevealCoin: false,
            hasOwnerCoin: false,
            canOpen: {
              allowed: false,
              reason: "an auction is already opening for this name (pending confirmation)",
            },
            canBid: { allowed: false, reason: "bidding is not open (phase: 'AVAILABLE')" },
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
            nextActionLabel: "Wait for Bidding",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
            auctionBiddingBlocks: 5,
            auctionRevealBlocks: 10,
            pendingBroadcastAction: "open",
          },
        },
      ),
    );
    render(<NameActionsModal name="pendingopen" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    const panel = await screen.findByTestId("action-pending-block");
    expect(panel).toHaveTextContent(/Open is broadcast and waiting to be mined/i);
    // The window still helps: it says what happens once it lands.
    expect(panel).toHaveTextContent("Bidding runs 5 blocks, then 10 blocks to reveal.");

    // None of the AVAILABLE copy, and no button inviting a second OPEN.
    expect(screen.queryByText(/Start a Vickrey auction/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /^Open$/ })).toBeNull();
    expect(screen.queryByText(/already opening for this name/i)).toBeNull();
  });

  /// Scoping the bids panel to the current auction (029/030) correctly stopped
  /// a lapsed auction's bid from being counted — but it also removed the only
  /// trace of it. The lockup is unrecoverable, so the wallet has to say where
  /// the money went rather than let the coin vanish from the UI.
  it("names the lockup stranded in an earlier auction of the same name", async () => {
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "reopened",
          state: "BIDDING",
          height: 779,
          renewal: null,
          owner: null,
          value: null,
          highest: null,
          stats: { blocksUntilReveal: 4, hoursUntilReveal: 1 },
        },
        {
          capabilities: {
            name: "reopened",
            phase: "BIDDING",
            taskState: "readyToBid",
            ownsName: false,
            nameIsRegistered: false,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            hasBidCommitment: false,
            hasBidCoin: false,
            hasRevealCoin: false,
            hasOwnerCoin: false,
            canOpen: { allowed: false, reason: null },
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
            nextActionLabel: "Place Bid",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
            strandedBidCount: 1,
            strandedLockupDoos: 100_000_000,
          },
        },
      ),
    );
    render(<NameActionsModal name="reopened" open onClose={() => {}} />, { wrapper: wrapper() });

    const notice = await screen.findByTestId("stranded-bids");
    expect(notice).toHaveTextContent("1 bid of yours from an auction that is over");
    expect(notice).toHaveTextContent("100 HNS is still locked");
    expect(notice).toHaveTextContent(/cannot be recovered/i);
  });

  it("names the lockup stranded on an expired name with no later auction", async () => {
    // Nobody revealed, so the name expired as the auction closed and reads
    // Available again: there is no later auction, and the stranded coin is a
    // bid never revealed, not a losing reveal.
    invokeMock.mockImplementation(
      routeModal(
        { name: "lapsedname", state: "AVAILABLE", height: null, stats: null },
        {
          capabilities: makeCapabilities({
            name: "lapsedname",
            phase: "AVAILABLE",
            taskState: "availableToOpen",
            canOpen: { allowed: true, reason: null },
            nextActionKey: "OPEN",
            nextActionLabel: "Open Auction",
            strandedBidCount: 2,
            strandedLockupDoos: 3_000_000,
          }),
        },
      ),
    );
    render(<NameActionsModal name="lapsedname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    const notice = await screen.findByTestId("stranded-bids");
    expect(notice).toHaveTextContent("2 bids of yours from auctions that are over");
    expect(notice).toHaveTextContent("3 HNS is still locked in them");
    expect(notice).not.toHaveTextContent(/earlier auction/i);
    expect(notice).not.toHaveTextContent(/losing reveal/i);
    expect(notice).toHaveTextContent(/cannot be recovered/i);
  });

  it("says the bids could not be checked when no node confirmed the stranded candidates", async () => {
    // Offline on an expired name: our bid of the lapsed auction is a candidate
    // no node could be asked about. Not a loss verdict, and not "nothing
    // stranded" either.
    invokeMock.mockImplementation(
      routeModal(
        { name: "offlinelapsed", state: "AVAILABLE", height: null, stats: null },
        {
          capabilities: makeCapabilities({
            name: "offlinelapsed",
            phase: "AVAILABLE",
            taskState: "availableToOpen",
            canOpen: { allowed: true, reason: null },
            nextActionKey: "OPEN",
            nextActionLabel: "Open Auction",
            strandedUncheckedCount: 1,
          }),
        },
      ),
    );
    render(<NameActionsModal name="offlinelapsed" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    const line = await screen.findByTestId("stranded-unchecked");
    expect(line).toHaveTextContent(
      "Some of your bids on this name could not be checked — sync with your node to see whether a lockup was lost.",
    );
    expect(screen.queryByTestId("stranded-bids")).toBeNull();
    expect(line).not.toHaveTextContent(/cannot be recovered/i);
  });

  it("says nothing about unchecked bids when every candidate was checked", async () => {
    invokeMock.mockImplementation(
      routeModal(
        { name: "checked", state: "AVAILABLE", height: null, stats: null },
        {
          capabilities: makeCapabilities({
            name: "checked",
            phase: "AVAILABLE",
            taskState: "availableToOpen",
            canOpen: { allowed: true, reason: null },
            nextActionKey: "OPEN",
            nextActionLabel: "Open Auction",
          }),
        },
      ),
    );
    render(<NameActionsModal name="checked" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByText("Open Auction")).toBeInTheDocument();
    expect(screen.queryByTestId("stranded-unchecked")).toBeNull();
  });

  it("says why our losing reveals have no redeem in the last block before expiry", async () => {
    // One block of the name left: hsd takes no redeem in the block it expires
    // in, so the backend reports no task, with the reason. No "Redeem" urgency
    // and no "nothing to do here" either.
    const reason =
      "Your losing bids on this name can no longer be redeemed: the name expires with the next block, and hsd takes no redeem of it from then on.";
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "lastblock",
          state: "CLOSED",
          height: 100,
          stats: { blocksUntilExpire: 1, daysUntilExpire: 0.01 },
        },
        {
          capabilities: makeCapabilities({
            name: "lastblock",
            phase: "CLOSED",
            taskState: "unavailableOther",
            hasRevealCoin: true,
            redeemableRevealCount: 1,
            redeemableValueDoos: 2_000_000,
            canRedeem: refused(
              "the name expires with the next block, and hsd takes no redeem of it from then on",
            ),
            nextActionReason: reason,
          }),
        },
      ),
    );
    render(<NameActionsModal name="lastblock" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByTestId("no-task-reason")).toHaveTextContent(reason);
    expect(screen.queryByText(/No auction actions are needed/)).toBeNull();
    expect(screen.queryByTestId("guided-redeem")).toBeNull();
  });

  it("shows Bid for a BIDDING name", async () => {
    invokeMock.mockImplementation(
      routeModal({
        name: "bidname",
        state: "BIDDING",
        height: 100,
        renewal: 200,
        owner: null,
        value: null,
        highest: 5_000_000,
        stats: { blocksUntilReveal: 50, hoursUntilReveal: 8 },
      }),
    );
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByText("Place a Bid")).toBeInTheDocument();
    expect(screen.getByText("Bid")).toBeInTheDocument();
  });

  // Regression: a BIDDING name this wallet has already bid on still invites
  // another bid. Several independent bids per name are allowed, so the guided
  // panel must keep the bid form rather than collapsing to the wait-for-reveal
  // panel it used to show once one bid was placed.
  it("still offers the bid form for a BIDDING name this wallet has already bid on", async () => {
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "bidname",
          state: "BIDDING",
          height: 100,
          renewal: 200,
          owner: null,
          value: null,
          highest: 5_000_000,
          // No reveal-countdown stats: nextTransition() returns null, so the
          // guided panel renders its no-countdown branch — the branch whose
          // copy must match the backend next_action_reason verbatim. (The
          // countdown branch appends "Reveal opens in …" and is covered
          // implicitly elsewhere.)
          stats: {},
        },
        {
          capabilities: {
            name: "bidname",
            phase: "BIDDING",
            taskState: "readyToBid",
            ownsName: false,
            nameIsRegistered: false,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            hasBidCommitment: true,
            myBidCount: 1,
            hasRevealCoin: false,
            hasOwnerCoin: false,
            canOpen: { allowed: false, reason: null },
            canBid: { allowed: true, reason: null },
            canReveal: { allowed: false, reason: "Reveal not open yet" },
            canRedeem: { allowed: false, reason: null },
            canRegister: { allowed: false, reason: "Phase is BIDDING" },
            canUpdate: { allowed: false, reason: null },
            canTransfer: { allowed: false, reason: null },
            canFinalize: { allowed: false, reason: null },
            nextActionKey: "BID",
            nextActionLabel: "Place a Bid",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
          },
        },
      ),
    );
    render(<NameActionsModal name="bidname" open onClose={() => {}} />, { wrapper: wrapper() });

    // The badge matches the on-chain phase.
    expect(await screen.findByText("Bidding")).toBeInTheDocument();
    expect(screen.queryByText("Waiting for Bidding")).not.toBeInTheDocument();
    // The bid form is still offered, holding an existing bid notwithstanding.
    expect(screen.getAllByLabelText("Bid (HNS)").length).toBeGreaterThan(0);
    expect(screen.getAllByLabelText("Lockup (HNS)").length).toBeGreaterThan(0);
  });

  it("shows Reveal for a REVEAL name", async () => {
    invokeMock.mockImplementation(
      routeModal({
        name: "revealname",
        state: "REVEAL",
        height: 100,
        renewal: 200,
        owner: { hash: "abc", index: 0 },
        value: null,
        highest: 5_000_000,
        stats: { blocksUntilClose: 20, hoursUntilClose: 4 },
      }),
    );
    render(<NameActionsModal name="revealname" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByText("Reveal Your Bid")).toBeInTheDocument();
    expect(screen.getAllByText("Reveal").length).toBeGreaterThanOrEqual(1);
  });

  it("shows Register for a CLOSED name (wonNeedsRegister task)", async () => {
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "closedname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: profile.receiveAddress, index: 0 },
          value: 1_000_000,
          highest: 2_000_000,
          stats: { blocksUntilExpire: 100 },
        },
        {
          capabilities: {
            name: "closedname",
            phase: "CLOSED",
            taskState: "wonNeedsRegister",
            ownsName: true,
            nameIsRegistered: false,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            hasBidCommitment: false,
            hasRevealCoin: false,
            hasOwnerCoin: true,
            canOpen: { allowed: false, reason: null },
            canBid: { allowed: false, reason: "Phase is CLOSED" },
            canReveal: { allowed: false, reason: "No commitment" },
            canRedeem: { allowed: false, reason: null },
            canRegister: { allowed: true, reason: null },
            canUpdate: { allowed: true, reason: null },
            canTransfer: { allowed: true, reason: null },
            canFinalize: { allowed: false, reason: null },
            canCancelTransfer: { allowed: false, reason: null },
            canRenew: { allowed: true, reason: null },
            canRevoke: { allowed: true, reason: null },
            nextActionKey: "REGISTER",
            nextActionLabel: "Register Name",
            nextActionReason: "You won the auction! Register the name to finalize ownership.",
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
          },
        },
      ),
    );
    render(<NameActionsModal name="closedname" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByText("Register Name")).toBeInTheDocument();
    expect(screen.getByText("Register")).toBeInTheDocument();
  });

  it("shows owner-manage for CLOSED name when wallet owns it and it is already registered", async () => {
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "ownedname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: profile.receiveAddress, index: 0 },
          registered: true,
          value: 1_000_000,
          highest: 2_000_000,
          stats: { blocksUntilExpire: 100 },
        },
        {
          capabilities: {
            name: "ownedname",
            phase: "CLOSED",
            taskState: "ownedNoUrgentAction",
            ownsName: true,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            nameIsRegistered: true,
            hasBidCommitment: false,
            hasRevealCoin: false,
            hasOwnerCoin: true,
            canOpen: { allowed: false, reason: null },
            canBid: { allowed: false, reason: "Phase is CLOSED" },
            canReveal: { allowed: false, reason: "No commitment" },
            canRedeem: { allowed: false, reason: null },
            canRegister: { allowed: true, reason: null },
            canUpdate: { allowed: true, reason: null },
            canTransfer: { allowed: true, reason: null },
            canFinalize: { allowed: false, reason: null },
            canCancelTransfer: { allowed: false, reason: null },
            canRenew: { allowed: true, reason: null },
            canRevoke: { allowed: true, reason: null },
            nextActionKey: null,
            nextActionLabel: "Manage",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
          },
        },
      ),
    );
    render(<NameActionsModal name="ownedname" open onClose={() => {}} />, { wrapper: wrapper() });

    // Should show the guided "You own this name" messaging
    expect(await screen.findByText(/You own this name/i)).toBeInTheDocument();
    // Management is auto-expanded for owned names with no urgent task — the
    // toggle already reads "Hide advanced actions" and Transfer/Renew are visible
    // without any click.
    expect(screen.getByText("Hide advanced actions")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Transfer$/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Renew$/i })).toBeInTheDocument();
  });

  it("shows a not-synced-locally banner for an owned CLOSED name with no local owner coin", async () => {
    const notSyncedReason = "owner coin not synced locally — connect a node and Refresh to manage";
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "unsyncedname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: profile.receiveAddress, index: 0 },
          registered: true,
          value: 1_000_000,
          highest: 2_000_000,
          stats: { blocksUntilExpire: 100 },
        },
        {
          capabilities: {
            name: "unsyncedname",
            phase: "CLOSED",
            taskState: "ownedNoUrgentAction",
            ownsName: true,
            nameIsRegistered: true,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            hasBidCommitment: false,
            hasRevealCoin: false,
            hasOwnerCoin: false,
            canOpen: { allowed: false, reason: null },
            canBid: { allowed: false, reason: "Phase is CLOSED" },
            canReveal: { allowed: false, reason: "No commitment" },
            canRedeem: { allowed: false, reason: null },
            canRegister: { allowed: false, reason: notSyncedReason },
            canUpdate: { allowed: false, reason: notSyncedReason },
            canTransfer: { allowed: false, reason: notSyncedReason },
            canFinalize: { allowed: false, reason: notSyncedReason },
            canCancelTransfer: { allowed: false, reason: notSyncedReason },
            canRenew: { allowed: false, reason: notSyncedReason },
            canRevoke: { allowed: false, reason: notSyncedReason },
            nextActionKey: null,
            nextActionLabel: "Manage",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
          },
        },
      ),
    );
    render(<NameActionsModal name="unsyncedname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    // Still gets the "you own this name" owned-manage framing…
    expect(await screen.findByText(/You own this name/i)).toBeInTheDocument();
    // …plus a clear reason why the owner actions aren't usable yet.
    const banner = screen.getByTestId("owner-coin-not-synced");
    expect(banner).toHaveTextContent(/hasn't synced locally/i);
    expect(banner).toHaveTextContent(notSyncedReason);

    // Must NOT fall through to the stale "third-party owned" static block.
    expect(
      screen.queryByText(/This name is already registered\. No auction actions are needed/i),
    ).not.toBeInTheDocument();
  });

  it("says a bid never revealed lost its lockup, not that the name is someone else's", async () => {
    const reason =
      "You did not reveal this bid in time; its lockup is lost and cannot be redeemed.";
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "unrevealedname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: "ab".repeat(32), index: 0 },
          registered: false,
          value: 800_000,
          highest: 800_000,
          stats: { blocksUntilExpire: 100 },
        },
        {
          capabilities: makeCapabilities({
            name: "unrevealedname",
            phase: "CLOSED",
            taskState: "lostUnrevealed",
            ownsName: false,
            hasBidCommitment: true,
            hasBidCoin: true,
            canRedeem: refused("no unspent reveal coin to redeem"),
            nextActionKey: null,
            nextActionLabel: "Lost — Not Revealed",
            nextActionReason: reason,
          }),
        },
      ),
    );
    render(<NameActionsModal name="unrevealedname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    expect(await screen.findByTestId("lost-unrevealed")).toHaveTextContent(reason);
    expect(
      screen.queryByText(/This name is already registered\. No auction actions are needed/i),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Redeem$/i })).not.toBeInTheDocument();
  });

  it("says a closed auction's bid is not settled yet offline, not that the name is owned or someone else's", async () => {
    const reason =
      "This auction closed with a bid of yours the wallet cannot settle yet. Sync with your node, then Refresh to see whether it was revealed in time.";
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "unsettledname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: "ab".repeat(32), index: 0 },
          registered: false,
          value: 800_000,
          highest: 800_000,
          stats: { blocksUntilExpire: 100 },
        },
        {
          capabilities: makeCapabilities({
            name: "unsettledname",
            phase: "CLOSED",
            taskState: "closedResultUnknown",
            ownsName: false,
            hasBidCommitment: true,
            hasBidCoin: true,
            canRedeem: refused("no unspent reveal coin to redeem"),
            nextActionKey: null,
            nextActionLabel: "Closed — Sync to See Result",
            nextActionReason: reason,
          }),
        },
      ),
    );
    render(<NameActionsModal name="unsettledname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    expect(await screen.findByTestId("closed-result-unknown")).toHaveTextContent(reason);
    expect(screen.getByTestId("name-phase")).toHaveTextContent("Closed — Sync to See Result");
    expect(screen.queryByText(/^Owned$/)).not.toBeInTheDocument();
    expect(screen.queryByText(/already registered/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Redeem$/i })).not.toBeInTheDocument();
  });

  it("offers Redeem on a revoked name holding a losing reveal of ours", async () => {
    // hsd lets an auction's losers redeem "even in a revoked state" until the
    // revoked name expires, so a rival's revoke must not hide the button.
    const reason = "Your bid lost. Redeem your reveal coin to reclaim the funds.";
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "revokedname",
          state: "REVOKED",
          height: 100,
          renewal: 140,
          owner: { hash: "ab".repeat(32), index: 0 },
          registered: true,
          value: 800_000,
          highest: 800_000,
          stats: { revokePeriodStart: 150, revokePeriodEnd: 215, blocksUntilReopen: 30 },
        },
        {
          capabilities: makeCapabilities({
            name: "revokedname",
            phase: "REVOKED",
            taskState: "lostNeedsRedeem",
            ownsName: false,
            hasRevealCoin: true,
            redeemableRevealCount: 1,
            redeemableValueDoos: 300_000,
            canRedeem: { allowed: true, reason: null },
            nextActionKey: "REDEEM",
            nextActionLabel: "Redeem Lockup",
            nextActionReason: reason,
          }),
        },
      ),
    );
    render(<NameActionsModal name="revokedname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    const panel = await screen.findByTestId("guided-redeem");
    expect(panel).toHaveTextContent(reason);
    expect(within(panel).getByRole("button", { name: /^Redeem$/i })).toBeEnabled();
  });

  it("auto-expands the management section for an owned name without clicking the toggle", async () => {
    invokeMock.mockImplementation(
      routeModal(
        {
          name: "autoexpandname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: profile.receiveAddress, index: 0 },
          registered: true,
          value: 1_000_000,
          highest: 2_000_000,
          stats: { blocksUntilExpire: 100 },
        },
        {
          capabilities: {
            name: "autoexpandname",
            phase: "CLOSED",
            taskState: "ownedNoUrgentAction",
            ownsName: true,
            // Registered, but its owner coin has not synced — the sections
            // exist (that is the stage), and each button carries its own
            // "not synced" reason.
            nameIsRegistered: true,
            transferPending: false,
            redeemableRevealCount: 0,
            redeemableValueDoos: 0,
            hasBidCommitment: false,
            hasRevealCoin: false,
            hasOwnerCoin: false,
            canOpen: { allowed: false, reason: null },
            canBid: { allowed: false, reason: "Phase is CLOSED" },
            canReveal: { allowed: false, reason: "No commitment" },
            canRedeem: { allowed: false, reason: null },
            canRegister: { allowed: false, reason: "owner coin not synced" },
            canUpdate: { allowed: false, reason: "owner coin not synced" },
            canTransfer: { allowed: false, reason: "owner coin not synced" },
            canFinalize: { allowed: false, reason: "owner coin not synced" },
            canCancelTransfer: { allowed: false, reason: "owner coin not synced" },
            canRenew: { allowed: false, reason: "owner coin not synced" },
            canRevoke: { allowed: false, reason: "owner coin not synced" },
            nextActionKey: null,
            nextActionLabel: "Manage",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
          },
        },
      ),
    );
    render(<NameActionsModal name="autoexpandname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    // No click on the "Manage actions" toggle — management controls should already be visible.
    const transferBtn = await screen.findByRole("button", { name: /^Transfer$/i });
    expect(transferBtn).toBeDisabled();
    const renewBtn = screen.getByRole("button", { name: /^Renew$/i });
    expect(renewBtn).toBeDisabled();

    // Owned names must never hit the stale "already registered" auction-block copy.
    expect(
      screen.queryByText(/This name is already registered\. No auction actions are needed/i),
    ).not.toBeInTheDocument();
  });
});

describe("NameActionsModal — write-capability gating", () => {
  it("disables the primary action when canWrite is false", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_name_action_capabilities") {
        return Promise.resolve(null);
      }
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: profile.id,
            unlocked: false,
            unlockedUntilEpochMs: null,
          });
        case "get_write_capability":
          return Promise.resolve({
            signerUnlocked: false,
            broadcasterAvailable: false,
            canWrite: false,
            reason: "Wallet is locked",
          });
        case "read_name_info":
          return Promise.resolve({
            name: "gatedname",
            state: "AVAILABLE",
            height: null,
            renewal: null,
            owner: null,
            value: null,
            highest: null,
            stats: null,
          });
        default:
          return Promise.resolve(null);
      }
    });

    render(<NameActionsModal name="gatedname" open onClose={() => {}} />, { wrapper: wrapper() });

    // Wait until the blocked banner appears, then assert the guided action is disabled.
    expect(await screen.findByTestId("name-actions-blocked")).toHaveTextContent(
      /name actions unavailable/i,
    );

    await waitFor(() => {
      const btn = screen.getByRole("button", { name: /Open/i });
      expect(btn).toBeDisabled();
    });
  });
});
