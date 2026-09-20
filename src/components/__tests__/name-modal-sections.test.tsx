import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor } from "@testing-library/react";
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
