import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

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
// The app copies via ../../lib/clipboard (which falls back to
// navigator.clipboard under jsdom); mock it so we can assert the exact text.
const clipboardWriteMock = vi.fn().mockResolvedValue(undefined);
vi.mock("../../lib/clipboard", () => ({
  writeText: (...args: unknown[]) => clipboardWriteMock(...args),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";
import type { SyncStatus } from "../../queries/sync";

const baseProfile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE000000000000",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "rs1qexamplereceiveaddr",
  lastSyncedHeight: 10,
  lastSyncedAt: null as string | null,
  lastExplorerSyncAt: null as string | null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

type Overrides = {
  profile?: Partial<typeof baseProfile>;
  profiles?: unknown[];
  unlocked?: boolean;
  canWrite?: boolean;
  draft?: unknown;
  spendableDoos?: number;
  confirmedDoos?: number;
  renewals?: unknown;
  drafts?: unknown[];
  names?: unknown[];
  history?: unknown[];
};

function routeInvoke(o: Overrides = {}) {
  const profile = { ...baseProfile, ...(o.profile ?? {}) };
  const unlocked = o.unlocked ?? false;
  const canWrite = o.canWrite ?? false;
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve(o.profiles ?? [profile]);
      case "get_signer_session":
        return Promise.resolve({
          walletProfileId: unlocked ? profile.id : null,
          unlocked,
          unlockedUntilEpochMs: unlocked ? Date.now() + 60000 : 0,
        });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: unlocked,
          broadcasterAvailable: true,
          canWrite,
          reason: canWrite ? null : "Unlock your wallet to sign transactions.",
        });
      case "get_wallet_balances":
        return Promise.resolve({
          liquidDoos: o.spendableDoos ?? 5_000_000,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          totalDoos: o.spendableDoos ?? 5_000_000,
        });
      case "read_balance":
        return Promise.resolve({
          confirmed: o.confirmedDoos ?? 0,
          unconfirmed: 0,
          locked_confirmed: 0,
          locked_unconfirmed: 0,
        });
      case "list_tx_drafts":
        return Promise.resolve(o.drafts ?? []);
      case "read_action_history":
        return Promise.resolve(o.history ?? []);
      case "refresh_tx_confirmations":
        return Promise.resolve(null);
      case "read_renewals":
        return Promise.resolve(
          o.renewals ?? {
            walletProfileId: profile.id,
            currentHeight: null,
            heightSource: "unknown",
            expiringSoonThresholdDays: 30,
            names: [],
          },
        );
      case "read_names":
        return Promise.resolve(
          o.names ?? [
            { name: "example", state: "CLOSED", height: 100, renewal: 200, owner: { hash: "tx1", index: 0 }, stats: null },
          ],
        );
      case "build_send_hns_draft":
        return Promise.resolve(
          o.draft ?? {
            id: "d1",
            walletProfileId: profile.id,
            action: "send_hns",
            status: "draft",
            summary: {
              action: "send_hns",
              sendTotalDoos: 1_000_000,
              feeDoos: 1410,
              changeDoos: 3_998_590,
              inputTotalDoos: 5_000_000,
              numInputs: 1,
              recipientAddress: "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2",
              txid: null,
              warnings: [],
            },
            errorMessage: null,
            txid: null,
            createdAt: "2026-01-01",
          },
        );
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/wallet"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  clipboardWriteMock.mockClear();
});

