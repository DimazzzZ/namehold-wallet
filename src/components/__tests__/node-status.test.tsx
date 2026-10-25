import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import settingsSource from "../Settings.tsx?raw";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: vi.fn().mockResolvedValue(undefined),
  disable: vi.fn().mockResolvedValue(undefined),
  isEnabled: vi.fn().mockResolvedValue(false),
}));

import { Settings } from "../Settings";
import { StatusStrip } from "../ui/StatusStrip";
import { loadSettings } from "../../test/fixtures/settings";

const profile = {
  id: "p1",
  label: "Primary",
  network: "mainnet",
  receiveAddress: "hs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

type NodeOver = Partial<{
  network: string | null;
  binary_found: boolean;
  process_alive: boolean;
  ownership: "own" | "outside" | "none";
  starting: boolean;
  connected: boolean;
  height: number | null;
  verification_progress: number | null;
  headers: number | null;
  synced: boolean;
  last_error: string | null;
  index_mismatch: boolean;
  read_source: "local" | "explorer";
}>;

function nodeStatus(over: NodeOver = {}) {
  return {
    binary: "/usr/local/bin/hsd",
    binary_found: true,
    version: "hsd 8.0.0",
    data_dir: "/Volumes/WD/hsd-data",
    network: "main" as string | null,
    process_alive: false,
    ownership: "none" as "own" | "outside" | "none",
    starting: false,
    connected: false,
    height: null,
    verification_progress: null,
    headers: null,
    node_mode: "full" as const,
    // The backend's own `chain_synced` verdict. The rule itself is tested in
    // Rust (`chain_synced`); these tests pin what the UI renders when handed
    // each answer, so each case states the verdict it is exercising rather
    // than re-deriving one here.
    synced: false,
    last_error: null,
    index_mismatch: false,
    read_source: "explorer",
    ...over,
  };
}

function route(node: ReturnType<typeof nodeStatus>) {
  return (cmd: string) => {
    switch (cmd) {
      case "node_status":
        return Promise.resolve(node);
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: null, unlocked: false, unlockedUntilEpochMs: 0 });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: false,
          broadcasterAvailable: false,
          canWrite: false,
          reason: null,
        });
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

beforeEach(() => {
  invokeMock.mockReset();
  loadSettings();
});

