import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// The modal's advanced area is a map of the name's lifecycle, not a catalogue
// of the wallet's verbs: a section is live, still ahead (one muted line), or
// not there at all. These drive it through the stages where the old flat
// `ownsName` filter got it wrong.

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

const no = { allowed: false, reason: "not now" };
const ok = { allowed: true, reason: null };

function capsFor(over: Record<string, unknown>) {
  return {
    name: "n",
    phase: "CLOSED",
    taskState: "unavailableOther",
    ownsName: false,
    transferPending: false,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    nameIsRegistered: false,
    hasBidCommitment: false,
    hasBidCoin: false,
    hasRevealCoin: false,
    hasOwnerCoin: false,
    revealTxid: null,
    bidValueDoos: null,
    lockupValueDoos: null,
    myBidCount: 0,
    canOpen: no,
    canBid: no,
    canReveal: no,
    canRedeem: no,
    canRegister: no,
    canUpdate: no,
    canTransfer: no,
    canFinalize: no,
    canCancelTransfer: no,
    canRenew: no,
    canRevoke: no,
    nextActionKey: null,
    nextActionLabel: null,
    nextActionReason: null,
    countdownLabel: null,
    countdownBlocks: null,
    countdownHours: null,
    ...over,
  };
}

function route(nameInfo: Record<string, unknown>, capabilities: Record<string, unknown>) {
  return (cmd: string) => {
    switch (cmd) {
      case "get_name_action_capabilities":
        return Promise.resolve(capabilities);
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

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("NameActionsModal — sections while the auction is still running", () => {
  const revealInfo = {
    name: "leadingname",
    state: "REVEAL",
    height: 100,
    renewal: null,
    owner: { hash: profile.receiveAddress, index: 0 },
    value: null,
    highest: 5_000_000,
    stats: { blocksUntilClose: 20 },
  };
  const leaderCaps = capsFor({
    name: "leadingname",
    phase: "REVEAL",
    taskState: "revealDoneWaitingForClose",
    // hsd names the highest revealer as owner long before anyone has won.
    ownsName: true,
    transferPending: false,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    nameIsRegistered: false,
    hasOwnerCoin: true,
    hasRevealCoin: true,
  });

  it("offers no records, ownership or signing on a name it has only bid on", async () => {
    invokeMock.mockImplementation(route(revealInfo, leaderCaps));
    render(<NameActionsModal name="leadingname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    await screen.findByTestId("name-phase");
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByTestId("dns-advanced-toggle")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel transfer" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Renew" })).not.toBeInTheDocument();
    expect(screen.queryByText(/Sign message for/)).not.toBeInTheDocument();
  });

  // The upcoming state is only worth anything if it reaches the screen. It
  // does so beside a live section — here the reveal the wallet still owes —
  // which is the common shape of this stage, not an edge case.
  it("names what records and ownership are waiting for, beside the live reveal", async () => {
    invokeMock.mockImplementation(route(revealInfo, { ...leaderCaps, canReveal: ok }));
    render(<NameActionsModal name="leadingname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    fireEvent.click(await screen.findByTestId("all-actions-toggle"));
    // Both records and ownership are waiting on the same thing, and both say so.
    expect(screen.getAllByText(/after you register this name/)).toHaveLength(2);
    expect(screen.getByText("DNS records")).toBeInTheDocument();
    expect(screen.getByText("Ownership")).toBeInTheDocument();
    expect(screen.getByText("Manual auction actions")).toBeInTheDocument();
  });

  it("does not call the menu 'Manage actions' when there is nothing to manage", async () => {
    invokeMock.mockImplementation(route(revealInfo, { ...leaderCaps, canReveal: ok }));
    render(<NameActionsModal name="leadingname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });

    const toggle = await screen.findByTestId("all-actions-toggle");
    expect(toggle).toHaveTextContent("Show all actions");
  });

  // The green badge reads the same wrong flag the sections used to. Leading an
  // auction is not holding the name, and saying so in the modal's most
  // confident-looking element is the claim the user has least reason to doubt.
  it("does not claim the wallet owns a name it is only leading", async () => {
    invokeMock.mockImplementation(route(revealInfo, leaderCaps));
    render(<NameActionsModal name="leadingname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });
    await screen.findByTestId("name-phase");

    expect(screen.queryByTestId("ownership-indicator")).not.toBeInTheDocument();
  });

  // The read-only DNS block in NameDetails is suppressed only when the
  // editable section has taken the records over. Those two gates have to move
  // together: point the read-only one at `ownsName` again and DNS vanishes
  // from this stage entirely — hidden here, and not rendered there either.
  it("still shows the read-only DNS block the editor is not taking over", async () => {
    invokeMock.mockImplementation(route(revealInfo, leaderCaps));
    render(<NameActionsModal name="leadingname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });
    await screen.findByTestId("name-phase");

    expect(await screen.findByText("DNS Records")).toBeInTheDocument();
    expect(screen.queryByTestId("dns-advanced-toggle")).not.toBeInTheDocument();
  });
});

describe("NameActionsModal — sections while a broadcast waits for a block", () => {
  it("closes the advanced menu entirely", async () => {
    invokeMock.mockImplementation(
      route(
        {
          name: "pendingname",
          state: "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: profile.receiveAddress, index: 0 },
          registered: true,
          value: 1_000_000,
          highest: 2_000_000,
          stats: { blocksUntilExpire: 100 },
        },
        capsFor({
          name: "pendingname",
          taskState: "ownedNoUrgentAction",
          ownsName: true,
          transferPending: false,
          redeemableRevealCount: 0,
          redeemableValueDoos: 0,
          nameIsRegistered: true,
          hasOwnerCoin: true,
          canUpdate: ok,
          canRenew: ok,
          pendingBroadcastAction: "update",
        }),
      ),
    );
    render(<NameActionsModal name="pendingname" open onClose={() => {}} />, {
      wrapper: wrapper(),
    });
    await screen.findByTestId("name-phase");
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByTestId("all-actions-toggle")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Renew" })).not.toBeInTheDocument();
    // Auto-expand still fires for a registered name, so without its own guard
    // the container renders as an empty bordered box holding nothing.
    expect(screen.queryByTestId("advanced-actions")).not.toBeInTheDocument();
  });
});

describe("NameActionsModal — sections on a name the wallet really owns", () => {
  it("keeps records, ownership and signing available", async () => {
    invokeMock.mockImplementation(
      route(
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
        capsFor({
          name: "ownedname",
          taskState: "ownedNoUrgentAction",
          ownsName: true,
          transferPending: false,
          redeemableRevealCount: 0,
          redeemableValueDoos: 0,
          nameIsRegistered: true,
          hasOwnerCoin: true,
          canUpdate: ok,
          canTransfer: ok,
          canRenew: ok,
          canRevoke: ok,
        }),
      ),
    );
    render(<NameActionsModal name="ownedname" open onClose={() => {}} />, { wrapper: wrapper() });
    await screen.findByTestId("name-phase");

    expect(await screen.findByTestId("dns-advanced-toggle")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Renew" })).toBeInTheDocument();
    expect(await screen.findByText(/Sign message for/)).toBeInTheDocument();
  });
});

describe("NameActionsModal — the Register step", () => {
  const wonInfo = {
    name: "wonname",
    state: "CLOSED",
    height: 100,
    renewal: 200,
    owner: { hash: profile.receiveAddress, index: 0 },
    registered: false,
    value: 12_000_000,
    highest: 1_000_000_000,
    stats: { blocksUntilExpire: 4979, daysUntilExpire: 34.5 },
  };
  const wonCaps = capsFor({
    name: "wonname",
    taskState: "wonNeedsRegister",
    ownsName: true,
    nameIsRegistered: false,
    hasOwnerCoin: true,
    hasRevealCoin: true,
    canRegister: ok,
    nextActionKey: "REGISTER",
    nextActionLabel: "Register Name",
  });

  // Reported from a live wallet: the Register panel dropped a DNS record
  // editor in front of the user with nothing said about it, so the obvious
  // reading was that records are required to register. They are not — hsd
  // caps the resource size and accepts an empty one — and the wallet already
  // sends an empty resource when the editor is untouched. The panel has to
  // say so, or the user stalls on a question the chain does not ask.
  it("says records are optional and keeps the editor out of the way", async () => {
    invokeMock.mockImplementation(route(wonInfo, wonCaps));
    render(<NameActionsModal name="wonname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByText("Register Name");
    expect(screen.getByText(/DNS records are optional/i)).toBeInTheDocument();
    // Not in the way: no rows until the user asks for them.
    expect(screen.queryByTestId("dns-rows")).not.toBeInTheDocument();
    // And Register is reachable without touching them.
    expect(screen.getByRole("button", { name: "Register" })).toBeEnabled();
  });

  it("opens the editor for whoever does want records up front", async () => {
    invokeMock.mockImplementation(route(wonInfo, wonCaps));
    render(<NameActionsModal name="wonname" open onClose={() => {}} />, { wrapper: wrapper() });

    fireEvent.click(await screen.findByTestId("register-dns-toggle"));
    expect(await screen.findByTestId("dns-rows")).toBeInTheDocument();
  });
});

describe("NameActionsModal — the Register step, laid out", () => {
  const wonRedeemInfo = {
    name: "wonredeem",
    state: "CLOSED",
    height: 100,
    renewal: 200,
    owner: { hash: profile.receiveAddress, index: 0 },
    registered: false,
    value: 12_000_000,
    highest: 1_000_000_000,
    stats: { blocksUntilExpire: 4979, daysUntilExpire: 34.5 },
  };
  // Won the name AND holding losing reveals on it — what outbidding yourself
  // leaves behind, and the state a live wallet was actually in.
  const wonRedeemCaps = capsFor({
    name: "wonredeem",
    taskState: "wonNeedsRegister",
    ownsName: true,
    nameIsRegistered: false,
    hasOwnerCoin: true,
    hasRevealCoin: true,
    canRegister: ok,
    canRedeem: ok,
    redeemableRevealCount: 3,
    redeemableValueDoos: 28_000_000,
    nextActionLabel: "Register Name",
  });

  async function openAdvanced() {
    invokeMock.mockImplementation(route(wonRedeemInfo, wonRedeemCaps));
    render(<NameActionsModal name="wonredeem" open onClose={() => {}} />, { wrapper: wrapper() });
    await screen.findByText("Register Name");
    const toggle = screen.queryByTestId("all-actions-toggle");
    if (toggle) fireEvent.click(toggle);
  }

  // The disclosure was a bare inline <button> beside the Register <Button>, so
  // the two shared a line and read as one broken control row.
  it("puts the optional-records disclosure on its own line, not beside Register", async () => {
    invokeMock.mockImplementation(route(wonRedeemInfo, wonRedeemCaps));
    render(<NameActionsModal name="wonredeem" open onClose={() => {}} />, { wrapper: wrapper() });

    const toggle = await screen.findByTestId("register-dns-toggle");
    const register = screen.getAllByRole("button", { name: "Register" })[0]!;
    expect(toggle.parentElement).not.toBe(register.parentElement);
  });

  // Register is the guided step. Repeating it inside the records section gave
  // two identical live buttons for one action, with nothing to choose between.
  it("offers Register exactly once while the guided step is showing it", async () => {
    await openAdvanced();
    expect(screen.getAllByRole("button", { name: "Register" })).toHaveLength(1);
  });

  // "Redeem" is a covenant name. On its own it tells the user nothing about
  // the 28 HNS the wallet is holding for them.
  it("says what Redeem reclaims, and hides the auction actions that are refused", async () => {
    await openAdvanced();
    expect(screen.getByText(/28(\.0+)? HNS/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Redeem" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Open" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reveal" })).not.toBeInTheDocument();
  });
});

describe("NameActionsModal — the window between broadcasting and the block", () => {
  // Reported live on a just-registered name. The root cause was the backend
  // losing track of ownership while our own register was unmined (pinned in
  // `names_action_context_tests`), but the header had a fault of its own: it
  // renders the task label whatever is in flight, so it kept urging an action
  // the wallet had already sent. The shape below is the one that survives the
  // backend fix — a genuinely lost auction whose redeem is on its way — and it
  // shows the same contradiction: a red "Lost — Redeem Now" over a panel
  // saying the redeem is already broadcast.
  const redeemInFlight = capsFor({
    name: "lostname",
    phase: "CLOSED",
    taskState: "lostNeedsRedeem",
    ownsName: false,
    nameIsRegistered: false,
    hasOwnerCoin: false,
    hasRevealCoin: true,
    redeemableRevealCount: 1,
    redeemableValueDoos: 5_000_000,
    pendingBroadcastAction: "redeem",
  });
  const lostInfo = {
    name: "lostname",
    state: "CLOSED",
    height: 779,
    renewal: 779,
    owner: { hash: "someoneelse", index: 0 },
    registered: true,
    value: 12_000_000,
    highest: 1_000_000_000,
    stats: { blocksUntilExpire: 4979, daysUntilExpire: 34.5 },
  };

  it("does not urge an action the wallet has already broadcast", async () => {
    invokeMock.mockImplementation(route(lostInfo, redeemInFlight));
    render(<NameActionsModal name="lostname" open onClose={() => {}} />, { wrapper: wrapper() });

    const badge = await screen.findByTestId("name-phase");
    expect(badge).not.toHaveTextContent(/redeem now/i);
    expect(badge).toHaveTextContent(/waiting for a block/i);
  });

  it("keeps the panel and the header telling the same story", async () => {
    invokeMock.mockImplementation(route(lostInfo, redeemInFlight));
    render(<NameActionsModal name="lostname" open onClose={() => {}} />, { wrapper: wrapper() });

    expect(await screen.findByText(/broadcast and waiting to be mined/i)).toBeInTheDocument();
    expect(screen.getByTestId("name-phase")).not.toHaveTextContent(/lost/i);
  });
});
