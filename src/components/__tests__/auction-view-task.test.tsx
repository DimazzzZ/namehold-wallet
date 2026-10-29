import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

import { NamesView } from "../NamesView";
import { NameRoute } from "../NamePage";
import { LocationProbe } from "../../test/namePage";

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
        <MemoryRouter initialEntries={["/names?filter=auction"]}>{children}</MemoryRouter>
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
    taskState: "availableToOpen",
    ownsName: false,
    nameIsRegistered: false,
    transferPending: false,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    hasBidCommitment: false,
    hasRevealCoin: false,
    hasOwnerCoin: false,
    canOpen: { allowed: true, reason: null },
    canBid: { allowed: false, reason: "Phase is AVAILABLE" },
    canReveal: { allowed: false, reason: "Phase is AVAILABLE" },
    canRedeem: { allowed: false, reason: "Phase is AVAILABLE" },
    canRegister: { allowed: false, reason: "Phase is AVAILABLE" },
    canUpdate: { allowed: false, reason: null },
    canTransfer: { allowed: false, reason: null },
    canFinalize: { allowed: false, reason: null },
    canCancelTransfer: { allowed: false, reason: null },
    canRenew: { allowed: false, reason: null },
    canRevoke: { allowed: false, reason: null },
    nextActionKey: "OPEN",
    nextActionLabel: "Open Auction",
    nextActionReason: null,
    countdownLabel: null,
    countdownBlocks: null,
    countdownHours: null,
    ...overrides,
  };
}

const CAPS_BY_NAME: Record<string, Record<string, unknown>> = {
  revealname: baseCaps("revealname", {
    phase: "REVEAL",
    taskState: "readyToReveal",
    hasBidCommitment: true,
    hasRevealCoin: true,
    canOpen: { allowed: false, reason: "Phase is REVEAL" },
    canBid: { allowed: false, reason: "Phase is REVEAL" },
    canReveal: { allowed: true, reason: null },
    canRedeem: { allowed: false, reason: null },
    canRegister: { allowed: false, reason: "Phase is REVEAL" },
    nextActionKey: "REVEAL",
    nextActionLabel: "Reveal Bid",
    nextActionReason: null,
    countdownLabel: "Auction closes in",
    countdownBlocks: 12,
    countdownHours: 2,
  }),
  wonname: baseCaps("wonname", {
    phase: "CLOSED",
    taskState: "wonNeedsRegister",
    ownsName: true,
    nameIsRegistered: false,
    transferPending: false,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    hasOwnerCoin: true,
    canOpen: { allowed: false, reason: "Phase is CLOSED" },
    canBid: { allowed: false, reason: "Phase is CLOSED" },
    canReveal: { allowed: false, reason: "No commitment" },
    canRedeem: { allowed: false, reason: "No reveal coin" },
    canRegister: { allowed: true, reason: null },
    canUpdate: { allowed: true, reason: null },
    canTransfer: { allowed: true, reason: null },
    canRenew: { allowed: true, reason: null },
    canRevoke: { allowed: true, reason: null },
    nextActionKey: "REGISTER",
    nextActionLabel: "Register Name",
    nextActionReason: "You won the auction! Register to finalize ownership.",
  }),
  lostname: baseCaps("lostname", {
    phase: "CLOSED",
    taskState: "lostNeedsRedeem",
    hasRevealCoin: true,
    canOpen: { allowed: false, reason: "Phase is CLOSED" },
    canBid: { allowed: false, reason: "Phase is CLOSED" },
    canReveal: { allowed: false, reason: "Phase is CLOSED" },
    canRedeem: { allowed: true, reason: null },
    canRegister: { allowed: false, reason: "Not the winner" },
    nextActionKey: "REDEEM",
    nextActionLabel: "Redeem Lockup",
    nextActionReason: "Your bid lost. Redeem your reveal coin.",
  }),
  // Pre-bid opening period: on-chain phase is OPENING (raw label "Opening"),
  // but the task state is waitingForBidding (label "Waiting for Bidding").
  // The modal must open on that task-state label, not flash the raw phase.
  openingname: baseCaps("openingname", {
    phase: "OPENING",
    taskState: "waitingForBidding",
    canOpen: { allowed: false, reason: "Auction already open" },
    canBid: { allowed: false, reason: "Bidding has not started" },
    nextActionKey: "NONE",
    nextActionLabel: "Wait for Bidding",
    nextActionReason: "Bidding opens automatically after the opening period.",
    countdownLabel: "Bidding starts in",
    countdownBlocks: 20,
    countdownHours: 3,
  }),
};

