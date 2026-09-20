import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));

import { AuctionsView } from "../AuctionsView";

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

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/auctions"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

/** Base capability shape — override per test/name. */
function baseCaps(name: string, overrides: Record<string, unknown>) {
  return {
    name,
    phase: "AVAILABLE",
    taskState: "unavailableOther",
    ownsName: false,
    nameIsRegistered: false,
    transferPending: false,
    hasBidCommitment: false,
    hasRevealCoin: false,
    hasOwnerCoin: false,
    canOpen: { allowed: false, reason: null },
    canBid: { allowed: false, reason: null },
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
    ...overrides,
  };
}

function baseRoutes(o: {
  positions?: string[];
  names?: unknown[];
  capsByName?: Record<string, Record<string, unknown>>;
}) {
  return (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "get_names_action_capabilities") {
      const names = (args as { names?: string[] })?.names ?? [];
      return Promise.resolve(names.map((n) => o.capsByName?.[n] ?? baseCaps(n, {})));
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
      case "read_names":
        return Promise.resolve(o.names ?? []);
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "refresh_tx_confirmations":
        return Promise.resolve(null);
      case "read_name_info":
        return Promise.resolve(null);
      case "read_auction_position_names":
        return Promise.resolve(o.positions ?? []);
      default:
        return Promise.resolve(null);
    }
  };
}

