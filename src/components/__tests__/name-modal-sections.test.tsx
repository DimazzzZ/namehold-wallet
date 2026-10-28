import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// The name page's tabs are a map of the name's lifecycle, not a catalogue
// of the wallet's verbs: a section is live, still ahead (one muted line), or
// not there at all. These drive it through the stages where the old flat
// `ownsName` filter got it wrong.

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NamePageAt } from "../../test/namePage";

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
        <>{children}</>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("Name page — sections while the auction is still running", () => {
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
    render(<NamePageAt path="/names/leadingname?tab=dns" />, {
      wrapper: wrapper(),
    });

    expect(await screen.findByTestId("upcoming-section-records")).toBeInTheDocument();
    expect(screen.queryByTestId("dns-advanced-toggle")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: /^Transfer & sell/ }));
    expect(await screen.findByText(/available after you register this name/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel transfer" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Renew" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: /^Sign message/ }));
    expect(screen.getByTestId("sign-unavailable")).toHaveTextContent(
      "Sign message — available after you register this name.",
    );
    expect(screen.queryByText(/Sign message for/)).not.toBeInTheDocument();
  });

  // The upcoming state is only worth anything if it reaches the screen. It
  // does so beside a live section — here the reveal the wallet still owes —
  // which is the common shape of this stage, not an edge case.
  it("names what records and ownership are waiting for, beside the live reveal", async () => {
    invokeMock.mockImplementation(route(revealInfo, { ...leaderCaps, canReveal: ok }));
    render(<NamePageAt path="/names/leadingname" />, {
      wrapper: wrapper(),
    });

    // Records, ownership and signing all wait on the same thing: each tab
    // carries the muted dot that says so.
    expect(await screen.findByTestId("tab-hint-dns")).toHaveAccessibleName(
      "Available after you register this name",
    );
    expect(screen.getByTestId("tab-hint-transfer")).toBeInTheDocument();
    expect(screen.getByTestId("tab-hint-sign")).toBeInTheDocument();
    expect(screen.getByTestId("manual-auction-toggle")).toHaveTextContent("Manual auction actions");
  });

  // The old green badge read the same wrong flag the sections used to. Leading
  // an auction is not holding the name, and saying so in the page's status
  // line is the claim the user has least reason to doubt.
  it("does not claim the wallet owns a name it is only leading", async () => {
    invokeMock.mockImplementation(route(revealInfo, leaderCaps));
    render(<NamePageAt path="/names/leadingname" />, {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(screen.getByTestId("name-phase")).toHaveTextContent("Revealed"));
    expect(screen.getByTestId("name-phase")).not.toHaveTextContent(/Owned|You won/);
  });

  // The DNS tab shows the read-only block whenever the editor is not live.
  // Point that gate at `ownsName` and DNS vanishes from this stage entirely.
  it("still shows the read-only DNS block the editor is not taking over", async () => {
    invokeMock.mockImplementation(
      route(revealInfo, { ...leaderCaps, taskState: "unavailableOther", ownsName: false }),
    );
    render(<NamePageAt path="/names/leadingname?tab=dns" />, {
      wrapper: wrapper(),
    });

    expect(await screen.findByTestId("name-info-dns-no-node")).toBeInTheDocument();
    expect(screen.queryByTestId("dns-advanced-toggle")).not.toBeInTheDocument();
  });
});

describe("Name page — sections while a broadcast waits for a block", () => {
  it("hides the header primary and every owner action", async () => {
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
    render(<NamePageAt path="/names/pendingname?tab=transfer" />, {
      wrapper: wrapper(),
    });
    await waitFor(() =>
      expect(screen.getByTestId("name-phase")).toHaveTextContent(
        "Update sent · waiting for a block",
      ),
    );
    expect(screen.queryByTestId("name-primary")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Renew" })).not.toBeInTheDocument();
    expect(
      screen.getByText("Actions come back once the sent transaction is mined."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: /^Sign message/ }));
    expect(screen.getByTestId("sign-unavailable")).toHaveTextContent(
      "Actions come back once the sent transaction is mined.",
    );
  });
});

describe("Name page — sections on a name the wallet really owns", () => {
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
    render(<NamePageAt path="/names/ownedname?tab=dns" />, { wrapper: wrapper() });

    expect(await screen.findByTestId("dns-advanced-toggle")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: /^Transfer & sell/ }));
    // The header primary and the Renew card.
    await waitFor(() => expect(screen.getAllByRole("button", { name: "Renew" })).toHaveLength(2));
    fireEvent.click(screen.getByRole("tab", { name: /^Sign message/ }));
    expect(await screen.findByText(/Sign message for/)).toBeInTheDocument();
  });

  // The paid-swap entry points are withdrawn. Only the holder of the TRANSFER
  // coin can finalize, which is the sender, so "Buy with payment" could never
  // be pressed by a buyer; and with every input signed SIGHASH_ALL nothing
  // about the flow is atomic. See docs/specs/2026-09-21-paid-name-swaps.md.
  // Claiming an offer already recorded is untouched — that panel renders
  // itself only when one exists.
  it("offers no way to start a paid swap", async () => {
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
          nameIsRegistered: true,
          hasOwnerCoin: true,
          canUpdate: ok,
          canTransfer: ok,
          canFinalize: ok,
          canRenew: ok,
        }),
      ),
    );
    render(<NamePageAt path="/names/ownedname?tab=transfer" />, { wrapper: wrapper() });
    await screen.findByTestId("name-sell");

    expect(screen.queryByRole("button", { name: /sell with payment/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /buy with payment/i })).not.toBeInTheDocument();
  });
});