describe("Node status (truthful, RPC-based)", () => {
  it("Settings shows Connected · block N when the RPC answers", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 218456, synced: true })),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Connected.*block 218456/i)).toBeInTheDocument();
    // When connected, the control offers Stop (not a Start that could lie green).
    expect(await screen.findByRole("button", { name: /Stop mainnet node/i })).toBeInTheDocument();
  });

  it("Settings shows the sync progress while behind the chain tip (blocks < headers)", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 40000,
          headers: 100000,
          verification_progress: 0.4,
          synced: false,
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    // pct is blocks/headers (40000/100000 = 40%), shown with the H/headers detail.
    expect(await screen.findByText(/Syncing · 40%/i)).toBeInTheDocument();
    expect(
      await screen.findByText(/Syncing the chain — 40% · block 40000 \/ 100000/i),
    ).toBeInTheDocument();
  });

  it("Settings shows Syncing without denominator when blocks == headers but progress < 0.9999 (early IBD)", async () => {
    // hsd in early IBD reports headers == height (hasn't learned peers' higher headers yet).
    // This is 19% complete, so it's not synced. Must NOT show the confusing "/ 65027" denominator.
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 65027,
          headers: 65027,
          verification_progress: 0.19,
          synced: false,
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Syncing · 19%/i)).toBeInTheDocument();
    // Check that we show the block height without a false denominator.
    const syncText = await screen.findByText(/Syncing the chain — 19% · block 65027/i);
    expect(syncText).toBeInTheDocument();
    // The text should NOT contain the confusing "/ 65027" (because headers == height).
    expect(syncText).not.toHaveTextContent(/\/ 65027/);
  });

  it("Settings shows Connected when blocks == headers even if progress plateaus at 99.9%", async () => {
    // Regression guard for the "stuck at 99.9%" bug: verificationprogress can
    // plateau just under 1.0 (0.9997 here) after the tip is reached. With
    // blocks == headers the node IS caught up, so the badge must flip to
    // Connected instead of sitting on "Syncing · 99.9%" forever.
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 317,
          headers: 317,
          verification_progress: 0.9997,
          synced: true,
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/^Connected · block 317$/i)).toBeInTheDocument();
    expect(screen.queryByText(/Syncing/i)).toBeNull();
  });

  it("Settings shows Syncing when blocks == headers but progress is far below the tip", async () => {
    // A node reporting blocks == headers while only ~8% verified hasn't
    // downloaded the real headers yet. Progress below the loose 0.999 floor
    // must still be treated as unsynced.
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 317,
          headers: 317,
          verification_progress: 0.08,
          synced: false,
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect((await screen.findAllByText(/Syncing/i)).length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText(/^Connected · block 317$/i)).toBeNull();
  });

  it("Settings shows Synced — 100% only when progress >= 0.9999", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 317,
          headers: 317,
          verification_progress: 0.9999,
          synced: true,
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    const bar = await screen.findByTestId("node-sync-progress");
    expect(bar).toHaveTextContent(/Synced — 100% · block 317 \/ 317/i);
  });

  it("Settings shows the progress bar while syncing too", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 40000,
          headers: 100000,
          verification_progress: 0.4,
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByTestId("node-sync-progress")).toHaveTextContent(/40%/);
  });

  it("Start is enabled when the binary is found and nothing is running", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: true })));
    render(<Settings />, { wrapper: wrapper() });
    const btn = await screen.findByRole("button", { name: /Start mainnet node/i });
    await waitFor(() => expect(btn).toBeEnabled());
  });

  it("Start is enabled when the binary isn't auto-found but an hsd path is set", async () => {
    loadSettings({ hsd_path: "/Users/me/.nvm/versions/node/v20/bin/hsd" });
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: false })));
    render(<Settings />, { wrapper: wrapper() });
    const btn = await screen.findByRole("button", { name: /Start mainnet node/i });
    await waitFor(() => expect(btn).toBeEnabled());
  });

  it("Start is disabled when no binary is found and no path is set", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: false })));
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByRole("button", { name: /Start mainnet node/i })).toBeDisabled();
  });

  it("surfaces the hsd start error (not a silent Starting…) when the RPC is down", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: false,
          process_alive: false,
          last_error:
            "This chain was synced without the address index, and hsd can't add an index to an existing chain. Re-sync with address indexing …",
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });
    const err = await screen.findByTestId("node-last-error");
    expect(err).toHaveTextContent(/indexes don't match|address index|re-sync/i);
    // A plain error (not an index mismatch) offers no re-sync button.
    expect(screen.queryByTestId("node-resync")).toBeNull();
  });

  it("offers a one-click Re-sync when the chain's indexes don't match (index_mismatch)", async () => {
    const orig = window.confirm;
    window.confirm = vi.fn(() => true);
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: false,
          process_alive: false,
          index_mismatch: true,
          last_error: "This chain's indexes don't match … Re-sync the node data …",
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    const btn = await screen.findByTestId("node-resync");
    fireEvent.click(btn);
    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).toContain("resync_hsd_chain");
    });
    window.confirm = orig;
  });

  it("Settings no longer shows the redundant Wallet block", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: true })));
    render(<Settings />, { wrapper: wrapper() });
    // Connections section still renders…
    expect(await screen.findByText(/Connections/i)).toBeInTheDocument();
    // …but the removed Wallet block's "Manage wallets" link is gone.
    expect(screen.queryByRole("button", { name: /Manage wallets/i })).toBeNull();
  });

  it("does not stop a connected node Namehold did not start (spec 2026-10-24 R7)", async () => {
    // External node: connected via RPC, but neither a child of ours nor an
    // earlier session's own node, so the backend reports it outside.
    invokeMock.mockImplementation(
      route(
        nodeStatus({ connected: true, process_alive: false, ownership: "outside", height: 500 }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByRole("button", { name: /Stop mainnet node/i })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /Start mainnet node/i })).toBeNull();
  });

  it("Settings shows Starting… when the process is alive but RPC isn't up yet", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({ connected: false, process_alive: true, ownership: "own", starting: true }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText("Starting mainnet node…")).toBeInTheDocument();
  });

  it("offers Stop for an own node an earlier session left that is not answering yet", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({ process_alive: false, connected: false, ownership: "own", starting: true }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByRole("button", { name: /Stop mainnet node/i })).toBeEnabled();
  });

  it("Settings shows Stopped when nothing is running", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/^Stopped$/i)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /Start mainnet node/i })).toBeInTheDocument();
  });

  it("StatusStrip says Node: Connected when the RPC answers", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 9 })),
    );
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(await screen.findByText("Node:")).toBeInTheDocument();
    expect(await screen.findByText("Connected")).toBeInTheDocument();
  });

  it("StatusStrip says Node: Offline when no node is connected", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(await screen.findByText("Node:")).toBeInTheDocument();
    expect(await screen.findByText("Offline")).toBeInTheDocument();
  });

  it("StatusStrip shows Source: Explorer when node is not synced", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ connected: false, read_source: "explorer" })));
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(await screen.findByText("Source:")).toBeInTheDocument();
    expect(await screen.findByText("Explorer")).toBeInTheDocument();
  });

  it("StatusStrip shows Source: Local when node is connected and synced", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 100,
          headers: 100,
          read_source: "local",
        }),
      ),
    );
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(await screen.findByText("Source:")).toBeInTheDocument();
    expect(await screen.findByText("Local")).toBeInTheDocument();
  });

  it("Settings shows Read source: Explorer when node is not synced", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ connected: false, read_source: "explorer" })));
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Read source:/)).toBeInTheDocument();
    expect(await screen.findByText("Explorer")).toBeInTheDocument();
  });

  it("Settings shows Read source: Local node cache when node is synced", async () => {
    invokeMock.mockImplementation(
      route(
        nodeStatus({
          connected: true,
          process_alive: true,
          height: 100,
          headers: 100,
          read_source: "local",
        }),
      ),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Read source:/)).toBeInTheDocument();
    expect(await screen.findByText("Local node cache")).toBeInTheDocument();
  });

  it("Settings shows updated explorer description about fallback behavior", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<Settings />, { wrapper: wrapper() });

    expect(
      await screen.findByText(
        /when the node is connected and fully synced, reads come from the local node cache/i,
      ),
    ).toBeInTheDocument();
  });
});