describe("WalletView (non-custodial)", () => {
  it("shows a locked signer and an Unlock control, with NO secret inputs in the DOM", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false, canWrite: false }));
    const { container } = render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Signer locked/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Unlock/i })).toBeInTheDocument();

    // The core guarantee: React never renders a password/secret input field.
    expect(container.querySelector('input[type="password"]')).toBeNull();
    // And no mnemonic entry surface exists.
    expect(container.querySelector("textarea")).toBeNull();
  });

  it("the Unlock button delegates to the secure unlock command", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Unlock/i }));

    // Unlocking must go through the secure command (which prompts in the Rust
    // secure window) — never a React-side passphrase path.
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "unlock_local_signer");
      expect(call?.[1]).toEqual({ walletProfileId: "p1" });
    });
  });

  it("a no-passphrase wallet shows one-click unlock copy (no passphrase prompt mention)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { hasPassphrase: false }, unlocked: false }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Signer locked/i)).toBeInTheDocument();
    expect(screen.getByText(/no passphrase.*click Unlock/i)).toBeInTheDocument();
    expect(screen.queryByText(/Unlock with your passphrase/i)).toBeNull();
    expect(screen.getByRole("button", { name: /Unlock/i })).toBeInTheDocument();
  });

  it("a passphrase wallet still shows the secure-window unlock copy", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { hasPassphrase: true }, unlocked: false }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Unlock with your passphrase \(in a secure window\)/i)).toBeInTheDocument();
  });

  it("send dialog collects only address + amount (no passphrase field)", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: true, canWrite: true }));
    const { container } = render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));

    expect(screen.getByText(/Destination Address/i)).toBeInTheDocument();
    expect(screen.getByText(/Amount \(HNS\)/i)).toBeInTheDocument();
    // No passphrase/secret input anywhere in the send flow.
    expect(container.querySelector('input[type="password"]')).toBeNull();
  });

  it("building a draft shows a fee/change preview before broadcast", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: true, canWrite: true }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    fireEvent.change(screen.getByPlaceholderText(/rs1q/i), {
      target: { value: "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2" },
    });
    fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /Review/i }));

    await waitFor(() =>
      expect(screen.getByText(/Sign & Broadcast/i)).toBeInTheDocument(),
    );
    expect(screen.getByText(/Fee/i)).toBeInTheDocument();
    expect(screen.getByText(/Change/i)).toBeInTheDocument();
  });

  it("shows a 'connect & sync a node' hint when explorer balance > 0 but spendable is 0", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ unlocked: true, canWrite: true, spendableDoos: 0, confirmedDoos: 1_400_000 }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(await screen.findByTestId("needs-node-sync")).toBeInTheDocument();
    // Can't send with nothing synced, even though the signer/node are ready.
    expect(screen.getByRole("button", { name: /Send HNS/i })).toBeDisabled();
  });

  it("renders Owned Names from the cache-backed read_names command", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(await screen.findByText(/Owned Names/i)).toBeInTheDocument();
    expect(await screen.findByText(/\.example/)).toBeInTheDocument();
  });

  it("Recent transactions: a covenant UPDATE shows net Amount 0 (name value carried, not spent), a send shows its amount", async () => {
    const drafts = [
      {
        id: "u1",
        walletProfileId: baseProfile.id,
        action: "update",
        status: "broadcasted",
        summary: {
          action: "update",
          // 222 HNS — the name's locked value, re-homed to your OWN new coin.
          sendTotalDoos: 222_000_000,
          feeDoos: 2620,
          changeDoos: 0,
          inputTotalDoos: 222_100_000,
          numInputs: 2,
          recipientAddress: null,
          txid: null,
          warnings: [],
          name: "ecology",
        },
        errorMessage: null,
        txid: null,
        createdAt: "2026-07-22",
      },
      {
        id: "s1",
        walletProfileId: baseProfile.id,
        action: "send_hns",
        status: "broadcasted",
        summary: {
          action: "send_hns",
          sendTotalDoos: 1_000_000,
          feeDoos: 1410,
          changeDoos: 0,
          inputTotalDoos: 1_001_410,
          numInputs: 1,
          recipientAddress: "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2",
          txid: null,
          warnings: [],
          name: null,
        },
        errorMessage: null,
        txid: null,
        createdAt: "2026-07-22",
      },
    ];
    invokeMock.mockImplementation(routeInvoke({ unlocked: false, drafts }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    // The UPDATE row must NOT present the 222 HNS name value as a cost...
    const updateRow = (await screen.findByText(/update · \.ecology/)).closest("tr")!;
    expect(screen.queryByText("222.000000")).not.toBeInTheDocument();
    // ...it shows net 0 in Amount, with an explanatory tooltip about the
    // carried value.
    expect(within(updateRow).getByText("0.000000")).toBeInTheDocument();
    expect(
      within(updateRow).getByTitle(
        /Name value 222\.000000 HNS is carried to your own new coin/i,
      ),
    ).toBeInTheDocument();
    // A real send still shows its outgoing amount.
    const sendRow = screen.getByText("send_hns").closest("tr")!;
    expect(within(sendRow).getByText("1.000000")).toBeInTheDocument();
  });

  const multiNames = [
    { name: "example", state: "CLOSED", height: 100, renewal: 200, owner: { hash: "tx1", index: 0 }, stats: null },
    { name: "another", state: "CLOSED", height: 101, renewal: 201, owner: { hash: "tx2", index: 0 }, stats: null },
    // "козёл" (Russian for "goat") — its ACE/raw form is xn--g1afek0h.
    { name: "xn--g1afek0h", state: "CLOSED", height: 102, renewal: 202, owner: { hash: "tx3", index: 0 }, stats: null },
  ];

  it("Owned Names filter: narrows by ASCII substring, unicode substring, and clears back to full list", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: false, names: multiNames }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    // All three rows present up front, including the decoded unicode name.
    expect(await screen.findByText(/\.example/)).toBeInTheDocument();
    expect(screen.getByText(/\.another/)).toBeInTheDocument();
    expect(screen.getByText(/\.козёл/)).toBeInTheDocument();
    expect(screen.getByText("Owned Names (3)")).toBeInTheDocument();

    const filterInput = screen.getByPlaceholderText(/filter/i);

    // ASCII substring narrows to the matching raw name.
    fireEvent.change(filterInput, { target: { value: "exam" } });
    expect(screen.getByText(/\.example/)).toBeInTheDocument();
    expect(screen.queryByText(/\.another/)).toBeNull();
    expect(screen.queryByText(/\.козёл/)).toBeNull();
    expect(screen.getByText("Owned Names (1 of 3)")).toBeInTheDocument();

    // Unicode substring that only appears in the decoded displayName still
    // finds the underlying xn-- row.
    fireEvent.change(filterInput, { target: { value: "коз" } });
    expect(screen.getByText(/\.козёл/)).toBeInTheDocument();
    expect(screen.queryByText(/\.example/)).toBeNull();
    expect(screen.queryByText(/\.another/)).toBeNull();
    expect(screen.getByText("Owned Names (1 of 3)")).toBeInTheDocument();

    // A query matching nothing shows a message instead of an empty table.
    fireEvent.change(filterInput, { target: { value: "zzzznotfound" } });
    expect(screen.getByText(/No names match/i)).toBeInTheDocument();
    expect(screen.getByText("Owned Names (0 of 3)")).toBeInTheDocument();

    // Clearing the input restores the full list.
    fireEvent.change(filterInput, { target: { value: "" } });
    expect(screen.getByText(/\.example/)).toBeInTheDocument();
    expect(screen.getByText(/\.another/)).toBeInTheDocument();
    expect(screen.getByText(/\.козёл/)).toBeInTheDocument();
    expect(screen.getByText("Owned Names (3)")).toBeInTheDocument();
  });

  it("watch-only profiles hide spend + unlock controls", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { watchOnly: true, kind: "watch_only_xpub" } }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Watch-only/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Send HNS/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /Unlock/i })).toBeNull();
  });
});