describe("Name page — the Register step", () => {
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
    render(<NamePageAt path="/names/wonname" />, { wrapper: wrapper() });

    await screen.findByText("Register Name");
    expect(screen.getByText(/DNS records are optional/i)).toBeInTheDocument();
    // Not in the way: no rows until the user asks for them.
    expect(screen.queryByTestId("dns-rows")).not.toBeInTheDocument();
    // And Register is reachable without touching them: the header primary and
    // the guided panel's secondary.
    await waitFor(() => {
      for (const b of screen.getAllByRole("button", { name: "Register" })) expect(b).toBeEnabled();
    });
    expect(screen.getByTestId("name-primary")).toHaveTextContent("Register");
  });

  it("opens the editor for whoever does want records up front", async () => {
    invokeMock.mockImplementation(route(wonInfo, wonCaps));
    render(<NamePageAt path="/names/wonname" />, { wrapper: wrapper() });

    fireEvent.click(await screen.findByTestId("register-dns-toggle"));
    expect(await screen.findByTestId("dns-rows")).toBeInTheDocument();
  });
});

describe("Name page — the Register step, laid out", () => {
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

  async function openManual() {
    invokeMock.mockImplementation(route(wonRedeemInfo, wonRedeemCaps));
    render(<NamePageAt path="/names/wonredeem" />, { wrapper: wrapper() });
    await screen.findByText("Register Name");
    fireEvent.click(await screen.findByTestId("manual-auction-toggle"));
  }

  // The disclosure was a bare inline <button> beside the Register <Button>, so
  // the two shared a line and read as one broken control row.
  it("puts the optional-records disclosure on its own line, not beside Register", async () => {
    invokeMock.mockImplementation(route(wonRedeemInfo, wonRedeemCaps));
    render(<NamePageAt path="/names/wonredeem" />, { wrapper: wrapper() });

    const toggle = await screen.findByTestId("register-dns-toggle");
    // The guided panel's Register (the header primary comes first).
    const register = screen.getAllByRole("button", { name: "Register" }).slice(-1)[0]!;
    expect(toggle.parentElement).not.toBe(register.parentElement);
  });

  // Register is the header primary. Repeating it inside the records section
  // gave two identical live buttons for one action, with nothing to choose
  // between; on Overview the guided panel's copy is secondary.
  it("offers Register once on the DNS tab, and only one blue Register anywhere", async () => {
    invokeMock.mockImplementation(route(wonRedeemInfo, wonRedeemCaps));
    render(<NamePageAt path="/names/wonredeem?tab=dns" />, { wrapper: wrapper() });
    await screen.findByTestId("dns-advanced-toggle");
    expect(screen.getAllByRole("button", { name: "Register" })).toHaveLength(1);
    expect(screen.getByTestId("name-primary")).toHaveTextContent("Register");

    fireEvent.click(screen.getByRole("tab", { name: /^Overview/ }));
    await screen.findByTestId("register-dns-toggle");
    const blue = screen
      .getAllByRole("button", { name: "Register" })
      .filter((b) => b.className.includes("bg-blue-600"));
    expect(blue).toEqual([screen.getByTestId("name-primary")]);
  });

  // "Redeem" is a covenant name. On its own it tells the user nothing about
  // the 28 HNS the wallet is holding for them.
  it("says what Redeem reclaims, and hides the auction actions that are refused", async () => {
    await openManual();
    expect(screen.getByText(/28(\.0+)? HNS/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Redeem" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Open" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reveal" })).not.toBeInTheDocument();
  });
});

describe("Name page — the window between broadcasting and the block", () => {
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
    render(<NamePageAt path="/names/lostname" />, { wrapper: wrapper() });

    await waitFor(() =>
      expect(screen.getByTestId("name-phase")).toHaveTextContent(/waiting for a block/i),
    );
    expect(screen.getByTestId("name-phase")).not.toHaveTextContent(/redeem now/i);
  });

  it("keeps the panel and the header telling the same story", async () => {
    invokeMock.mockImplementation(route(lostInfo, redeemInFlight));
    render(<NamePageAt path="/names/lostname" />, { wrapper: wrapper() });

    expect(await screen.findByText(/broadcast and waiting to be mined/i)).toBeInTheDocument();
    expect(screen.getByTestId("name-phase")).not.toHaveTextContent(/lost/i);
  });
});
