import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// Auction-UX behaviours added after the live auction GUI test:
//   * a broadcast tx visibly settles Pending → Confirmed (and Not confirmed);
//   * "Locked in Auctions" balance is surfaced when a bid lockup exists;
//   * a reveal-required alert appears for names in the REVEAL phase;
//   * the name modal shows the live phase + countdown and the DNS row editor
//     serializes to the record array the build_*_draft commands expect.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";
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

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/wallet"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("WalletView — auction UX", () => {
  /** Minimal capability object — override per test via `opts.caps[name]`. */
  function fallbackCaps(name: string): Record<string, unknown> {
    return {
      name,
      phase: "UNKNOWN",
      taskState: "unavailableOther",
      ownsName: false,
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
    };
  }

  function routeWallet(opts: {
    drafts?: unknown[];
    names?: unknown[];
    lockupDoos?: number;
    caps?: Record<string, Record<string, unknown>>;
  }) {
    return (cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "get_wallet_balances":
          return Promise.resolve({
            liquidDoos: 5_000_000,
            nameControlDoos: 0,
            nameLockupDoos: opts.lockupDoos ?? 0,
            totalDoos: 5_000_000 + (opts.lockupDoos ?? 0),
          });
        case "read_balance":
          return Promise.resolve({ confirmed: 0, unconfirmed: 0, locked_confirmed: 0, locked_unconfirmed: 0 });
        case "list_tx_drafts":
          return Promise.resolve(opts.drafts ?? []);
        case "read_names":
          return Promise.resolve(opts.names ?? []);
        case "get_names_action_capabilities": {
          const requested = (args as { names?: string[] })?.names ?? [];
          return Promise.resolve(
            requested.map((n) => opts.caps?.[n] ?? fallbackCaps(n)),
          );
        }
        default:
          return Promise.resolve(null);
      }
    };
  }

  const draft = (over: Record<string, unknown>) => ({
    id: "d1",
    walletProfileId: "p1",
    action: "send_hns",
    status: "broadcasted",
    summary: { action: "send_hns", sendTotalDoos: 1_000_000, feeDoos: 1410 },
    errorMessage: null,
    txid: "abcdef0123456789",
    confirmationHeight: null,
    createdAt: "2026-01-01",
    ...over,
  });

  it("renders Pending / Confirmed / Not confirmed for tx statuses", async () => {
    invokeMock.mockImplementation(
      routeWallet({
        drafts: [
          draft({ id: "a", txid: "aaa0000000000001", status: "confirmed", confirmationHeight: 437 }),
          draft({ id: "b", txid: "bbb0000000000002", status: "broadcasted" }),
          draft({ id: "c", txid: "ccc0000000000003", status: "dropped", errorMessage: "never confirmed" }),
        ],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    // The confirmed draft renders its status badge ("Confirmed") and its block
    // height ("#437", now in its own Block column) in the same row.
    const blockCell = await screen.findByText("#437");
    const confirmedRow = blockCell.closest("tr")!;
    expect(within(confirmedRow).getByText("Confirmed")).toBeInTheDocument();
    expect(screen.getByText("Pending")).toBeInTheDocument();
    expect(screen.getByText("Not confirmed")).toBeInTheDocument();
  });

  it("shows the Locked in Auctions balance only when a lockup exists", async () => {
    invokeMock.mockImplementation(routeWallet({ lockupDoos: 2_000_000 }));
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    const card = await screen.findByTestId("balance-locked-auctions");
    expect(card).toHaveTextContent("Locked in Auctions");
    expect(card).toHaveTextContent("2.000000");
  });

  it("hides the Locked in Auctions card when there is no lockup", async () => {
    invokeMock.mockImplementation(routeWallet({ lockupDoos: 0 }));
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    expect(screen.queryByTestId("balance-locked-auctions")).toBeNull();
  });

  it("raises a reveal-required alert for names the capability model marks readyToReveal", async () => {
    invokeMock.mockImplementation(
      routeWallet({
        names: [{ name: "examplename", state: "REVEAL", height: 1, renewal: 2, owner: { hash: "t", index: 0 }, stats: null }],
        caps: {
          examplename: {
            ...fallbackCaps("examplename"),
            phase: "REVEAL",
            taskState: "readyToReveal",
            hasBidCommitment: true,
            countdownLabel: "Auction closes in",
            countdownBlocks: 12,
            countdownHours: 2,
          },
        },
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    const alert = await screen.findByTestId("reveal-alert");
    expect(alert).toHaveTextContent(/Action required: reveal/i);
    expect(alert).toHaveTextContent(".examplename");
    // Countdown fragment included honestly (F2 fix).
    expect(alert).toHaveTextContent(/12 blocks/i);
  });

  it("does NOT raise a reveal alert for a name in REVEAL phase this wallet never bid on", async () => {
    // Old bug (F2): the banner used to filter by raw phase alone, so ANY
    // REVEAL-phase name — even one this wallet has no bid on — triggered the
    // alert. The capability model must be the only source of truth.
    invokeMock.mockImplementation(
      routeWallet({
        names: [{ name: "notmine", state: "REVEAL", height: 1, renewal: 2, owner: null, stats: null }],
        caps: {
          notmine: {
            ...fallbackCaps("notmine"),
            phase: "REVEAL",
            taskState: "unavailableOther",
          },
        },
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    expect(screen.queryByTestId("reveal-alert")).toBeNull();
  });

  it("does NOT raise a lost/redeem alert for a CLOSED-no-owner name without our reveal coin", async () => {
    // F2 fix: a CLOSED name with owner:null used to ALWAYS show "you lost,
    // redeem" even when this wallet never placed a bid. Only a genuine
    // lostNeedsRedeem capability (we hold a redeemable reveal coin) may
    // trigger the banner.
    invokeMock.mockImplementation(
      routeWallet({
        names: [{ name: "neverbidon", state: "CLOSED", height: 1, renewal: 2, owner: null, stats: null }],
        caps: {
          neverbidon: {
            ...fallbackCaps("neverbidon"),
            phase: "CLOSED",
            taskState: "unavailableOther",
          },
        },
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    expect(screen.queryByTestId("redeem-alert")).toBeNull();
  });

  it("shows a degraded notice when the batch capabilities query persistently errors (Task 12 review / Task 14)", async () => {
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "get_names_action_capabilities") {
        return Promise.reject(new Error("db locked"));
      }
      return routeWallet({
        names: [{ name: "examplename", state: "REVEAL", height: 1, renewal: 2, owner: null, stats: null }],
      })(cmd, args);
    });
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");

    const notice = await screen.findByTestId("urgent-tasks-degraded");
    expect(notice).toHaveTextContent(/Couldn't verify urgent auction tasks/i);
    // No false "everything is fine" — no urgency banner shows either.
    expect(screen.queryByTestId("reveal-alert")).toBeNull();
  });

  it("shows no degraded notice when the batch capabilities query succeeds", async () => {
    invokeMock.mockImplementation(
      routeWallet({
        names: [{ name: "examplename", state: "OPEN", height: 1, renewal: 2, owner: null, stats: null }],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    // Give the (successful) capabilities query a tick to settle.
    await waitFor(() => expect(invokeMock.mock.calls.some((c) => c[0] === "get_names_action_capabilities")).toBe(true));
    expect(screen.queryByTestId("urgent-tasks-degraded")).toBeNull();
  });

  it("raises the redeem alert only when the capability model says lostNeedsRedeem", async () => {
    invokeMock.mockImplementation(
      routeWallet({
        names: [{ name: "lostbid", state: "CLOSED", height: 1, renewal: 2, owner: null, stats: null }],
        caps: {
          lostbid: {
            ...fallbackCaps("lostbid"),
            phase: "CLOSED",
            taskState: "lostNeedsRedeem",
            hasRevealCoin: true,
          },
        },
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    const alert = await screen.findByTestId("redeem-alert");
    expect(alert).toHaveTextContent(/Lost bid/i);
    expect(alert).toHaveTextContent(".lostbid");
  });
});

describe("NameActionsModal — phase header + DNS editor", () => {
  function routeModal(captured: { records?: unknown }) {
    return (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "get_name_action_capabilities") {
        return Promise.resolve({
          name: "cuatesttld",
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
          nextActionReason: null,
          countdownLabel: null,
          countdownBlocks: null,
          countdownHours: null,
        });
      }
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "read_name_info":
          return Promise.resolve({
            name: "cuatesttld",
            state: "CLOSED",
            height: 100,
            renewal: 200,
            owner: { hash: profile.receiveAddress, index: 0 },
            value: 1_000_000,
            highest: 2_000_000,
            registered: false,
            stats: { blocksUntilExpire: 100 },
          });
        case "build_register_draft":
          captured.records = args?.records;
          return Promise.resolve({ id: "reg1", status: "draft" });
        case "sign_tx_draft":
          return Promise.resolve({ id: "reg1", status: "signed" });
        case "broadcast_tx_draft":
          return Promise.resolve({ draftId: "reg1", txid: "f".repeat(64), status: "broadcasted" });
        default:
          return Promise.resolve(null);
      }
    };
  }

  it("shows the live phase badge and the recommended action", async () => {
    invokeMock.mockImplementation(routeModal({}));
    render(<NameActionsModal name="cuatesttld" open onClose={() => {}} />, { wrapper: wrapper() });

    // With capabilities/task-state for wonNeedsRegister, the badge shows "Won!".
    expect(await screen.findByTestId("name-phase")).toBeInTheDocument();
    // The guided action should show Register.
    expect(await screen.findByRole("button", { name: /^Register$/i })).toBeInTheDocument();
  });

  it("serializes the DNS row editor into the records array on Register", async () => {
    const captured: { records?: unknown } = {};
    invokeMock.mockImplementation(routeModal(captured));
    render(<NameActionsModal name="cuatesttld" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByTestId("dns-rows");
    // Default first row is a TXT — fill its value, then Register.
    fireEvent.change(screen.getByLabelText("record value"), {
      target: { value: "cua-agent-verified" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Register$/i }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).toContain("build_register_draft");
    });
    expect(captured.records).toEqual([{ type: "TXT", txt: ["cua-agent-verified"] }]);
  });
});

describe("NameActionsModal — recover bid commitment (Task 2 / C2)", () => {
  function routeReveal(
    captured: { recover?: Record<string, unknown> },
    hasBidCommitment: boolean,
  ) {
    return (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "get_name_action_capabilities") {
        return Promise.resolve({
          name: "lostbidname",
          phase: "REVEAL",
          taskState: "unavailableOther",
          ownsName: false,
          hasBidCommitment,
          hasRevealCoin: false,
          hasOwnerCoin: false,
          canOpen: { allowed: false, reason: null },
          canBid: { allowed: false, reason: "bidding is not open (phase: 'REVEAL')" },
          canReveal: {
            allowed: false,
            reason: hasBidCommitment
              ? "no unspent reveal coin found (sync first?)"
              : "no bid commitment found for this name",
          },
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
        });
      }
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: profile.id, unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "read_name_info":
          return Promise.resolve({
            name: "lostbidname",
            state: "REVEAL",
            height: 100,
            renewal: 200,
            owner: null,
            value: null,
            highest: null,
            registered: false,
            stats: {},
          });
        case "recover_bid_commitment":
          captured.recover = args;
          return Promise.resolve({
            name: "lostbidname",
            address: "rs1qrecovered",
            bidValueDoos: 10_000_000,
            lockupValueDoos: 20_000_000,
          });
        default:
          return Promise.resolve(null);
      }
    };
  }

  it("renders the recover-bid input in the no-commitment REVEAL state and calls recover_bid_commitment with the raw name + doos value", async () => {
    const captured: { recover?: Record<string, unknown> } = {};
    invokeMock.mockImplementation(routeReveal(captured, false));
    render(<NameActionsModal name="lostbidname" open onClose={() => {}} />, { wrapper: wrapper() });

    const recoverBox = await screen.findByTestId("recover-bid");
    expect(recoverBox).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Your bid amount (HNS)"), {
      target: { value: "10" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Recover bid$/i }));

    await waitFor(() => {
      expect(captured.recover).toBeTruthy();
    });
    // The raw (ASCII) name is passed through untouched, and the HNS input is
    // converted to doos before the backend call.
    expect(captured.recover?.name).toBe("lostbidname");
    expect(captured.recover?.bidValueDoos).toBe(10_000_000);
  });

  it("does not render the recover-bid input once a commitment already exists", async () => {
    invokeMock.mockImplementation(routeReveal({}, true));
    render(<NameActionsModal name="lostbidname" open onClose={() => {}} />, { wrapper: wrapper() });

    await screen.findByTestId("name-phase");
    expect(screen.queryByTestId("recover-bid")).toBeNull();
  });
});