describe("Settings — bid backup export (Task 2 / C2)", () => {
  it("shows a warning to back up alongside the seed, has the backend write the file at the chosen path", async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    (save as unknown as ReturnType<typeof vi.fn>).mockResolvedValue("/tmp/bid-backup.json");

    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "node_status":
          return Promise.resolve(nodeStatus());
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: null,
            unlocked: false,
            unlockedUntilEpochMs: 0,
          });
        case "get_write_capability":
          return Promise.resolve({
            signerUnlocked: false,
            broadcasterAvailable: false,
            canWrite: false,
            reason: null,
          });
        case "save_bid_backup":
          return Promise.resolve(null);
        default:
          return Promise.resolve(null);
      }
    });
    render(<Settings />, { wrapper: wrapper() });

    // Warning copy about backing up alongside the seed.
    expect(await screen.findByText(/store it alongside your seed phrase/i)).toBeInTheDocument();

    const button = await screen.findByTestId("export-bid-backup");
    // The button is disabled until the active profile loads.
    await waitFor(() => expect(button).not.toBeDisabled());
    fireEvent.click(button);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_bid_backup", {
        walletProfileId: profile.id,
        path: "/tmp/bid-backup.json",
      });
    });
    // The file is written by the backend: the webview has no fs permission.
    expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("export_bid_commitments");
  });

  it("does not import the fs plugin", () => {
    expect(settingsSource).not.toContain("plugin-fs");
  });

  it("does nothing when the save dialog is cancelled", async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    (save as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "node_status":
          return Promise.resolve(nodeStatus());
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: null,
            unlocked: false,
            unlockedUntilEpochMs: 0,
          });
        case "get_write_capability":
          return Promise.resolve({
            signerUnlocked: false,
            broadcasterAvailable: false,
            canWrite: false,
            reason: null,
          });
        case "save_bid_backup":
          return Promise.resolve(null);
        default:
          return Promise.resolve(null);
      }
    });
    render(<Settings />, { wrapper: wrapper() });

    const button = await screen.findByTestId("export-bid-backup");
    await waitFor(() => expect(button).not.toBeDisabled());
    fireEvent.click(button);

    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("save_bid_backup");
  });
});