const secondProfile = {
  ...baseProfile,
  id: "p2",
  label: "Trading",
  receiveAddress: "rs1qsecondaddr",
  active: false,
};

describe("WalletView — last successful sync timestamp (Task 11 / S1)", () => {
  it("shows a dash when the profile has never synced", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { lastSyncedAt: null, lastExplorerSyncAt: null } }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByText(/Last successful sync:\s*—/)).toBeInTheDocument();
  });

  it("shows a formatted timestamp once the profile has synced", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profile: { lastSyncedAt: "2026-07-10 12:00:00" } }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    // formatDate() renders the naive-UTC sqlite timestamp in the user's
    // locale — just prove it's no longer the "never synced" dash.
    expect(screen.queryByText(/Last successful sync:\s*—/)).toBeNull();
    expect(screen.getByText(/Last successful sync:/)).toBeInTheDocument();
  });

  // Finding 2 (review fix): explorer-only mode never advances `lastSyncedAt`
  // (only the node-RPC step does) — `lastExplorerSyncAt` is the only
  // freshness signal that moves there, so it alone must be enough to clear
  // the dash.
  it("shows a formatted timestamp from lastExplorerSyncAt alone (explorer-only mode, no node sync)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        profile: { lastSyncedAt: null, lastExplorerSyncAt: "2026-07-14 09:00:00" },
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.queryByText(/Last successful sync:\s*—/)).toBeNull();
    expect(screen.getByText(/Last successful sync:/)).toBeInTheDocument();
  });

  // The line shows ONE timestamp — whichever sync path most recently
  // completed — rather than two separately-labeled fields (see
  // `latestTimestamp` in lib/utils.ts). Prove the newer of the two wins by
  // making the explorer timestamp older than the node one and checking the
  // rendered text matches the node timestamp's formatting, not the
  // explorer's.
  it("prefers the newer of lastSyncedAt / lastExplorerSyncAt when both are set", async () => {
    const older = "2026-01-01 00:00:00";
    const newer = "2026-07-14 09:00:00";
    invokeMock.mockImplementation(
      routeInvoke({ profile: { lastSyncedAt: newer, lastExplorerSyncAt: older } }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const expected = new Date(`${newer.replace(" ", "T")}Z`).toLocaleString();
    expect(screen.getByText(new RegExp(`Last successful sync:\\s*${expected.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`))).toBeInTheDocument();
  });
});

