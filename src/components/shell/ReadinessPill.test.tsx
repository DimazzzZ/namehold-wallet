import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { ReadinessPill, readiness } from "./ReadinessPill";
import { useUiStore } from "../../stores/ui";
import { OUTSIDE_NODE_NOT_STOPPED } from "../../lib/nodeNetwork";

const hot = { kind: "mnemonic_hot", watchOnly: false, network: "mainnet" } as const;
const ledger = { kind: "ledger_hardware", watchOnly: false, network: "mainnet" } as const;
const watch = { kind: "watch_only_xpub", watchOnly: true, network: "mainnet" } as const;
const cap = (
  over: Partial<{
    canWrite: boolean;
    signerUnlocked: boolean;
    broadcasterAvailable: boolean;
    reason: string | null;
  }> = {},
) => ({
  signerUnlocked: true,
  broadcasterAvailable: true,
  canWrite: true,
  reason: null,
  ...over,
});

describe("readiness()", () => {
  const cases: [string, Parameters<typeof readiness>[0], string, string | null][] = [
    [
      "1 no wallet",
      { profile: null, writeCap: cap(), node: null },
      "No wallet",
      "create or import one",
    ],
    ["2 watch-only", { profile: watch, writeCap: cap(), node: null }, "Watch-only", "can't send"],
    [
      "3a ledger ready",
      {
        profile: ledger,
        writeCap: cap({ signerUnlocked: false }),
        node: { connected: true, synced: true },
      },
      "Ready to send",
      "confirm on your Ledger",
    ],
    [
      "3 ready with height",
      { profile: hot, writeCap: cap(), node: { connected: true, synced: true, height: 813204 } },
      "Ready to send",
      "block 813,204",
    ],
    ["3 ready, no height", { profile: hot, writeCap: cap(), node: null }, "Ready to send", null],
    [
      "4 offline",
      { profile: hot, writeCap: cap({ canWrite: false, broadcasterAvailable: false }), node: {} },
      "Can't send",
      "node offline",
    ],
    [
      "5 starting",
      {
        profile: hot,
        writeCap: cap({ canWrite: false, broadcasterAvailable: false }),
        node: { starting: true },
      },
      "Can't send",
      "node starting",
    ],
    [
      "6 syncing",
      {
        profile: hot,
        writeCap: cap({ canWrite: false, broadcasterAvailable: false }),
        node: { connected: true, synced: false, verification_progress: 0.617 },
      },
      "Can't send",
      "node syncing 61%",
    ],
    [
      "7 no broadcaster",
      {
        profile: hot,
        writeCap: cap({ canWrite: false, broadcasterAvailable: false, reason: "SPV" }),
        node: { connected: true, synced: true },
      },
      "Can't send",
      "can't broadcast",
    ],
    [
      "8 locked",
      { profile: hot, writeCap: cap({ canWrite: false, signerUnlocked: false }), node: null },
      "Locked",
      "unlock to send",
    ],
    [
      "8a ledger guard",
      { profile: ledger, writeCap: cap({ canWrite: false, reason: "why" }), node: null },
      "Can't send",
      "see why",
    ],
    ["9 loading", { profile: hot, writeCap: undefined, node: null }, "Checking…", null],
  ];
  it.each(cases)("%s", (_, input, label, detail) => {
    const r = readiness(input);
    expect(r.label).toBe(label);
    expect(r.detail).toBe(detail);
  });

  it("puts writeCap.reason in the summary when nothing can broadcast", () => {
    const r = readiness({
      profile: hot,
      writeCap: cap({ canWrite: false, broadcasterAvailable: false, reason: "Remote off." }),
      node: { connected: true, synced: true },
    });
    expect(r.summary).toBe("Remote off.");
  });

  it("never invents a sync percentage", () => {
    const r = readiness({
      profile: hot,
      writeCap: cap({ canWrite: false, broadcasterAvailable: false }),
      node: { connected: true, synced: false },
    });
    expect(r.detail).toBe("node syncing");
    expect(r.syncPct).toBeNull();
  });
});

// --- The popover, against a mocked backend ---

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "mainnet",
  watchOnly: false,
  hasPassphrase: true,
  active: true,
  lastSyncedAt: null,
  lastExplorerSyncAt: null,
  lastSyncedHeight: null,
};

function node(over: Record<string, unknown> = {}) {
  return {
    network: "main",
    process_alive: false,
    ownership: "none",
    starting: false,
    connected: false,
    height: null,
    verification_progress: null,
    headers: null,
    synced: false,
    read_source: "explorer",
    node_mode: "full",
    ...over,
  };
}