describe("StatusStrip Node pill menu", () => {
  it("shows Start node when offline", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<StatusStrip />, { wrapper: wrapper() });

    const pill = await screen.findByTestId("status-strip-node");
    fireEvent.click(pill);

    expect(await screen.findByTestId("status-strip-node-start")).toBeInTheDocument();
    expect(screen.queryByTestId("status-strip-node-stop")).not.toBeInTheDocument();
    expect(screen.queryByTestId("status-strip-node-resync")).not.toBeInTheDocument();
  });

  it("shows Stop and Re-sync when connected", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 100 })),
    );
    render(<StatusStrip />, { wrapper: wrapper() });

    const pill = await screen.findByTestId("status-strip-node");
    fireEvent.click(pill);

    expect(await screen.findByTestId("status-strip-node-stop")).toBeInTheDocument();
    expect(screen.getByTestId("status-strip-node-resync")).toBeInTheDocument();
    expect(screen.queryByTestId("status-strip-node-start")).not.toBeInTheDocument();
  });

  it("Start node calls start_hsd", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<StatusStrip />, { wrapper: wrapper() });

    const pill = await screen.findByTestId("status-strip-node");
    fireEvent.click(pill);

    const startBtn = await screen.findByTestId("status-strip-node-start");
    fireEvent.click(startBtn);

    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).toContain("start_hsd");
    });
  });

  it("says when the active network's node is starting (spec 2026-10-24 R11)", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ process_alive: true, ownership: "own", starting: true })),
    );
    render(<StatusStrip />, { wrapper: wrapper() });
    expect(await screen.findByText("Starting mainnet node…")).toBeInTheDocument();
  });

  it("names the active network on Start (spec 2026-10-24 R8)", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<StatusStrip />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("status-strip-node"));
    expect(await screen.findByTestId("status-strip-node-start")).toHaveTextContent(
      "Start mainnet node",
    );
  });

  it("names no network when no wallet is active (spec 2026-10-24 R8)", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ network: null })));
    render(<StatusStrip />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("status-strip-node"));
    expect(await screen.findByTestId("status-strip-node-start")).toHaveTextContent(/^Start node$/);
  });

  it("disables Re-sync for a node Namehold did not start (spec 2026-10-24 R7)", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, ownership: "outside", height: 100 })),
    );
    render(<StatusStrip />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("status-strip-node"));
    expect(await screen.findByTestId("status-strip-node-resync")).toBeDisabled();
  });

  it("disables Stop for a node Namehold did not start (spec 2026-10-24 R7)", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, ownership: "outside", height: 100 })),
    );
    render(<StatusStrip />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("status-strip-node"));
    expect(await screen.findByTestId("status-strip-node-stop")).toBeDisabled();
  });

  it("always shows Open Settings as a secondary action", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<StatusStrip />, { wrapper: wrapper() });

    const pill = await screen.findByTestId("status-strip-node");
    fireEvent.click(pill);

    expect(await screen.findByTestId("status-strip-node-settings")).toBeInTheDocument();
  });

  it("Sending pill still navigates to /settings (no menu)", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 100 })),
    );
    render(<StatusStrip />, { wrapper: wrapper() });

    // Sending pill should be a plain button, not a popover trigger.
    const sendingPill = await screen.findByText("Sending:");
    const sendingBtn = sendingPill.closest("button")!;
    // Clicking it should NOT open a menu (no role=menu appears).
    fireEvent.click(sendingBtn);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });
});
