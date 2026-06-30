import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { Settings } from "../Settings";
import { StatusStrip } from "../ui/StatusStrip";
import { useSettingsStore } from "../../stores/settings";

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
  binary_found: boolean;
  process_alive: boolean;
  connected: boolean;
  height: number | null;
  verification_progress: number | null;
  headers: number | null;
}>;

function nodeStatus(over: NodeOver = {}) {
  return {
    binary: "/usr/local/bin/hsd",
    binary_found: true,
    version: "hsd 8.0.0",
    data_dir: "/Volumes/WD/hsd-data",
    network: "main",
    process_alive: false,
    connected: false,
    height: null,
    verification_progress: null,
    headers: null,
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

function loadSettings(over: Partial<Record<string, string>> = {}) {
  useSettingsStore.setState({
    loaded: true,
    settings: {
      node_rpc_url: "http://127.0.0.1:12037",
      node_rpc_api_key: "",
      hsd_prefix: "",
      hsd_path: "",
      explorer_api_url: "https://e.hnsfans.com",
      address_gap_limit: "20",
      signer_session_timeout_seconds: "900",
      advanced_mode: "false",
      onboarding_complete: "true",
      ...over,
    },
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  loadSettings();
});

describe("Node status (truthful, RPC-based)", () => {
  it("Settings shows Connected · block N when the RPC answers", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ connected: true, process_alive: true, height: 218456 })));
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Connected.*block 218456/i)).toBeInTheDocument();
    // When connected, the control offers Stop (not a Start that could lie green).
    expect(await screen.findByRole("button", { name: /Stop hsd/i })).toBeInTheDocument();
  });

  it("Settings shows the sync progress while behind the chain tip (blocks < headers)", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 40000, headers: 100000, verification_progress: 0.4 })),
    );
    render(<Settings />, { wrapper: wrapper() });

    // pct is blocks/headers (40000/100000 = 40%), shown with the H/headers detail.
    expect(await screen.findByText(/Syncing · 40%/i)).toBeInTheDocument();
    expect(await screen.findByText(/Syncing the chain — 40% · block 40000 \/ 100000/i)).toBeInTheDocument();
  });

  it("Settings shows Connected at the chain tip even when progress < 100% (regtest)", async () => {
    // blocks == headers (tip reached) but verificationprogress only 0.9997 — must
    // read as Connected, not perpetually "Syncing".
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 317, headers: 317, verification_progress: 0.9997 })),
    );
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Connected.*block 317/i)).toBeInTheDocument();
    expect(screen.queryByText(/Syncing/i)).toBeNull();
  });

  it("Settings shows a 100% progress bar at the chain tip (always visible when connected)", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 317, headers: 317, verification_progress: 0.9997 })),
    );
    render(<Settings />, { wrapper: wrapper() });

    // The progress block now renders even at the tip (was hidden when synced).
    const bar = await screen.findByTestId("node-sync-progress");
    expect(bar).toHaveTextContent(/Synced — 100% · block 317 \/ 317/i);
  });

  it("Settings shows the progress bar while syncing too", async () => {
    invokeMock.mockImplementation(
      route(nodeStatus({ connected: true, process_alive: true, height: 40000, headers: 100000, verification_progress: 0.4 })),
    );
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByTestId("node-sync-progress")).toHaveTextContent(/40%/);
  });

  it("Start hsd is enabled when the binary is found and nothing is running", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: true })));
    render(<Settings />, { wrapper: wrapper() });
    const btn = await screen.findByRole("button", { name: /Start hsd/i });
    await waitFor(() => expect(btn).toBeEnabled());
  });

  it("Start hsd is enabled when the binary isn't auto-found but an hsd path is set", async () => {
    loadSettings({ hsd_path: "/Users/me/.nvm/versions/node/v20/bin/hsd" });
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: false })));
    render(<Settings />, { wrapper: wrapper() });
    const btn = await screen.findByRole("button", { name: /Start hsd/i });
    await waitFor(() => expect(btn).toBeEnabled());
  });

  it("Start hsd is disabled when no binary is found and no path is set", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: false })));
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByRole("button", { name: /Start hsd/i })).toBeDisabled();
  });

  it("Settings no longer shows the redundant Wallet block", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ binary_found: true })));
    render(<Settings />, { wrapper: wrapper() });
    // Connections section still renders…
    expect(await screen.findByText(/Connections/i)).toBeInTheDocument();
    // …but the removed Wallet block's "Manage wallets" link is gone.
    expect(screen.queryByRole("button", { name: /Manage wallets/i })).toBeNull();
  });

  it("Settings shows Starting… when the process is alive but RPC isn't up yet", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ connected: false, process_alive: true })));
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/Starting…/i)).toBeInTheDocument();
  });

  it("Settings shows Stopped when nothing is running", async () => {
    invokeMock.mockImplementation(route(nodeStatus()));
    render(<Settings />, { wrapper: wrapper() });

    expect(await screen.findByText(/^Stopped$/i)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /Start hsd/i })).toBeInTheDocument();
  });

  it("StatusStrip says Node: Connected when the RPC answers", async () => {
    invokeMock.mockImplementation(route(nodeStatus({ connected: true, process_alive: true, height: 9 })));
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
});