describe("WalletView multi-wallet management", () => {
  it("shows Add wallet + Manage entry points with an active profile", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByRole("button", { name: /\+ Add wallet/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Manage wallets/i })).toBeInTheDocument();
  });

  it("Manage opens a modal listing all wallets; Switch activates a non-active one", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ profiles: [{ ...baseProfile, active: true }, secondProfile] }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Manage wallets/i }));

    // Both wallets listed in the dialog.
    expect(await screen.findByText("Trading")).toBeInTheDocument();
    // The non-active wallet (Trading) exposes a Switch action.
    fireEvent.click(screen.getByRole("button", { name: /^Switch$/i }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "set_active_wallet_profile");
      expect(call?.[1]).toEqual({ walletProfileId: "p2" });
    });
  });

  it("deleting the active wallet auto-switches to a remaining one", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    invokeMock.mockImplementation(
      routeInvoke({ profiles: [{ ...baseProfile, active: true }, secondProfile] }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Manage wallets/i }));
    await screen.findByText("Trading");

    // Delete the active wallet (the first Delete button = Primary's row).
    fireEvent.click(screen.getAllByRole("button", { name: /^Delete$/i })[0]!);
    await waitFor(() => {
      expect(invokeMock.mock.calls.find((c) => c[0] === "delete_wallet_profile")?.[1]).toEqual({
        walletProfileId: "p1",
      });
    });
    // …then re-activates the remaining wallet (p2).
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.find((c) => c[0] === "set_active_wallet_profile")?.[1],
      ).toEqual({ walletProfileId: "p2" });
    });
  });

  it("Add wallet → Create uses the entered label (not a hardcoded one)", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /\+ Add wallet/i }));

    // The add form opens on the chooser; pick "Create a new wallet".
    fireEvent.click(await screen.findByText(/Create a new wallet/i));
    const nameInput = screen.getByLabelText(/Wallet Name/i) as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: "Savings" } });
    fireEvent.click(screen.getByRole("button", { name: /Create in secure window/i }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "secure_create_wallet");
      expect(call?.[1]).toEqual({ label: "Savings", network: "mainnet" });
    });
  });

  it("no active profile renders the add-wallet chooser", async () => {
    invokeMock.mockImplementation(routeInvoke({ profiles: [] }));
    render(<WalletView />, { wrapper: wrapper() });

    expect(await screen.findByText(/Import your wallet/i)).toBeInTheDocument();
    expect(screen.getByText(/Create a new wallet/i)).toBeInTheDocument();
    expect(screen.getByText(/Watch-only/i)).toBeInTheDocument();
  });
});

