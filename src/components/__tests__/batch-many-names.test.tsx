import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, configure } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// A wallet with hundreds of names: capabilities asked in chunks the backend
// accepts, a transfer of more than 100 names sent as a series after one
// confirm, transfers allowed from chain facts before capabilities arrive, the
// names that block a transfer named and dropped in one click, progress shown.

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

// Hundreds of real table rows render slowly when the whole suite runs in
// parallel: give these tests and their waits room instead of flaking.
vi.setConfig({ testTimeout: 30_000 });
configure({ asyncUtilTimeout: 10_000 });

const RECIPIENT = "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2";
const FEE = 15_933;

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

const owned = (name: string, over: Record<string, unknown> = {}) => ({
  name,
  state: "CLOSED",
  height: 100,
  renewal: 200,
  registered: true,
  transfer: 0,
  owner: { hash: `t-${name}`, index: 0 },
  stats: { blocksUntilExpire: 50_000, daysUntilExpire: 347 },
  ...over,
});

const pad = (i: number) => String(i).padStart(3, "0");
const many = (n: number) => Array.from({ length: n }, (_, i) => owned(`name${pad(i)}`));

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
    canRevoke: no,
  };
}

interface Opts {
  names?: ReturnType<typeof owned>[];
  /** Capabilities never answer (a slow node). */
  capsPending?: boolean;
  /** The nth broadcast (1-based) fails. */
  failBroadcast?: number;
  /** Builds wait for this promise. */
  buildGate?: Promise<void>;
  /** The nth build (1-based) fails. */
  failBuild?: number;
  /** The nth sign (1-based) fails (a declined secure window). */
  failSign?: number;
  /** The second sign waits for this promise. */
  secondSignGate?: Promise<void>;
  /** The capability call for a chunk containing this name fails. */
  failCapsWith?: string;
}