function routeInvoke() {
  return (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "get_names_action_capabilities") {
      const names = (args as { names?: string[] })?.names ?? [];
      return Promise.resolve(names.map((n) => CAPS_BY_NAME[n] ?? baseCaps(n, {})));
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
      case "list_watchlist":
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "read_names":
        return Promise.resolve([
          {
            name: "revealname",
            state: "REVEAL",
            height: 100,
            renewal: 200,
            owner: null,
            stats: null,
          },
          // registered: false explicitly (genuine "won, not yet registered" /
          // "lost, not yet redeemed" cases).
          {
            name: "wonname",
            state: "CLOSED",
            height: 100,
            renewal: 200,
            owner: { hash: "abc", index: 0 },
            stats: null,
            registered: false,
          },
          {
            name: "lostname",
            state: "CLOSED",
            height: 100,
            renewal: 200,
            owner: { hash: "abc", index: 0 },
            stats: null,
            registered: false,
          },
        ]);
      default:
        return Promise.resolve(null);
    }
  };
}

// The Auctions page is the Names page's In auction chip now.
describe("NamesView — task-driven row rendering", () => {
  it("shows task-state statuses for won / lost / reveal names", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    // Wait for capability-derived statuses
    expect(await screen.findByText("You won")).toBeInTheDocument();
    expect(await screen.findByText("You lost")).toBeInTheDocument();
    expect(await screen.findByText("Reveal your bid")).toBeInTheDocument();
    // Each row's action
    expect(screen.getByRole("button", { name: "Register" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Redeem" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reveal" })).toBeInTheDocument();
    // The count reflects the actual visible rows
    expect(screen.getByTestId("names-chip-auction")).toHaveTextContent("In auction 3");
    expect(screen.getAllByTestId("names-row")).toHaveLength(3);
  });

  it("fetches capabilities with ONE batch invoke, not one per row", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText("You won");

    const batchCalls = invokeMock.mock.calls.filter(
      (c) => c[0] === "get_names_action_capabilities",
    );
    // React Query may re-run the query function on remount/strict-effects,
    // but every call must carry the FULL name list — never a per-row call.
    expect(batchCalls.length).toBeGreaterThan(0);
    for (const call of batchCalls) {
      const names = (call[1] as { names?: string[] })?.names ?? [];
      expect(names.sort()).toEqual(["lostname", "revealname", "wonname"]);
    }
    // Crucially, the old per-name command must never be invoked from this view.
    expect(invokeMock.mock.calls.some((c) => c[0] === "get_name_action_capabilities")).toBe(false);
  });

  it("sorts rows by urgency: readyToReveal, then wonNeedsRegister, then lostNeedsRedeem", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText("You won");
    const rowText = screen.getAllByTestId("names-row").map((r) => r.textContent ?? "");
    const revealIdx = rowText.findIndex((t) => /Reveal your bid/.test(t));
    const wonIdx = rowText.findIndex((t) => /You won/.test(t));
    const lostIdx = rowText.findIndex((t) => /You lost/.test(t));
    expect(revealIdx).toBeGreaterThanOrEqual(0);
    expect(wonIdx).toBeGreaterThan(revealIdx);
    expect(lostIdx).toBeGreaterThan(wonIdx);
  });

  it("renders the countdown in the status detail instead of a raw height column", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText("Reveal your bid");
    expect(screen.queryByText(/^Height$/i)).not.toBeInTheDocument();
    // revealname has countdownBlocks: 12, countdownHours: 2.
    expect(screen.getByText("· 12 blocks (~2h) left")).toBeInTheDocument();
    // Names without countdown data fall back to an em dash.
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("keeps an owned CLOSED name with no task out of In auction, includes a won one", async () => {
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "get_names_action_capabilities") {
        const names = (args as { names?: string[] })?.names ?? [];
        return Promise.resolve(
          names.map((n) =>
            n === "wonneedsregister"
              ? baseCaps("wonneedsregister", {
                  phase: "CLOSED",
                  taskState: "wonNeedsRegister",
                  ownsName: true,
                  nameIsRegistered: false,
                  transferPending: false,
                  redeemableRevealCount: 0,
                  redeemableValueDoos: 0,
                  hasOwnerCoin: true,
                  canOpen: { allowed: false, reason: "Phase is CLOSED" },
                  canBid: { allowed: false, reason: "Phase is CLOSED" },
                  canReveal: { allowed: false, reason: "No commitment" },
                  canRedeem: { allowed: false, reason: "No reveal coin" },
                  canRegister: { allowed: true, reason: null },
                  canUpdate: { allowed: true, reason: null },
                  canTransfer: { allowed: true, reason: null },
                  canRenew: { allowed: true, reason: null },
                  canRevoke: { allowed: true, reason: null },
                  nextActionKey: "REGISTER",
                  nextActionLabel: "Register Name",
                  nextActionReason: "You won the auction! Register to finalize ownership.",
                })
              : // The owned name below: nothing to do, so not in an auction.
                baseCaps(n, {
                  phase: "CLOSED",
                  taskState: "ownedNoAction",
                  ownsName: true,
                  nameIsRegistered: true,
                  transferPending: false,
                  redeemableRevealCount: 0,
                  redeemableValueDoos: 0,
                  hasOwnerCoin: true,
                  canUpdate: { allowed: true, reason: null },
                  canTransfer: { allowed: true, reason: null },
                  canRenew: { allowed: true, reason: null },
                  canRevoke: { allowed: true, reason: null },
                  nextActionKey: "NONE",
                  nextActionLabel: "Owned",
                }),
          ),
        );
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
        case "list_watchlist":
        case "list_tx_drafts":
          return Promise.resolve([]);
        case "read_names":
          return Promise.resolve([
            // Genuinely-owned name whose `registered` flag is UNKNOWN (the field
            // is omitted entirely): explorer-sourced or otherwise-incomplete
            // data must NOT be treated as "still needs a task". Must be
            // excluded from In auction.
            {
              name: "unknownregisteredname",
              state: "CLOSED",
              height: 100,
              renewal: 200,
              owner: { hash: "abc", index: 0 },
              stats: null,
            },
            // Genuine "won, not yet registered" case — `registered` is
            // EXPLICITLY false. Must remain included in In auction.
            {
              name: "wonneedsregister",
              state: "CLOSED",
              height: 100,
              renewal: 200,
              owner: { hash: "def", index: 0 },
              stats: null,
              registered: false,
            },
          ]);
        default:
          return Promise.resolve(null);
      }
    });

    render(<NamesView />, { wrapper: wrapper() });

    // The genuine "won, needs register" name shows up as actionable.
    expect(await screen.findByText("You won")).toBeInTheDocument();

    // The owned name with unknown `registered` status must NOT leak into
    // In auction, and the count must reflect only the real task. (v2 sends
    // every owned name in the one caps batch; it lands under Owned instead.)
    expect(screen.queryByText(/unknownregisteredname/i)).not.toBeInTheDocument();
    expect(screen.getByTestId("names-chip-auction")).toHaveTextContent("In auction 1");
    expect(screen.getByTestId("names-chip-owned")).toHaveTextContent("Owned 1");
  });
});

