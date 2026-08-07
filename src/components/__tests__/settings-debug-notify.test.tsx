/**
 * Settings — dev-only Debug notifications panel.
 *
 * Verifies the panel renders under a Tauri runtime in dev, and that clicking
 * a kind button invokes the `simulate_notification` command with the right
 * `kind`. The underlying Rust command is `#[cfg(all(debug_assertions,
 * not(test)))]`-gated; this test only exercises the frontend wiring.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(true),
  requestPermission: vi.fn().mockResolvedValue("granted"),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: vi.fn().mockResolvedValue(undefined),
  disable: vi.fn().mockResolvedValue(undefined),
  isEnabled: vi.fn().mockResolvedValue(false),
}));

import { Settings } from "../Settings";
import { useSettingsStore } from "../../stores/settings";

function route(cmd: string) {
  switch (cmd) {
    case "node_status":
      return Promise.resolve({
        binary: null,
        binary_found: false,
        version: null,
        data_dir: null,
        network: "main",
        process_alive: false,
        connected: false,
        height: null,
        verification_progress: null,
        headers: null,
        last_error: null,
        index_mismatch: false,
        read_source: "explorer",
      });
    case "list_wallet_profiles":
      return Promise.resolve([]);
    case "get_signer_session":
      return Promise.resolve({ walletProfileId: null, unlocked: false, unlockedUntilEpochMs: 0 });
    case "get_write_capability":
      return Promise.resolve({
        signerUnlocked: false,
        broadcasterAvailable: false,
        canWrite: false,
        reason: null,
      });
    case "simulate_notification":
      return Promise.resolve(null);
    case "update_setting":
      return Promise.resolve(null);
    default:
      return Promise.resolve(null);
  }
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
      autostart_hsd: "true",
      explorer_api_url: "https://e.hnsfans.com",
      address_gap_limit: "20",
      signer_session_timeout_seconds: "900",
      onboarding_complete: "true",
      deadline_notify_enabled: "false",
      deadline_notify_reveal_lead_blocks: "144",
      deadline_notify_renewal_lead_days: "30",
      watchlist_notify_enabled: "false",
      watchlist_notify_bidding_soon_lead_blocks: "144",
      watchlist_notify_highest_bid_threshold_hns: "",
      background_sync_enabled: "1",
      node_mode: "full",
      explorer_fallback_url: "",
      chain_source: "local_node",
      close_to_tray: "1",
      launch_at_login: "0",
      ...over,
    },
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
  loadSettings();
  // The panel gates on isTauri(): pretend we're inside a Tauri shell.
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
});

afterEach(() => {
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("Settings — Debug notifications panel", () => {
  it("renders the dev-only panel with a button per kind", async () => {
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByTestId("debug-notifications-panel")).toBeInTheDocument();
    for (const kind of ["reveal", "renewal", "bidding", "reopened", "bidding_soon", "highbid"]) {
      expect(screen.getByTestId(`sim-notify-${kind}`)).toBeInTheDocument();
    }
  });

  it("invokes simulate_notification with the clicked kind", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const btn = await screen.findByTestId("sim-notify-bidding");
    fireEvent.click(btn);
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "simulate_notification");
      expect(call?.[1]).toEqual({ kind: "bidding" });
    });
    expect(await screen.findByTestId("debug-notify-status")).toHaveTextContent(/Fired: bidding/i);
  });
})