// Task C: the Sync action becomes a Stop button while a background sync is
// running (calling cancel_full_sync), and the repair progress line reports
// the new, honest fields (repairCandidates is now the whole-run backlog;
// repairRemaining converges to 0) instead of the old per-window "X / Y".
function baseSyncStatus(overrides: Partial<SyncStatus>): SyncStatus {
  return {
    running: false,
    step: "idle",
    progressLabel: "",
    repaired: 0,
    repairCandidates: 0,
    repairRemaining: 0,
    discovered: 0,
    namesSynced: 0,
    errors: [],
    startedAt: null,
    finishedAt: null,
    discoverAddressesTotal: 0,
    discoverAddressesDone: 0,
    discoverTxsScanned: 0,
    discoverCandidates: 0,
    discoverCurrentName: "",
    waiting: false,
    cancelRequested: false,
    ...overrides,
  };
}

describe("WalletView — sync Stop button + honest progress", () => {
  it("shows a Stop button (not Sync) while running, and Stop invokes cancel_full_sync", async () => {
    const base = routeInvoke({ unlocked: true, canWrite: true });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_sync_status") {
        return Promise.resolve(
          baseSyncStatus({ running: true, step: "repair", progressLabel: "Repairing owned names…" }),
        );
      }
      if (cmd === "cancel_full_sync") return Promise.resolve(null);
      return base(cmd);
    });
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const stopBtn = await screen.findByRole("button", { name: /^Stop$/i });
    expect(screen.queryByRole("button", { name: /^Sync$/i })).toBeNull();

    fireEvent.click(stopBtn);
    await waitFor(() => {
      expect(invokeMock.mock.calls.some((c) => c[0] === "cancel_full_sync")).toBe(true);
    });
  });

  it("shows a plain Sync button (not Stop) when idle", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: true, canWrite: true }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(await screen.findByRole("button", { name: /^Sync$/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Stop$/i })).toBeNull();
  });

  it("renders honest Checked/Owned/Remaining repair progress from the new fields", async () => {
    const base = routeInvoke({ unlocked: true, canWrite: true });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_sync_status") {
        return Promise.resolve(
          baseSyncStatus({
            running: true,
            step: "repair",
            progressLabel: "Repairing owned names…",
            repairCandidates: 540,
            repairRemaining: 535,
            repaired: 3,
          }),
        );
      }
      return base(cmd);
    });
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const panel = await screen.findByTestId("sync-status");
    // 540 total - 535 remaining = 5 checked so far.
    expect(panel.textContent).toMatch(/Checked: 5/);
    expect(panel.textContent).toMatch(/Owned: \+3/);
    expect(panel.textContent).toMatch(/Remaining: ~535/);
    // The old "Repaired: X / Y" wording is gone.
    expect(panel.textContent).not.toMatch(/Repaired:/);
  });
});