function route(n: ReturnType<typeof node>, profiles: unknown[] = [profile]) {
  return (cmd: string) => {
    switch (cmd) {
      case "node_status":
        return Promise.resolve(n);
      case "list_wallet_profiles":
        return Promise.resolve(profiles);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: null, unlocked: false, unlockedUntilEpochMs: 0 });
      case "get_write_capability":
        return Promise.resolve(
          cap({ canWrite: false, signerUnlocked: false, broadcasterAvailable: false }),
        );
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

async function openPopover() {
  fireEvent.click(await screen.findByTestId("readiness-pill"));
  return screen.getByTestId("readiness-popover");
}

beforeEach(() => {
  invokeMock.mockReset();
  useUiStore.setState({ readinessOpen: false, walletMenuOpen: false, walletsDialog: null });
});

describe("ReadinessPill popover", () => {
  it("offers Start {network} node when offline, which calls start_hsd", async () => {
    invokeMock.mockImplementation(route(node()));
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    const start = await screen.findByTestId("readiness-node-start");
    expect(start).toHaveTextContent("Start mainnet node");
    expect(screen.queryByTestId("readiness-node-stop")).not.toBeInTheDocument();
    fireEvent.click(start);
    await waitFor(() => expect(invokeMock.mock.calls.map((c) => c[0])).toContain("start_hsd"));
  });

  it("names no network when no wallet is active", async () => {
    invokeMock.mockImplementation(route(node({ network: null }), []));
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    expect(await screen.findByTestId("readiness-node-start")).toHaveTextContent(/^Start node$/);
  });

  it("says the node is starting and offers Cancel start", async () => {
    invokeMock.mockImplementation(
      route(node({ process_alive: true, ownership: "own", starting: true })),
    );
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    expect(await screen.findByText("Starting mainnet node…")).toBeInTheDocument();
    expect(screen.getByTestId("readiness-node-cancel")).toBeInTheDocument();
  });

  it("offers Stop and Re-sync when connected; Re-sync asks first", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    invokeMock.mockImplementation(
      route(node({ connected: true, synced: true, ownership: "own", height: 100 })),
    );
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    fireEvent.click(await screen.findByTestId("readiness-node-resync"));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Re-sync node data?"));
    expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("resync_hsd");
    expect(screen.getByTestId("readiness-node-stop")).toBeEnabled();
    confirm.mockRestore();
  });

  it("disables Stop and Re-sync for an outside node, even while it syncs", async () => {
    invokeMock.mockImplementation(
      route(node({ connected: true, synced: false, ownership: "outside", height: 100 })),
    );
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    expect(await screen.findByTestId("readiness-node-stop")).toBeDisabled();
    expect(screen.getByTestId("readiness-node-resync")).toBeDisabled();
    expect(OUTSIDE_NODE_NOT_STOPPED).toMatch(/not started by Namehold/);
  });

  it("shows the read source", async () => {
    invokeMock.mockImplementation(
      route(node({ connected: true, synced: true, ownership: "own", read_source: "local" })),
    );
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    expect(await screen.findByText("Reads from your local node")).toBeInTheDocument();
  });

  it("shows the signer row with its passphrase hint for a hot wallet", async () => {
    invokeMock.mockImplementation(route(node()));
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    expect(await screen.findByText("Signer locked")).toBeInTheDocument();
    expect(screen.getByText("Unlock with your passphrase, in a secure window")).toBeInTheDocument();
    expect(screen.getByTestId("readiness-signer-toggle")).toHaveTextContent("Unlock");
  });

  it("shows no signer row for a Ledger, but says it signs on the device", async () => {
    invokeMock.mockImplementation(route(node(), [{ ...profile, kind: "ledger_hardware" }]));
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    expect(await screen.findByText("Signs on your Ledger")).toBeInTheDocument();
    expect(screen.queryByText(/Signer (un)?locked/)).toBeNull();
    expect(screen.queryByTestId("readiness-signer-toggle")).toBeNull();
  });

  it("Connections in Settings closes the popover", async () => {
    invokeMock.mockImplementation(route(node()));
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    fireEvent.click(screen.getByText(/Connections in Settings/));
    expect(screen.queryByTestId("readiness-popover")).not.toBeInTheDocument();
  });

  it("Esc closes the popover and stops there", async () => {
    invokeMock.mockImplementation(route(node()));
    const onDocEsc = vi.fn();
    document.addEventListener("keydown", onDocEsc);
    render(<ReadinessPill />, { wrapper: wrapper() });
    await openPopover();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("readiness-popover")).not.toBeInTheDocument();
    expect(onDocEsc).not.toHaveBeenCalled();
    document.removeEventListener("keydown", onDocEsc);
  });
});