describe("AuctionsView — auction positions merged with live caps (Task 2)", () => {
  it("shows a confirmed-open position with readyToBid caps as a Bidding row with countdown + Place Bid action", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["namehold"],
        names: [],
        capsByName: {
          namehold: baseCaps("namehold", {
            phase: "BIDDING",
            taskState: "readyToBid",
            canBid: { allowed: true, reason: null },
            nextActionKey: "BID",
            nextActionLabel: "Place Bid",
            countdownLabel: "Reveal starts in",
            countdownBlocks: 5,
            countdownHours: 1,
          }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(".namehold")).toBeInTheDocument();
    expect(screen.getByText(/Ready to Bid/i)).toBeInTheDocument();
    expect(screen.getByText(/5 blocks/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Place Bid$/i })).toBeInTheDocument();
    expect(screen.getByText(/Active Auctions \(1\)/i)).toBeInTheDocument();
  });

  it("routes the RAW name (punycode-safe) to the modal when a position row's action is clicked", async () => {
    // `xn--e1adigm` decodes to "козел" — the row must render the pretty
    // Unicode form while the click routes the RAW ACE name to the modal.
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["xn--e1adigm"],
        names: [],
        capsByName: {
          "xn--e1adigm": baseCaps("xn--e1adigm", {
            phase: "BIDDING",
            taskState: "readyToBid",
            canBid: { allowed: true, reason: null },
            nextActionKey: "BID",
            nextActionLabel: "Place Bid",
          }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(".козел")).toBeInTheDocument();
    expect(screen.queryByText(".xn--e1adigm")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Place Bid$/i }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "read_name_info");
      expect(call?.[1]).toEqual({ name: "xn--e1adigm" });
    });
    // This used to assert that the modal fetched `get_name_action_capabilities`
    // for the raw name — which `handleOpenManagement`'s cache bridge, added in
    // the same commit as this test, deliberately prevents: it seeds the single
    // capability query from the batch the table already holds, so the modal
    // opens on the row's task state instead of flashing the raw on-chain phase.
    // The RAW-name routing is already proven above; what is worth pinning here
    // is the bridge itself, keyed by the raw name so nothing refetches.
    expect(invokeMock.mock.calls.some((c) => c[0] === "get_name_action_capabilities")).toBe(false);
    const batchCall = invokeMock.mock.calls.find((c) => c[0] === "get_names_action_capabilities");
    expect(batchCall?.[1]).toMatchObject({ names: ["xn--e1adigm"] });
  });

  it("dedups a position name that's already owned — no double row, not double-counted", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["ownedname"],
        names: [
          {
            name: "ownedname",
            state: "CLOSED",
            height: 100,
            renewal: 200,
            owner: { hash: "abc", index: 0 },
            stats: null,
            registered: true,
          },
        ],
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/No active auctions/i);
    expect(screen.queryByText(".ownedname")).not.toBeInTheDocument();
    expect(screen.getByText(/Active Auctions \(0\)/i)).toBeInTheDocument();
    // Deduped BEFORE the caps batch — never even requested.
    expect(invokeMock.mock.calls.some((c) => c[0] === "get_names_action_capabilities")).toBe(false);
  });

  it("does NOT show a position whose live caps say unavailableOther (nothing to do)", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["donename"],
        names: [],
        capsByName: {
          donename: baseCaps("donename", { taskState: "unavailableOther" }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/No active auctions/i);
    expect(screen.queryByText(".donename")).not.toBeInTheDocument();
    expect(screen.getByText(/Active Auctions \(0\)/i)).toBeInTheDocument();
  });

  it("a confirmed open whose caps are still availableToOpen shows 'In auction' / 'View' — never invites a re-open", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["justopened"],
        names: [],
        capsByName: {
          justopened: baseCaps("justopened", {
            phase: "AVAILABLE",
            taskState: "availableToOpen",
            canOpen: { allowed: true, reason: null },
            nextActionKey: "OPEN",
            nextActionLabel: "Open Auction",
          }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(".justopened")).toBeInTheDocument();
    expect(screen.getByText(/In auction/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^View$/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Open Auction/i })).not.toBeInTheDocument();
  });

  it("a broadcasted open the node/explorer hasn't caught up to (waitingForBidding) shows Waiting for Bidding / View", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["waitingname"],
        names: [],
        capsByName: {
          waitingname: baseCaps("waitingname", {
            phase: "OPENING",
            taskState: "waitingForBidding",
            nextActionKey: "NONE",
            nextActionLabel: "Wait for Bidding",
          }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(".waitingname")).toBeInTheDocument();
    expect(screen.getByText(/Waiting for Bidding/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^View$/i })).toBeInTheDocument();
    // Folded into the count, not the empty state.
    expect(screen.getByText(/Active Auctions \(1\)/i)).toBeInTheDocument();
    expect(screen.queryByText(/No active auctions/i)).not.toBeInTheDocument();
  });

  it("an action of ours still in the mempool outranks the phase label", async () => {
    // Between broadcast and the next block the chain still reports the old
    // phase, so the row read "Ready to Bid" right after the user placed a bid —
    // as if nothing had happened. On a chain that mines on demand it stayed
    // that way indefinitely.
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["inflight"],
        names: [],
        capsByName: {
          inflight: baseCaps("inflight", {
            phase: "BIDDING",
            taskState: "readyToBid",
            nextActionKey: "BID",
            nextActionLabel: "Place Bid",
            pendingBroadcastAction: "bid",
          }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(".inflight")).toBeInTheDocument();
    expect(await screen.findByTestId("auction-pending-block")).toHaveTextContent(
      "Bid · waiting for a block",
    );
    expect(screen.queryByText(/^Ready to Bid$/i)).toBeNull();
  });

  it("filters a dropped-open position (no live caps) while keeping a live bidding sibling — the vmp3rt3/vmp3rt4 case", async () => {
    // Mirrors an observed regtest sequence: `vmp3rt4`'s open tx was broadcast
    // but never confirmed, so its draft settled to `dropped`; `vmp3rt3`'s open
    // confirmed and its bid confirmed. Even if the backend were to leak the
    // dropped name into positions, it has no live caps in an active-position
    // task state, so the client must not render it — only `vmp3rt3` shows.
    invokeMock.mockImplementation(
      baseRoutes({
        positions: ["vmp3rt3", "vmp3rt4"],
        names: [],
        capsByName: {
          vmp3rt3: baseCaps("vmp3rt3", {
            phase: "BIDDING",
            taskState: "readyToBid",
            hasBidCommitment: true,
            canBid: { allowed: true, reason: null },
            nextActionKey: "BID",
            nextActionLabel: "Place Bid",
            countdownLabel: "Reveal starts in",
            countdownBlocks: 5,
            countdownHours: 1,
          }),
          // Dropped open: node has no auction for it, so its caps land in a
          // non-active state (nothing to do).
          vmp3rt4: baseCaps("vmp3rt4", { taskState: "unavailableOther" }),
        },
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(".vmp3rt3")).toBeInTheDocument();
    // Multi-bid: a BIDDING row that already holds a commitment (readyToBid +
    // hasBidCommitment) is labeled "Bidding" to match the modal's phase badge,
    // while still inviting another independent bid.
    expect(screen.getByText(/Bidding/i)).toBeInTheDocument();
    // vmp3rt4 must not render as a row, and the count reflects only the live one.
    expect(screen.queryByText(".vmp3rt4")).not.toBeInTheDocument();
    expect(screen.getByText(/Active Auctions \(1\)/i)).toBeInTheDocument();
  });
});
