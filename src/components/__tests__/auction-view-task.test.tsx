import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
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
    taskState: "availableToOpen",
    ownsName: false,
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
};

function routeInvoke() {
  return (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "get_names_action_capabilities") {
      const names = (args as { names?: string[] })?.names ?? [];
      return Promise.resolve(
        names.map((n) => CAPS_BY_NAME[n] ?? baseCaps(n, {})),
      );
    }
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
      case "get_write_capability":
        return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
      case "read_names":
        return Promise.resolve([
          { name: "revealname", state: "REVEAL", height: 100, renewal: 200, owner: null, stats: null },
          // registered: false explicitly (genuine "won, not yet registered" /
          // "lost, not yet redeemed" cases) — required for the CLOSED-phase
          // gate in AuctionsView, which only admits an explicit `false`.
          { name: "wonname", state: "CLOSED", height: 100, renewal: 200, owner: { hash: "abc", index: 0 }, stats: null, registered: false },
          { name: "lostname", state: "CLOSED", height: 100, renewal: 200, owner: { hash: "abc", index: 0 }, stats: null, registered: false },
        ]);
      default:
        return Promise.resolve(null);
    }
  };
}

describe("AuctionsView — task-driven row rendering", () => {
  it("shows task-state labels for won / lost / reveal names", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<AuctionsView />, { wrapper: wrapper() });

    // Wait for capability-derived labels using getAllByText with await
    expect(await screen.findByText(/Won — Register Now/i)).toBeInTheDocument();
    expect(await screen.findByText(/Lost — Redeem Now/i)).toBeInTheDocument();
    expect(await screen.findByText(/Ready to Reveal/i)).toBeInTheDocument();
    // The count reflects the actual visible rows
    expect(screen.getByText(/Active Auctions \(3\)/i)).toBeInTheDocument();
  });

  it("fetches capabilities with ONE batch invoke, not one per row", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/Won — Register Now/i);

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
    expect(
      invokeMock.mock.calls.some((c) => c[0] === "get_name_action_capabilities"),
    ).toBe(false);
  });

  it("sorts rows by urgency: readyToReveal, then wonNeedsRegister, then lostNeedsRedeem", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/Won — Register Now/i);
    const rows = screen.getAllByRole("row").slice(1); // drop the header row
    const rowText = rows.map((r) => r.textContent ?? "");
    const revealIdx = rowText.findIndex((t) => /Ready to Reveal/i.test(t));
    const wonIdx = rowText.findIndex((t) => /Won — Register Now/i.test(t));
    const lostIdx = rowText.findIndex((t) => /Lost — Redeem Now/i.test(t));
    expect(revealIdx).toBeGreaterThanOrEqual(0);
    expect(wonIdx).toBeGreaterThan(revealIdx);
    expect(lostIdx).toBeGreaterThan(wonIdx);
  });

  it("renders a countdown in the countdown column instead of raw height", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/Ready to Reveal/i);
    expect(screen.getByText(/Countdown/i)).toBeInTheDocument();
    expect(screen.queryByText(/^Height$/i)).not.toBeInTheDocument();
    // revealname has countdownBlocks: 12, countdownHours: 2.
    expect(screen.getByText(/12 blocks/i)).toBeInTheDocument();
    // Names without countdown data fall back to an em dash.
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("excludes CLOSED names with unknown registered status, includes explicit registered:false", async () => {
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
              : // Not expected to be requested for the owned/unknown-registered
                // name below since it must be filtered out before it's ever
                // added to the batch — but return something sane if it is.
                baseCaps(n, {
                  phase: "CLOSED",
                  taskState: "ownedNoAction",
                  ownsName: true,
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
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "read_names":
          return Promise.resolve([
            // Genuinely-owned name whose `registered` flag is UNKNOWN (the field
            // is omitted entirely — this is the real reproduction of the bug:
            // explorer-sourced or otherwise-incomplete data must NOT be treated
            // as "still needs a task"). Must be excluded from Active Auctions.
            {
              name: "unknownregisteredname",
              state: "CLOSED",
              height: 100,
              renewal: 200,
              owner: { hash: "abc", index: 0 },
              stats: null,
            },
            // Genuine "won, not yet registered" case — `registered` is
            // EXPLICITLY false. Must remain included in Active Auctions.
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

    render(<AuctionsView />, { wrapper: wrapper() });

    // The genuine "won, needs register" name shows up as actionable.
    expect(await screen.findByText(/Won — Register Now/i)).toBeInTheDocument();

    // The owned name with unknown `registered` status must NOT leak into
    // Active Auctions, and the count must reflect only the real task.
    expect(screen.queryByText(/unknownregisteredname/i)).not.toBeInTheDocument();
    expect(screen.getByText(/Active Auctions \(1\)/i)).toBeInTheDocument();

    // And it must never even be sent in the batch request.
    const batchCall = invokeMock.mock.calls.find((c) => c[0] === "get_names_action_capabilities");
    expect((batchCall?.[1] as { names?: string[] })?.names).toEqual(["wonneedsregister"]);
  });
});