describe("WalletView — punycode display (Task 4)", () => {
  it("renders the decoded Unicode form of an xn-- owned name, but sends the RAW name to the backend on Manage", async () => {
    const base = routeInvoke({ unlocked: true, canWrite: true });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "read_names") {
        return Promise.resolve([
          {
            name: "xn--e1adigm",
            state: "CLOSED",
            height: 100,
            renewal: 200,
            owner: { hash: "tx1", index: 0 },
            registered: true,
            stats: null,
          },
        ]);
      }
      return base(cmd);
    });
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");

    // The table renders the decoded Unicode label ("козел" is the Cyrillic
    // decoding of xn--e1adigm), not the raw ACE form.
    expect(await screen.findByText(".козел")).toBeInTheDocument();
    expect(screen.queryByText(".xn--e1adigm")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /^Manage$/i }));

    // Opening NameActionsModal must query the backend with the RAW on-chain
    // name — punycode decoding is display-only and must never leak into an
    // invoke() call.
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "read_name_info");
      expect(call?.[1]).toEqual({ name: "xn--e1adigm" });
    });
    const capsCall = invokeMock.mock.calls.find(
      (c) => c[0] === "get_name_action_capabilities",
    );
    expect(capsCall?.[1]).toMatchObject({ name: "xn--e1adigm" });

    // The modal title shows the decoded name with the raw ACE form alongside
    // it in parentheses, so the user can still see the on-chain form. (The
    // table row behind the modal also shows ".козел", so there are two
    // matches for that text — this asserts at least one, i.e. the modal's.)
    expect((await screen.findAllByText(".козел")).length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("(.xn--e1adigm)")).toBeInTheDocument();
  });
});

describe("WalletView — expiring-soon renewal banner (Task 3 / C3)", () => {
  const expiringRenewals = (source: string) => ({
    walletProfileId: "p1",
    currentHeight: 260000,
    heightSource: "node",
    expiringSoonThresholdDays: 30,
    names: [
      {
        name: "urgentname",
        state: "CLOSED",
        renewalHeight: 156000,
        expiresAtHeight: 261120,
        blocksUntilExpire: 1120,
        daysUntilExpire: 7.8,
        source,
        expiringSoon: true,
      },
    ],
  });

  it("shows the banner for a chain-sourced expiring name and Renew opens the modal with the raw name", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ unlocked: true, canWrite: true, renewals: expiringRenewals("chain") }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    const alert = await screen.findByTestId("expiring-alert");
    expect(alert.textContent).toMatch(/renew/i);
    expect(alert.textContent).toContain(".urgentname");

    fireEvent.click(screen.getByRole("button", { name: /^Renew$/i }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "get_name_action_capabilities",
      );
      expect(call?.[1]).toMatchObject({ name: "urgentname" });
    });
  });

  it("does NOT fire the banner from stale csv-import data", async () => {
    invokeMock.mockImplementation(
      routeInvoke({ unlocked: true, canWrite: true, renewals: expiringRenewals("csv-import") }),
    );
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    expect(screen.queryByTestId("expiring-alert")).toBeNull();
  });

  it("shows no banner when nothing is expiring", async () => {
    invokeMock.mockImplementation(routeInvoke({ unlocked: true, canWrite: true }));
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    expect(screen.queryByTestId("expiring-alert")).toBeNull();
  });
});

describe("WalletView — Recent transactions shows the name (Task 2)", () => {
  it("renders the decoded name next to the action for an 'open' draft", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        unlocked: true,
        canWrite: true,
        drafts: [
          {
            id: "d-open-1",
            walletProfileId: "p1",
            action: "open",
            status: "broadcasted",
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
              name: "example",
            },
            errorMessage: null,
            txid: "abc123txid",
            confirmationHeight: null,
            createdAt: "2026-01-01",
          },
        ],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const row = await screen.findByText(/open/i, { selector: "td" });
    expect(row.textContent).toMatch(/open/i);
    expect(row.textContent).toContain(".example");
  });

  it("shows no name fragment when the draft summary has no name (e.g. a plain send)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        unlocked: true,
        canWrite: true,
        drafts: [
          {
            id: "d-send-1",
            walletProfileId: "p1",
            action: "send_hns",
            status: "confirmed",
            summary: {
              action: "send_hns",
              sendTotalDoos: 1_000_000,
              feeDoos: 1410,
              changeDoos: 0,
              inputTotalDoos: 1_000_000,
              numInputs: 1,
              recipientAddress: "rs1qexample",
              txid: null,
              warnings: [],
            },
            errorMessage: null,
            txid: "def456txid",
            confirmationHeight: 500,
            createdAt: "2026-01-01",
          },
        ],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const row = await screen.findByText(/send_hns/i, { selector: "td" });
    expect(row.textContent?.trim()).toBe("send_hns");
  });
});

