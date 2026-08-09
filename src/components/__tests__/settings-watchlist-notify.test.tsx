/**
 * Settings — Watchlist notifications section.
 *
 * Verifies the three keys (`watchlist_notify_enabled`,
 * `watchlist_notify_bidding_soon_lead_blocks`,
 * `watchlist_notify_highest_bid_threshold_hns`) round-trip through the
 * Settings form and the underlying `update_setting` command.
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
  requestPermission: vi.fn().mockResolvedValue("granted"),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({ enable: vi.fn().mockResolvedValue(undefined), disable: vi.fn().mockResolvedValue(undefined), isEnabled: vi.fn().mockResolvedValue(false) }));

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
      tray_hint_shown: "0",
      launch_at_login: "0",
      ...over,
    },
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
  loadSettings();
});

describe("Settings — Watchlist notifications", () => {
  it("renders the enable toggle unchecked by default", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const box = await screen.findByTestId("watchlist-notify-toggle");
    expect(box).not.toBeChecked();
  });

  it("shows the lead-time and threshold inputs only when enabled", async () => {
    loadSettings({ watchlist_notify_enabled: "true" });
    render(<Settings />, { wrapper: wrapper() });
    expect(await screen.findByTestId("watchlist-notify-bidding-lead-input")).toBeInTheDocument();
    expect(await screen.findByTestId("watchlist-notify-highbid-input")).toBeInTheDocument();
  });

  it("persists all three keys via update_setting when Save is clicked", async () => {
    render(<Settings />, { wrapper: wrapper() });

    // Enable
    const toggle = await screen.findByTestId("watchlist-notify-toggle");
    fireEvent.click(toggle);

    // Change the two numeric fields.
    const leadInput = await screen.findByTestId("watchlist-notify-bidding-lead-input");
    fireEvent.change(leadInput, { target: { value: "72" } });
    const highbidInput = await screen.findByTestId("watchlist-notify-highbid-input");
    fireEvent.change(highbidInput, { target: { value: "250" } });

    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const findCall = (key: string) =>
        invokeMock.mock.calls.find(
          (c) =>
            c[0] === "update_setting" &&
            (c[1] as { key?: string })?.key === key,
        );
      expect(findCall("watchlist_notify_enabled")?.[1]).toEqual({
        key: "watchlist_notify_enabled",
        value: "true",
      });
      expect(findCall("watchlist_notify_bidding_soon_lead_blocks")?.[1]).toEqual({
        key: "watchlist_notify_bidding_soon_lead_blocks",
        value: "72",
      });
      expect(findCall("watchlist_notify_highest_bid_threshold_hns")?.[1]).toEqual({
        key: "watchlist_notify_highest_bid_threshold_hns",
        value: "250",
      });
    });
  });
});
