/**
 * Settings — Autostart HSD checkbox.
 *
 * The Rust setup hook reads `autostart_hsd` from the SQLite settings table on
 * app launch to decide whether to spawn hsd automatically. Here we only cover
 * the frontend surface: default value, render/toggle, and that Save persists
 * the change via `update_setting`.
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
      advanced_mode: "false",
      onboarding_complete: "true",
      deadline_notify_enabled: "false",
      deadline_notify_reveal_lead_blocks: "144",
      deadline_notify_renewal_lead_days: "30",
      background_sync_enabled: "1",
      node_mode: "full",
      explorer_fallback_url: "",
      ...over,
    },
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
  loadSettings();
});

describe("Settings — Autostart HSD checkbox", () => {
  it("renders the checkbox checked by default (DEFAULT_SETTINGS.autostart_hsd = 'true')", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    expect(box).toBeChecked();
    // The label text is visible next to the checkbox.
    expect(
      screen.getByText(/Autostart HSD when the app launches/i),
    ).toBeInTheDocument();
  });

  it("renders unchecked when the setting is 'false'", async () => {
    loadSettings({ autostart_hsd: "false" });
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    expect(box).not.toBeChecked();
  });

  it("persists a toggle to 'false' via update_setting when Save is clicked", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    fireEvent.click(box); // "true" -> "false"
    expect(box).not.toBeChecked();

    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" &&
          (c[1] as { key?: string })?.key === "autostart_hsd",
      );
      expect(call?.[1]).toEqual({ key: "autostart_hsd", value: "false" });
    });
  });
});