describe("NamesView — canonical table design", () => {
  it("the names table follows the unified table contract", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });
    await screen.findByText("You won");

    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    const table = document.querySelector("table");
    expect(table).toBeTruthy();
    assertCanonicalTable(table as HTMLTableElement, { name: "Names" });
  });
});

describe("NamesView — batch-bid button", () => {
  it("batch-bid button disabled with its reason when profile is watch-only", async () => {
    const watchOnlyProfile = { ...profile, watchOnly: true };
    const route = (cmd: string, _args?: Record<string, unknown>) => {
      if (cmd === "get_names_action_capabilities") {
        return Promise.resolve([]);
      }
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([watchOnlyProfile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: watchOnlyProfile.id,
            unlocked: false,
            unlockedUntilEpochMs: 0,
          });
        case "get_write_capability":
          return Promise.resolve({
            signerUnlocked: false,
            broadcasterAvailable: false,
            canWrite: false,
            reason: "Watch-only",
          });
        case "list_watchlist":
        case "list_tx_drafts":
          return Promise.resolve([]);
        case "read_names":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    };
    invokeMock.mockImplementation(route);
    render(<NamesView />, { wrapper: wrapper() });

    // Wait for the view to load (empty state shows this message)
    await screen.findByText(/No active auctions/i);

    // Batch-bid is disabled once the watch-only profile resolves. Use waitFor
    // to allow the profile query to settle — the button renders enabled
    // (isWatchOnly defaults to false) until then.
    const btn = screen.getByTestId("open-batch-bid-btn");
    await waitFor(() => expect(btn).toBeDisabled());
    fireEvent.mouseEnter(btn.parentElement!);
    expect(await screen.findByText("Watch-only wallets can't bid")).toBeInTheDocument();
    fireEvent.click(btn);
    expect(screen.queryByText("Batch Bid")).not.toBeInTheDocument();
  });

  it("batch-bid button enabled when profile is non-custodial", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    // Wait for the view to load
    await screen.findByText("You won");

    // Batch-bid button should be enabled
    expect(screen.getByTestId("open-batch-bid-btn")).toBeEnabled();
  });

  it("clicking batch-bid button opens BatchBidModal", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText("You won");
    const batchBidBtn = screen.getByTestId("open-batch-bid-btn");
    fireEvent.click(batchBidBtn);

    // BatchBidModal should render with title "Batch Bid"
    await waitFor(() => {
      expect(screen.getByText("Batch Bid")).toBeInTheDocument();
    });
  });

  it("names:batchBid action-bus event opens modal when not watch-only", async () => {
    const { dispatchAction } = await import("../../lib/actionBus");
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText("You won");

    // Dispatch the action-bus event
    act(() => {
      dispatchAction("names:batchBid");
    });

    // Modal should open
    await waitFor(() => {
      expect(screen.getByText("Batch Bid")).toBeInTheDocument();
    });
  });

  it("closing BatchBidModal hides it", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText("You won");
    const batchBidBtn = screen.getByTestId("open-batch-bid-btn");
    fireEvent.click(batchBidBtn);

    // Modal opens
    await screen.findByText("Batch Bid");

    // Find and click the close button (× in the dialog header)
    const closeButtons = screen.getAllByRole("button", { name: "Close" });
    const closeBtn = closeButtons[closeButtons.length - 1]!; // last one is the modal close
    fireEvent.click(closeBtn);

    // Modal should be hidden
    await waitFor(() => {
      expect(screen.queryByText("Batch Bid")).not.toBeInTheDocument();
    });
  });
});