describe("WalletView — Account public key (xpub) card for Namebase", () => {
  const REAL_XPUB =
    "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";

  it("renders the card showing the FULL account xpub (not truncated)", async () => {
    invokeMock.mockImplementation(routeInvoke({ profile: { accountXpub: REAL_XPUB } }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const card = await screen.findByTestId("account-xpub-card");
    // Header label (multiple matches exist because the Alert body also
    // mentions "account public key (xpub)" — pick the header specifically).
    expect(within(card).getAllByText(/Account public key \(xpub\)/i).length).toBeGreaterThan(0);
    expect(screen.getByTestId("account-xpub-value")).toHaveTextContent(REAL_XPUB);
  });

  it("Copy writes the exact xpub to the clipboard and flips the label to 'Copied!'", async () => {
    invokeMock.mockImplementation(routeInvoke({ profile: { accountXpub: REAL_XPUB } }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const copyBtn = screen.getByTestId("copy-xpub");
    expect(copyBtn).toHaveTextContent(/Copy public key/i);

    fireEvent.click(copyBtn);

    await waitFor(() => {
      expect(clipboardWriteMock).toHaveBeenCalledWith(REAL_XPUB);
    });
    await waitFor(() => {
      expect(screen.getByTestId("copy-xpub")).toHaveTextContent(/Copied!/i);
    });
  });

  it("shows the Namebase / single-signature info Alert", async () => {
    invokeMock.mockImplementation(routeInvoke({ profile: { accountXpub: REAL_XPUB } }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const card = await screen.findByTestId("account-xpub-card");
    expect(
      within(card).getByText(/For Namebase \/ xpub-import payees only/i),
    ).toBeInTheDocument();
    expect(within(card).getByText(/single-signature wallet/i)).toBeInTheDocument();
  });

  it("renders for watch-only profiles too (they also carry an accountXpub)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        profile: { accountXpub: REAL_XPUB, watchOnly: true, kind: "watch_only_xpub" },
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    expect(screen.getByTestId("account-xpub-value")).toHaveTextContent(REAL_XPUB);
  });
});

describe("WalletView — density cleanup (Disclosure + CopyField regroup)", () => {
  const REAL_XPUB =
    "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";

  it("xpub disclosure is closed by default but the full xpub stays in the DOM", async () => {
    invokeMock.mockImplementation(routeInvoke({ profile: { accountXpub: REAL_XPUB } }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const toggle = screen.getByRole("button", {
      name: /Show account public key \(xpub\) for Namebase/i,
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    // Full value is still in the DOM (mounted-but-hidden), so tests can assert
    // the exact xpub string without opening the disclosure first.
    expect(screen.getByTestId("account-xpub-value")).toHaveTextContent(REAL_XPUB);
  });

  it("clicking the disclosure toggle opens it (aria-expanded flips)", async () => {
    invokeMock.mockImplementation(routeInvoke({ profile: { accountXpub: REAL_XPUB } }));
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const toggle = screen.getByRole("button", {
      name: /Show account public key \(xpub\) for Namebase/i,
    });
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    // Copy button is reachable once open.
    expect(screen.getByTestId("copy-xpub")).toBeInTheDocument();
  });

  it('the Details footer disclosure is collapsed by default and hides the profile diagnostic', async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    const details = screen.getByRole("button", { name: /^\s*›?\s*Details\s*$/ });
    expect(details).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    // Details body still surfaces the "Last successful sync:" label.
    expect(screen.getByText(/Last successful sync:/)).toBeInTheDocument();
  });

  it("Copy Address writes the receive address to the clipboard", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Copy Address/i }));
    await waitFor(() => {
      expect(clipboardWriteMock).toHaveBeenCalledWith("rs1qexamplereceiveaddr");
    });
  });

  it("the QR is hidden by default and toggled by the 'Show QR' button", async () => {
    invokeMock.mockImplementation(routeInvoke());
    const { container } = render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("Primary");
    // No SVG QR mounted initially.
    expect(container.querySelector("svg[role='img']")?.getAttribute("aria-label")).not.toBe(
      "rs1qexamplereceiveaddr",
    );
    fireEvent.click(screen.getByRole("button", { name: /Show QR/i }));
    // After clicking, the toggle label flips and an SVG is now rendered inside the receive card.
    expect(screen.getByRole("button", { name: /Hide QR/i })).toBeInTheDocument();
  });

  it("Recent activity card renders classified rows from read_action_history and links to /activity", async () => {
    // Unix seconds for 2026-07-24 12:00:00 UTC — locks the long-date
    // assertion to a known "July 24, 2026" output regardless of the
    // runner's timezone (formatDateLong uses UTC-normalized parsing).
    const unix = Math.floor(Date.UTC(2026, 6, 24, 12) / 1000);
    invokeMock.mockImplementation(
      routeInvoke({
        history: [
          {
            txid: "aa",
            action: "receive",
            name: null,
            nameHash: null,
            valueDoos: 100_000_000,
            direction: "receive",
            height: 100,
            time: unix,
            confirmed: true,
            counterparty: null,
          },
          {
            txid: "bb",
            action: "bid",
            name: "foo",
            nameHash: "deadbeef",
            // Self-homed BID: net-external flow is 0 (matches the drafts
            // card's `netSpendDoos`).
            valueDoos: 0,
            direction: "send",
            height: 200,
            time: unix,
            confirmed: true,
            counterparty: null,
          },
        ],
      }),
    );
    render(<WalletView />, { wrapper: wrapper() });

    // "Recent activity" header should appear, and the classified rows.
    await waitFor(() => expect(screen.getByText("Recent activity")).toBeInTheDocument());
    // A row with the decoded name shows up as `.foo` inside a table cell.
    await waitFor(() =>
      expect(
        screen.getByText((_, el) => el?.tagName === "TD" && el.textContent === ".foo"),
      ).toBeInTheDocument(),
    );
    // The See all → link routes to /activity.
    const seeAll = screen.getByRole("button", { name: /See all/ });
    expect(seeAll).toBeInTheDocument();
    // Alignment with the drafts card: the BID row (with self-homed
    // valueDoos=0) shows "0.000000" byte-identical to the drafts card, and
    // is colored NEUTRAL (gray) — a self-homed name action is not a loss.
    const zeroSpans = screen
      .getAllByText("0.000000")
      .filter((el) => el.tagName === "SPAN");
    expect(zeroSpans.some((el) => el.className.includes("text-gray-700"))).toBe(true);
    expect(zeroSpans.some((el) => el.className.includes("text-red-600"))).toBe(false);
    // The receive row (positive inflow) is green with a leading "+".
    const incomeSpan = screen
      .getAllByText((_, el) => el?.tagName === "SPAN" && /^\+100\.000000$/.test(el.textContent ?? ""))
      .find((el) => el.className.includes("text-green-600"));
    expect(incomeSpan).toBeTruthy();
    // Long-form date: the row's Date cell reads "July 24, 2026", not the
    // locale-dependent "24/07/2026" or "7/24/2026" that `formatDate` would
    // produce. formatDateLong is the shared helper.
    expect(screen.getAllByText("July 24, 2026").length).toBeGreaterThan(0);
  });
});
