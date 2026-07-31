import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
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
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";
import { AuctionsView } from "../AuctionsView";
import { NameActionsModal } from "../NameActionsModal";

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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "get_wallet_balances":
          return Promise.resolve({ liquidDoos: 5_000_000, nameControlDoos: 0, nameLockupDoos: 0, totalDoos: 5_000_000 });
        case "read_balance":
          return Promise.resolve({ confirmed: 0, unconfirmed: 0, locked_confirmed: 0, locked_unconfirmed: 0 });
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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
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
  function routeModal(nameInfo: Record<string, unknown> | null, overrides: Record<string, unknown> = {}) {
    return (cmd: string) => {
      if (cmd === "get_name_action_capabilities") {
        return Promise.resolve(overrides.capabilities ?? null);
      }
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
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
      routeModal({
        name: "closedname",
        state: "CLOSED",
        height: 100,
        renewal: 200,
        owner: { hash: profile.receiveAddress, index: 0 },
        value: 1_000_000,
        highest: 2_000_000,
        stats: { blocksUntilExpire: 100 },
      }, {
        capabilities: {
          name: "closedname",
          phase: "CLOSED",
          taskState: "wonNeedsRegister",
          ownsName: true,
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
      }),
    );
    render(<NameActionsModal name="closedname" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByText("Register Name")).toBeInTheDocument();
    expect(screen.getByText("Register")).toBeInTheDocument();
  });

  it("shows owner-manage for CLOSED name when wallet owns it and it is already registered", async () => {
    invokeMock.mockImplementation(
      routeModal({
        name: "ownedname",
        state: "CLOSED",
        height: 100,
        renewal: 200,
        owner: { hash: profile.receiveAddress, index: 0 },
        registered: true,
        value: 1_000_000,
        highest: 2_000_000,
        stats: { blocksUntilExpire: 100 },
      }, {
        capabilities: {
          name: "ownedname",
          phase: "CLOSED",
          taskState: "ownedNoUrgentAction",
          ownsName: true,
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
      }),
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
    const notSyncedReason =
      "owner coin not synced locally — connect a node and Refresh to manage";
    invokeMock.mockImplementation(
      routeModal({
        name: "unsyncedname",
        state: "CLOSED",
        height: 100,
        renewal: 200,
        owner: { hash: profile.receiveAddress, index: 0 },
        registered: true,
        value: 1_000_000,
        highest: 2_000_000,
        stats: { blocksUntilExpire: 100 },
      }, {
        capabilities: {
          name: "unsyncedname",
          phase: "CLOSED",
          taskState: "ownedNoUrgentAction",
          ownsName: true,
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
      }),
    );
    render(<NameActionsModal name="unsyncedname" open onClose={() => {}} />, { wrapper: wrapper() });

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

  it("auto-expands the management section for an owned name without clicking the toggle", async () => {
    invokeMock.mockImplementation(
      routeModal({
        name: "autoexpandname",
        state: "CLOSED",
        height: 100,
        renewal: 200,
        owner: { hash: profile.receiveAddress, index: 0 },
        registered: true,
        value: 1_000_000,
        highest: 2_000_000,
        stats: { blocksUntilExpire: 100 },
      }, {
        capabilities: {
          name: "autoexpandname",
          phase: "CLOSED",
          taskState: "ownedNoUrgentAction",
          ownsName: true,
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
      }),
    );
    render(<NameActionsModal name="autoexpandname" open onClose={() => {}} />, { wrapper: wrapper() });

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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: false, unlockedUntilEpochMs: null });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: false, broadcasterAvailable: false, canWrite: false, reason: "Wallet is locked" });
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
    expect(await screen.findByTestId("name-actions-blocked")).toHaveTextContent(/name actions unavailable/i);

    await waitFor(() => {
      const btn = screen.getByRole("button", { name: /Open/i });
      expect(btn).toBeDisabled();
    });
  });
});
