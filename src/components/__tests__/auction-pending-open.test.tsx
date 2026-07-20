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

/** A pending-open tx draft — as `list_tx_drafts` would return right after
 *  a build_open_draft → sign → broadcast, before the name is tracked. */
function openDraft(name: string, status: string, overrides: Record<string, unknown> = {}) {
  return {
    id: `draft-${name}`,
    walletProfileId: "p1",
    action: "open",
    status,
    summary: {
      action: "open",
      sendTotalDoos: 0,
      feeDoos: 1000,
      changeDoos: 0,
      inputTotalDoos: 1000,
      numInputs: 1,
      recipientAddress: null,
      txid: null,
      warnings: [],
      name,
    },
    errorMessage: null,
    txid: "abc123txid",
    confirmationHeight: null,
    createdAt: "2026-01-01",
    ...overrides,
  };
}

function baseRoutes(o: {
  drafts?: unknown[];
  names?: unknown[];
  caps?: (names: string[]) => unknown[];
}) {
  return (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "get_names_action_capabilities") {
      const names = (args as { names?: string[] })?.names ?? [];
      return Promise.resolve(o.caps ? o.caps(names) : []);
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
        return Promise.resolve(o.drafts ?? []);
      case "refresh_tx_confirmations":
        return Promise.resolve(null);
      case "read_name_info":
        return Promise.resolve(null);
      case "get_name_action_capabilities":
        return Promise.resolve({
          name: (args as { name?: string })?.name,
          phase: "OPENING",
          taskState: "waitingForBidding",
          ownsName: false,
          hasBidCommitment: false,
          hasRevealCoin: false,
          hasOwnerCoin: false,
          canOpen: {
            allowed: false,
            reason: "an auction is already opening for this name (pending confirmation)",
          },
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
          nextActionKey: "NONE",
          nextActionLabel: "Wait for Bidding",
          nextActionReason: null,
          countdownLabel: null,
          countdownBlocks: null,
          countdownHours: null,
        });
      default:
        return Promise.resolve(null);
    }
  };
}

describe("AuctionsView — pending-open synthetic rows (Task 2)", () => {
  it("shows a just-broadcast OPEN draft as an 'Opening — pending confirmation' row", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        drafts: [openDraft("freshname", "broadcasted")],
        names: [],
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    expect(await screen.findByText(/Opening — pending confirmation/i)).toBeInTheDocument();
    expect(screen.getByText(".freshname")).toBeInTheDocument();
    // Folded into the Active Auctions count — not the empty state.
    expect(screen.getByText(/Active Auctions \(1\)/i)).toBeInTheDocument();
    expect(screen.queryByText(/No active auctions/i)).not.toBeInTheDocument();
  });

  it("does NOT show a pending-open row for a draft status of plain 'draft' (unsigned/unbroadcast)", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        drafts: [openDraft("unsignedname", "draft")],
        names: [],
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/No active auctions/i);
    expect(screen.queryByText(/Opening — pending confirmation/i)).not.toBeInTheDocument();
  });

  it("dedups against a name already tracked (e.g. now OPENING in read_names) — no duplicate row", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        drafts: [openDraft("trackedname", "broadcasted")],
        names: [
          {
            name: "trackedname",
            state: "OPENING",
            height: 100,
            renewal: 200,
            owner: null,
            stats: null,
          },
        ],
        caps: (names) =>
          names.map((n) => ({
            name: n,
            phase: "OPENING",
            taskState: "waitingForBidding",
            ownsName: false,
            hasBidCommitment: false,
            hasRevealCoin: false,
            hasOwnerCoin: false,
            canOpen: { allowed: false, reason: "Phase is OPENING" },
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
            nextActionKey: "NONE",
            nextActionLabel: "Wait for Bidding",
            nextActionReason: null,
            countdownLabel: null,
            countdownBlocks: null,
            countdownHours: null,
          })),
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    // The real tracked row renders (Wait for Bidding is its View label).
    // `useReadNames` and `useTxDrafts` are two independent queries that can
    // settle a render apart; wait for the STEADY state (both settled) rather
    // than the first render where `.trackedname` appears, which could still
    // be the synthetic row if drafts resolved a tick ahead of names.
    await waitFor(() => {
      expect(screen.getByText(/Active Auctions \(1\)/i)).toBeInTheDocument();
      expect(screen.queryByText(/Opening — pending confirmation/i)).not.toBeInTheDocument();
    });
    // Only ONE row for the name — no synthetic duplicate.
    expect(screen.getAllByText(".trackedname").length).toBe(1);
  });

  it("clicking View on a synthetic pending-open row opens the modal with the RAW name", async () => {
    invokeMock.mockImplementation(
      baseRoutes({
        drafts: [openDraft("xn--rawname", "signed")],
        names: [],
      }),
    );
    render(<AuctionsView />, { wrapper: wrapper() });

    await screen.findByText(/Opening — pending confirmation/i);
    fireEvent.click(screen.getByRole("button", { name: /^View$/i }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "read_name_info");
      expect(call?.[1]).toEqual({ name: "xn--rawname" });
    });
    const capsCall = invokeMock.mock.calls.find(
      (c) => c[0] === "get_name_action_capabilities",
    );
    expect(capsCall?.[1]).toMatchObject({ name: "xn--rawname" });
  });
});
