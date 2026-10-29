import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// The ⋯ menu of every Names row (owned names too, not only watched ones): it
// runs a name's one-click action for that name alone, leaving the selection
// alone, and opens the watchlist actions.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));
vi.mock("../../lib/clipboard", () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NamesView } from "../NamesView";

const profile = {
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

const owned = (name: string) => ({
  name,
  state: "CLOSED",
  height: 100,
  renewal: 200,
  registered: true,
  transfer: 0,
  owner: { hash: `t-${name}`, index: 0 },
  stats: { blocksUntilExpire: 50_000, daysUntilExpire: 347 },
});

function cap(name: string) {
  const yes = { allowed: true, reason: null };
  const no = { allowed: false, reason: "not allowed" };
  return {
    name,
    phase: "CLOSED",
    taskState: "ownedNoUrgentAction",
    ownsName: true,
    nameIsRegistered: true,
    canOpen: no,
    canBid: no,
    canReveal: no,
    canRedeem: no,
    canRegister: no,
    canRenew: yes,
    canUpdate: yes,
    canTransfer: yes,
    canFinalize: no,
    canCancelTransfer: no,
    canRevoke: yes,
  };
}

function route() {
  let builds = 0;
  return async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return [profile];
      case "get_signer_session":
        return {
          walletProfileId: profile.id,
          unlocked: true,
          unlockedUntilEpochMs: Date.now() + 60000,
        };
      case "get_write_capability":
        return { signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null };
      case "read_names":
        return ["alpha", "beta", "gamma"].map(owned);
      case "list_tx_drafts":
      case "read_action_history":
      case "list_watchlist":
      case "read_auction_position_names":
        return [];
      case "read_renewals":
        return {
          walletProfileId: profile.id,
          currentHeight: null,
          heightSource: "unknown",
          expiringSoonThresholdDays: 30,
          names: [],
        };
      case "get_names_action_capabilities":
        return (args?.names as string[]).map(cap);
      case "build_batch_renew_draft": {
        builds += 1;
        const names = args?.names as string[];
        return {
          id: `draft-${builds}`,
          walletProfileId: profile.id,
          action: "batch-renew",
          status: "draft",
          summary: {
            action: "batch-renew",
            sendTotalDoos: 0,
            feeDoos: 2_000,
            changeDoos: 0,
            inputTotalDoos: 0,
            numInputs: names.length,
            recipientAddress: null,
            txid: null,
            warnings: [],
            nameList: names,
          },
          errorMessage: null,
          txid: null,
          createdAt: "2026-01-01",
        };
      }
      case "sign_tx_draft":
        return { id: args?.draftId };
      case "broadcast_tx_draft":
        return { txid: "txid1deadbeef00", status: "ok" };
      default:
        return null;
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
        <MemoryRouter initialEntries={["/names"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

async function openMenu(name: string) {
  const row = (await screen.findByLabelText(`Select ${name}`)).closest("tr")!;
  // The menu fills in once the name's capabilities have arrived.
  await waitFor(() => expect(calls("get_names_action_capabilities").length).toBeGreaterThan(0));
  fireEvent.click(within(row).getByRole("button", { name: "More" }));
  return screen.findByRole("menuitem", { name: "Open name" });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route());
  Element.prototype.scrollIntoView = vi.fn();
});

describe("Names — the row menu", () => {
  it("an owned name has a menu with its actions and its tabs, never Revoke", async () => {
    render(<NamesView />, { wrapper: wrapper() });
    await openMenu("beta");
    await screen.findByRole("menuitem", { name: "Renew" });
    const items = screen.getAllByRole("menuitem").map((m) => m.textContent?.trim());
    expect(items).toEqual([
      "Open name",
      "Renew",
      "Transfer or sell…",
      "Edit DNS records",
      "Sign message",
    ]);
  });

  it("Renew from the menu renews that one name and keeps the selection", async () => {
    render(<NamesView />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByLabelText("Select alpha"));
    expect(screen.getByText("1 selected")).toBeInTheDocument();

    await openMenu("beta");
    fireEvent.click(await screen.findByRole("menuitem", { name: "Renew" }));
    await screen.findByRole("dialog");
    expect(screen.getByText("Confirm renew")).toBeInTheDocument();
    expect(calls("build_batch_renew_draft").map((c) => c[1].names)).toEqual([["beta"]]);

    fireEvent.click(screen.getByRole("button", { name: /^Confirm$/i }));
    await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByText("1 selected")).toBeInTheDocument();
  });

  it("with nothing selected the batch actions stay hidden; selecting a name shows them", async () => {
    render(<NamesView />, { wrapper: wrapper() });
    const select = await screen.findByTestId("batch-select-transferable");
    await waitFor(() => expect(select).not.toBeDisabled());
    expect(screen.queryByTestId("batch-action-bar")).toBeNull();
    expect(screen.queryByText("0 selected")).toBeNull();
    expect(screen.queryByTestId("batch-renew-btn")).toBeNull();
    expect(screen.queryByTestId("batch-transfer-btn")).toBeNull();

    fireEvent.click(screen.getByLabelText("Select alpha"));
    expect(screen.getByTestId("batch-action-bar")).toHaveTextContent("1 selected");
    expect(screen.getByTestId("batch-renew-btn")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Select alpha"));
    expect(screen.queryByTestId("batch-action-bar")).toBeNull();
  });

  it("every row has a visible star that watches the name, and unwatches it once watched", async () => {
    let watched: { name: string; tags: string; addedAt: string }[] = [];
    const base = route();
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "list_watchlist") return watched;
      if (cmd === "add_to_watchlist") {
        watched = [{ name: args?.name as string, tags: "", addedAt: "2026-10-29" }];
        return null;
      }
      if (cmd === "remove_from_watchlist") {
        watched = [];
        return null;
      }
      return base(cmd, args);
    });
    render(<NamesView />, { wrapper: wrapper() });
    const row = (await screen.findByLabelText("Select gamma")).closest("tr")!;
    const star = within(row).getByRole("button", { name: "Watch .gamma" });
    expect(star).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(star);
    await waitFor(() =>
      expect(calls("add_to_watchlist").map((c) => c[1])).toEqual([{ name: "gamma" }]),
    );
    const on = await within(row).findByRole("button", { name: "Unwatch .gamma" });
    expect(on).toHaveAttribute("aria-pressed", "true");
    // The row stays put (no row is opened by the click).
    expect(screen.getAllByTestId("names-row")).toHaveLength(3);

    fireEvent.click(on);
    await waitFor(() =>
      expect(calls("remove_from_watchlist").map((c) => c[1])).toEqual([{ name: "gamma" }]),
    );
  });
});
