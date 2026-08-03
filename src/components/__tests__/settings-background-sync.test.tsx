/**
 * Settings — Background sync daemon checkbox.
 *
 * The toggle is applied IMMEDIATELY (no Save button) because flipping it has
 * side effects the user should see right away: it spawns or stops the
 * `namehold-syncd` daemon process. The specialized `set_background_sync_enabled`
 * command persists the setting AND performs the spawn/stop — unlike other
 * settings that go through the generic `update_setting` on Save.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
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
  isPermissionGranted: vi.fn().mockResolvedValue(false),
  requestPermission: vi.fn().mockResolvedValue("default"),
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
    case "update_setting":
    case "set_background_sync_enabled":
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
      hsrd_rpc_url: "http://127.0.0.1:12037",
      hsrd_authorization: "",
      hsrd_data_dir: "",
      hsrd_path: "",
      autostart_hsrd: "true",
      explorer_api_url: "https://e.hnsfans.com",
      address_gap_limit: "20",
      signer_session_timeout_seconds: "900",
      advanced_mode: "false",
      onboarding_complete: "true",
      deadline_notify_enabled: "false",
      deadline_notify_reveal_lead_blocks: "144",
      deadline_notify_renewal_lead_days: "30",
      background_sync_enabled: "1",
      ...over,
    },
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
  loadSettings();
});

describe("Settings — Background sync checkbox", () => {
  it("renders the checkbox checked by default (DEFAULT_SETTINGS.background_sync_enabled = '1')", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("background-sync-checkbox");
    expect(box).toBeChecked();
    expect(
      screen.getByText(/Sync in background/i),
    ).toBeInTheDocument();
  });

  it("renders unchecked when the setting is '0'", async () => {
    loadSettings({ background_sync_enabled: "0" });
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("background-sync-checkbox");
    expect(box).not.toBeChecked();
  });

  it("toggles OFF via set_background_sync_enabled immediately (no Save button needed)", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("background-sync-checkbox");
    // Starts checked (default).
    expect(box).toBeChecked();

    fireEvent.click(box);
    expect(box).not.toBeChecked();

    // The specialized command is invoked directly — no Save button click needed.
    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "set_background_sync_enabled",
      );
      expect(call?.[1]).toEqual({ enabled: false });
    });
  });

  it("toggles ON via set_background_sync_enabled immediately", async () => {
    loadSettings({ background_sync_enabled: "0" });
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("background-sync-checkbox");
    expect(box).not.toBeChecked();

    fireEvent.click(box);
    expect(box).toBeChecked();

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "set_background_sync_enabled",
      );
      expect(call?.[1]).toEqual({ enabled: true });
    });
  });
});
