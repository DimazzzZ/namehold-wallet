/**
 * Settings controls for the deadline notification scanner (I1 / Task 4).
 *
 * Covers the one bit of non-trivial UI logic: the enable toggle drives an
 * immediate OS permission request (must fire from THIS user gesture — a
 * deferred request after Save would silently fail on macOS), and the result
 * (granted/denied/unsupported) is reflected back in the UI. Lead-time input
 * binding is plain `updateField` wiring, already covered by the same pattern
 * elsewhere, so it isn't re-tested in detail here.
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

const isPermissionGrantedMock = vi.fn();
const requestPermissionMock = vi.fn();
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: (...a: unknown[]) => isPermissionGrantedMock(...a),
  requestPermission: (...a: unknown[]) => requestPermissionMock(...a),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({ enable: vi.fn().mockResolvedValue(undefined), disable: vi.fn().mockResolvedValue(undefined), isEnabled: vi.fn().mockResolvedValue(false) }));

import { Settings } from "../Settings";
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

function nodeStatus() {
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
    last_error: null,
    index_mismatch: false,
    read_source: "explorer",
  };
}

function route(cmd: string) {
  switch (cmd) {
    case "node_status":
      return Promise.resolve(nodeStatus());
    case "list_wallet_profiles":
      return Promise.resolve([profile]);
    case "get_signer_session":
      return Promise.resolve({ walletProfileId: null, unlocked: false, unlockedUntilEpochMs: 0 });
    case "get_write_capability":
      return Promise.resolve({ signerUnlocked: false, broadcasterAvailable: false, canWrite: false, reason: null });
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
  isPermissionGrantedMock.mockReset();
  requestPermissionMock.mockReset();
  loadSettings();
});

describe("Settings — deadline notifications (I1 / Task 4)", () => {
  it("is off by default and hides lead-time inputs", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const toggle = await screen.findByTestId("deadline-notify-toggle");
    expect(toggle).not.toBeChecked();
    expect(screen.queryByText(/Reveal window lead time/i)).toBeNull();
  });

  it("requests OS permission the moment the toggle is turned on, and shows lead-time inputs once granted", async () => {
    isPermissionGrantedMock.mockResolvedValue(false);
    requestPermissionMock.mockResolvedValue("granted");
    render(<Settings />, { wrapper: wrapper() });

    const toggle = await screen.findByTestId("deadline-notify-toggle");
    fireEvent.click(toggle);

    await waitFor(() => expect(requestPermissionMock).toHaveBeenCalled());
    expect(await screen.findByText(/Reveal window lead time/i)).toBeInTheDocument();
    expect(screen.queryByTestId("notification-permission-denied")).toBeNull();
  });

  it("shows a non-blocking warning (not a crash) when the OS denies permission", async () => {
    isPermissionGrantedMock.mockResolvedValue(false);
    requestPermissionMock.mockResolvedValue("denied");
    render(<Settings />, { wrapper: wrapper() });

    const toggle = await screen.findByTestId("deadline-notify-toggle");
    fireEvent.click(toggle);

    expect(await screen.findByTestId("notification-permission-denied")).toBeInTheDocument();
    // The toggle itself stays on — denial degrades gracefully, it doesn't
    // silently revert the user's choice.
    expect(toggle).toBeChecked();
    // Lead-time inputs are still usable even without OS delivery.
    expect(screen.getByText(/Reveal window lead time/i)).toBeInTheDocument();
  });

  it("does not show a blocked warning when permission has simply never been decided yet (review Minor 8)", async () => {
    // The plugin's `isPermissionGranted()` command can legitimately resolve
    // `null` for "not yet determined" (see `checkNotificationPermission`'s
    // doc comment) — a naive falsy-coercion would show the SAME "blocked"
    // warning as an explicit denial. It must not.
    loadSettings({ deadline_notify_enabled: "true" });
    isPermissionGrantedMock.mockResolvedValue(null);
    render(<Settings />, { wrapper: wrapper() });

    await screen.findByText(/Reveal window lead time/i);
    expect(screen.queryByTestId("notification-permission-denied")).toBeNull();
  });

  it("does not prompt for permission again if already granted", async () => {
    loadSettings({ deadline_notify_enabled: "true" });
    isPermissionGrantedMock.mockResolvedValue(true);
    render(<Settings />, { wrapper: wrapper() });

    await screen.findByText(/Reveal window lead time/i);
    expect(requestPermissionMock).not.toHaveBeenCalled();
  });

  it("turning the toggle off does not request permission", async () => {
    loadSettings({ deadline_notify_enabled: "true" });
    isPermissionGrantedMock.mockResolvedValue(true);
    render(<Settings />, { wrapper: wrapper() });

    const toggle = await screen.findByTestId("deadline-notify-toggle");
    await waitFor(() => expect(toggle).toBeChecked());
    fireEvent.click(toggle);

    expect(toggle).not.toBeChecked();
    expect(requestPermissionMock).not.toHaveBeenCalled();
  });
});