function route(o: Opts = {}) {
  const list = o.names ?? many(150);
  let builds = 0;
  let broadcasts = 0;
  let signs = 0;
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
        return list;
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
        if (o.capsPending) return new Promise(() => {});
        if (o.failCapsWith && (args?.names as string[]).includes(o.failCapsWith)) {
          throw new Error("node did not answer");
        }
        return (args?.names as string[]).map(cap);
      case "build_batch_renew_draft":
      case "build_batch_transfer_draft": {
        if (o.buildGate) await o.buildGate;
        builds += 1;
        if (builds === o.failBuild) throw new Error("could not build the draft");
        const names = args?.names as string[];
        return {
          id: `draft-${builds}`,
          walletProfileId: profile.id,
          action: "batch-transfer",
          status: "draft",
          summary: {
            action: "batch-transfer",
            sendTotalDoos: 1_000_000 * names.length,
            feeDoos: FEE,
            changeDoos: 0,
            inputTotalDoos: 0,
            numInputs: names.length,
            recipientAddress: RECIPIENT,
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
        signs += 1;
        if (signs === o.failSign) throw new Error("Confirmation declined");
        if (signs === 2 && o.secondSignGate) await o.secondSignGate;
        return { id: args?.draftId };
      case "broadcast_tx_draft":
        broadcasts += 1;
        if (broadcasts === o.failBroadcast) throw new Error("node refused the transaction");
        return { txid: `txid${broadcasts}deadbeef00`, status: "ok" };
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

async function selectAllTransferable() {
  const btn = await screen.findByTestId("batch-select-transferable");
  await waitFor(() => expect(btn).not.toBeDisabled());
  fireEvent.click(btn);
}

function startTransfer() {
  fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
    target: { value: RECIPIENT },
  });
  fireEvent.click(screen.getByTestId("batch-transfer-btn"));
}

beforeEach(() => {
  invokeMock.mockReset();
  Element.prototype.scrollIntoView = vi.fn();
});

describe("Names — a wallet with hundreds of names", () => {
  it("asks for capabilities in chunks the backend accepts (at most 100 names each)", async () => {
    const names = many(250);
    invokeMock.mockImplementation(route({ names }));
    render(<NamesView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(calls("get_names_action_capabilities").length).toBeGreaterThanOrEqual(3),
    );
    const asked = calls("get_names_action_capabilities").map((c) => c[1].names as string[]);
    expect(asked.every((a) => a.length <= 100)).toBe(true);
    expect(new Set(asked.flat())).toEqual(new Set(names.map((n) => n.name)));
  });

  it("sends a transfer of 150 names as two transactions after one confirm, with the fee of both", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrapper() });
    await selectAllTransferable();
    expect(screen.getByText("150 selected")).toBeInTheDocument();
    startTransfer();

    const dialog = await screen.findByText(/Confirm batch transfer/i);
    expect(dialog).toBeInTheDocument();
    const body = screen.getByRole("dialog").textContent ?? "";
    expect(body).toMatch(/150\s*names\s*in\s*2\s*transactions/);
    expect(body).toMatch(/~0\.031866\s*HNS for all transactions/); // 2 × 15,933 doos
    expect(screen.getByTestId("batch-amount")).toHaveTextContent("in the first of 2 transactions");

    fireEvent.click(screen.getByRole("button", { name: /^Confirm$/i }));
    await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(2));
    const built = calls("build_batch_transfer_draft").map((c) => (c[1].names as string[]).length);
    expect(built).toEqual([100, 50]);
    expect(calls("sign_tx_draft").map((c) => c[1].draftId)).toEqual(["draft-1", "draft-2"]);
    await waitFor(() => expect(screen.getByText("0 selected")).toBeInTheDocument());
  });

  it("keeps the names not yet sent selected when a later transaction fails", async () => {
    invokeMock.mockImplementation(route({ failBroadcast: 2 }));
    render(<NamesView />, { wrapper: wrapper() });
    await selectAllTransferable();
    startTransfer();
    await screen.findByText(/Confirm batch transfer/i);
    fireEvent.click(screen.getByRole("button", { name: /^Confirm$/i }));
    await waitFor(() => expect(screen.getByText("50 selected")).toBeInTheDocument());
    expect(calls("broadcast_tx_draft")).toHaveLength(2);
  });

  it("allows a transfer from chain facts while capabilities are still loading", async () => {
    invokeMock.mockImplementation(route({ capsPending: true }));
    render(<NamesView />, { wrapper: wrapper() });
    await selectAllTransferable();
    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    expect(screen.getByTestId("batch-transfer-btn")).not.toBeDisabled();
  });

  it("names the names that block a transfer and drops them in one click", async () => {
    const names = [
      ...many(5),
      owned("lapsed", { registered: false }),
      owned("gone", { stats: { blocksUntilExpire: 0, daysUntilExpire: 0 } }),
    ];
    invokeMock.mockImplementation(route({ names, capsPending: true }));
    render(<NamesView />, { wrapper: wrapper() });
    await screen.findByText(/\.lapsed/);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all names" }));
    const help = await screen.findByTestId("batch-transfer-help");
    expect(help).toHaveTextContent("lapsed (not registered)");
    expect(help).toHaveTextContent("gone (expired)");
    fireEvent.click(screen.getByTestId("batch-drop-blockers"));
    expect(screen.getByText("5 selected")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
      target: { value: RECIPIENT },
    });
    expect(screen.getByTestId("batch-transfer-btn")).not.toBeDisabled();
  });

  it("shows which transaction is being built while it is built", async () => {
    let open!: () => void;
    const buildGate = new Promise<void>((r) => (open = r));
    invokeMock.mockImplementation(route({ buildGate }));
    render(<NamesView />, { wrapper: wrapper() });
    await selectAllTransferable();
    startTransfer();
    expect(await screen.findByTestId("batch-progress")).toHaveTextContent(
      "Building transfer transaction 1 of 2…",
    );
    open();
    await screen.findByText(/Confirm batch transfer/i);
    expect(screen.queryByTestId("batch-progress")).toBeNull();
  });

  describe("edge cases", () => {
    const confirm = () => fireEvent.click(screen.getByRole("button", { name: /^Confirm$/i }));
    const dialogText = () => screen.getByRole("dialog").textContent ?? "";

    it("exactly 100 names is one transaction and says nothing about a series", async () => {
      invokeMock.mockImplementation(route({ names: many(100) }));
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      startTransfer();
      await screen.findByText(/Confirm batch transfer/i);
      expect(dialogText()).not.toMatch(/transactions/);
      confirm();
      await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(1));
      expect(
        calls("build_batch_transfer_draft").map((c) => (c[1].names as string[]).length),
      ).toEqual([100]);
    });

    it("101 names is 100 then 1", async () => {
      invokeMock.mockImplementation(route({ names: many(101) }));
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      startTransfer();
      await screen.findByText(/Confirm batch transfer/i);
      expect(dialogText()).toMatch(/in\s*2\s*transactions/);
      confirm();
      await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(2));
      expect(
        calls("build_batch_transfer_draft").map((c) => (c[1].names as string[]).length),
      ).toEqual([100, 1]);
    });

    it("a failed build of the second transaction keeps the names not yet sent selected", async () => {
      invokeMock.mockImplementation(route({ failBuild: 2 }));
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      startTransfer();
      await screen.findByText(/Confirm batch transfer/i);
      confirm();
      await waitFor(() => expect(screen.getByText("50 selected")).toBeInTheDocument());
      expect(calls("broadcast_tx_draft")).toHaveLength(1);
      expect(screen.queryByTestId("batch-progress")).toBeNull();
    });

    it("a declined first signature sends nothing and keeps the confirmation open", async () => {
      invokeMock.mockImplementation(route({ failSign: 1 }));
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      startTransfer();
      await screen.findByText(/Confirm batch transfer/i);
      confirm();
      await waitFor(() => expect(calls("sign_tx_draft")).toHaveLength(1));
      expect(calls("broadcast_tx_draft")).toHaveLength(0);
      expect(screen.getByText(/Confirm batch transfer/i)).toBeInTheDocument();
      expect(screen.getByText("150 selected")).toBeInTheDocument();
    });

    it("Cancel after the first draft, then Transfer again, starts the series from the first chunk", async () => {
      invokeMock.mockImplementation(route());
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      startTransfer();
      await screen.findByText(/Confirm batch transfer/i);
      fireEvent.click(screen.getByRole("button", { name: /^Cancel$/i }));
      fireEvent.click(screen.getByTestId("batch-transfer-btn"));
      await screen.findByText(/Confirm batch transfer/i);
      confirm();
      await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(2));
      expect(
        calls("build_batch_transfer_draft").map((c) => (c[1].names as string[]).length),
      ).toEqual([100, 100, 50]);
      expect(calls("sign_tx_draft").map((c) => c[1].draftId)).toEqual(["draft-2", "draft-3"]);
    });

    it("a renewal of 150 names is a series too", async () => {
      invokeMock.mockImplementation(route());
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      fireEvent.click(screen.getByTestId("batch-renew-btn"));
      await screen.findByText(/Confirm batch renew/i);
      confirm();
      await waitFor(() => expect(calls("broadcast_tx_draft")).toHaveLength(2));
      expect(calls("build_batch_renew_draft").map((c) => (c[1].names as string[]).length)).toEqual([
        100, 50,
      ]);
    });

    it("shows which transaction is being signed and sent", async () => {
      let open!: () => void;
      const secondSignGate = new Promise<void>((r) => (open = r));
      invokeMock.mockImplementation(route({ secondSignGate }));
      render(<NamesView />, { wrapper: wrapper() });
      await selectAllTransferable();
      startTransfer();
      await screen.findByText(/Confirm batch transfer/i);
      confirm();
      expect(await screen.findByTestId("batch-progress")).toHaveTextContent(
        "Signing and sending transfer transaction 2 of 2…",
      );
      open();
      await waitFor(() => expect(screen.queryByTestId("batch-progress")).toBeNull());
    });

    it("names each chain fact that blocks a transfer, and a capability's own reason", async () => {
      const names = [
        owned("plain"),
        owned("moving", { transfer: 100 }),
        owned("bidding", { state: "BIDDING" }),
        owned("nostats", { stats: null }),
        owned("listed"),
      ];
      invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "get_names_action_capabilities") {
          return (args?.names as string[]).map((n) =>
            n === "listed"
              ? {
                  ...cap(n),
                  canTransfer: { allowed: false, reason: "this name is locked for sale" },
                }
              : cap(n),
          );
        }
        return route({ names })(cmd, args);
      });
      render(<NamesView />, { wrapper: wrapper() });
      await screen.findByText(/\.listed/);
      await waitFor(() => expect(calls("get_names_action_capabilities").length).toBeGreaterThan(0));
      fireEvent.click(screen.getByRole("checkbox", { name: "Select all names" }));
      const help = await screen.findByTestId("batch-transfer-help");
      await waitFor(() => expect(help).toHaveTextContent("listed (this name is locked for sale)"));
      expect(help).toHaveTextContent(/moving \(a transfer is already pending\)/);
      expect(help).not.toHaveTextContent(/plain \(|nostats \(/);
      fireEvent.change(screen.getByTestId("batch-transfer-recipient-input"), {
        target: { value: RECIPIENT },
      });
      expect(screen.getByTestId("batch-transfer-btn")).toBeDisabled();
      fireEvent.click(screen.getByTestId("batch-drop-blockers"));
      expect(screen.getByTestId("batch-transfer-btn")).not.toBeDisabled();
    });

    it("exactly 100 names ask for capabilities once, 101 twice", async () => {
      invokeMock.mockImplementation(route({ names: many(100) }));
      const first = render(<NamesView />, { wrapper: wrapper() });
      await waitFor(() => expect(calls("get_names_action_capabilities")).toHaveLength(1));
      first.unmount();
      invokeMock.mockReset();
      invokeMock.mockImplementation(route({ names: many(101) }));
      render(<NamesView />, { wrapper: wrapper() });
      await waitFor(() => expect(calls("get_names_action_capabilities")).toHaveLength(2));
    });

    it("a failed capability chunk keeps the answers of the others", async () => {
      // name150 sits in the second chunk of 100; the first chunk still answers.
      invokeMock.mockImplementation(route({ names: many(150), failCapsWith: "name149" }));
      render(<NamesView />, { wrapper: wrapper() });
      await screen.findByText(/\.name000/);
      const row = (n: string) =>
        screen.getAllByTestId("names-row").find((r) => r.textContent?.includes(`.${n}`))!;
      await waitFor(() => expect(row("name000")).toHaveTextContent(/Owned/));
    });
  });
});