describe("NamesView ↔ name page — task-state label consistency", () => {
  // Regression: the row showed the capability-derived task-state label
  // ("Waiting for Bidding") while the modal, on open, briefly showed the raw
  // on-chain phase ("Opening") until its own single-name capability fetch
  // resolved. That flash is a state divergence the user can see: the row and
  // the modal for the SAME name disagree about the phase.
  //
  // Fix under test: (1) a cache bridge in NamesView's openName
  // that seeds the modal's single-name caps query from the batch result the
  // table already has, and (2) a loading gate in the modal that suppresses
  // the raw-phase fallback until caps has settled. Together, the modal must
  // open already displaying the same task-state label as the row.
  function routeOpening(cmd: string, args?: Record<string, unknown>) {
    if (cmd === "get_names_action_capabilities") {
      const names = (args as { names?: string[] })?.names ?? [];
      return Promise.resolve(names.map((n) => CAPS_BY_NAME[n] ?? baseCaps(n, {})));
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
      case "list_watchlist":
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "read_names":
        return Promise.resolve([
          {
            name: "openingname",
            state: "OPENING",
            height: 100,
            renewal: 200,
            owner: null,
            stats: null,
          },
        ]);
      case "read_name_info":
        // Modal's own phase-fallback source. Deliberately the raw OPENING
        // state — if the modal ever fell back to `badge.label`, it would show
        // "Opening", the exact divergence this test guards against.
        return Promise.resolve({
          name: (args as { name?: string })?.name ?? "openingname",
          state: "OPENING",
          height: 100,
          renewal: 200,
          owner: null,
          stats: null,
        });
      default:
        return Promise.resolve(null);
    }
  }

  it("the name page opens on the row's task state, never the raw phase", async () => {
    invokeMock.mockImplementation(routeOpening);
    render(
      <>
        <Routes>
          <Route path="/names" element={<NamesView />} />
          <Route path="/names/:name" element={<NameRoute />} />
        </Routes>
        <LocationProbe />
      </>,
      { wrapper: wrapper() },
    );

    // The row shows the capability-derived status (nameRows wording for
    // waitingForBidding: "Opening · bidding opens in …").
    const row = await screen.findByTestId("names-row");
    await waitFor(() => expect(row).toHaveTextContent("bidding opens in 20 blocks (~3h)"));

    // Open the name page by clicking the row (a waitingForBidding row has no
    // button of its own).
    await act(async () => {
      fireEvent.click(row);
    });

    // The page's status line (data-testid="name-phase") must tell the same
    // task state — never the raw "Opening" phase label, never the transient
    // "Checking…" placeholder (the cache bridge seeds caps on open).
    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent(/^\/names\//));
    const phase = await screen.findByTestId("name-phase");
    await waitFor(() => {
      expect(phase).toHaveTextContent(/^Auction opening/);
    });
    expect(phase).not.toHaveTextContent(/Opening/);
    expect(phase).not.toHaveTextContent(/Checking/);
    expect(invokeMock.mock.calls.some((c) => c[0] === "get_name_action_capabilities")).toBe(false);
  });
});
