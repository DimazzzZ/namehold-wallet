/**
 * Settings — Fee rate override field (global default for all transactions).
 *
 * The fee-rate field validates input as a whole number of doos/kvB.
 * Empty input is valid (clears the override). Non-numeric input shows an error
 * and disables the Save button.
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

function loadSettings(feeRate: string = "") {
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
      fee_rate_doos_per_kvb: feeRate,
    },
  });
}

function feeRateInput(): HTMLInputElement {
  return screen.getByTestId("settings-fee-rate") as HTMLInputElement;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
});

describe("Settings — Fee rate override", () => {
  it("renders fee-rate input with correct initial value", async () => {
    loadSettings("5000");
    render(<Settings />, { wrapper: wrapper() });

    const input = await screen.findByTestId("settings-fee-rate");
    expect(input).toHaveValue("5000");
  });

  it("shows error when input is non-numeric", async () => {
    loadSettings("");
    render(<Settings />, { wrapper: wrapper() });

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "abc" } });

    // Error message should appear
    await waitFor(() => {
      expect(screen.getByText(/Fee rate must be a whole number/i)).toBeInTheDocument();
    });
  });

  it("clears error when input is valid", async () => {
    loadSettings("");
    render(<Settings />, { wrapper: wrapper() });

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "abc" } });

    // Error appears
    await screen.findByText(/Fee rate must be a whole number/i);

    // Change to valid value
    fireEvent.change(input, { target: { value: "4000" } });

    // Error should disappear
    await waitFor(() => {
      expect(screen.queryByText(/Fee rate must be a whole number/i)).not.toBeInTheDocument();
    });
  });

  it("disables Save button when error is present", async () => {
    loadSettings("");
    render(<Settings />, { wrapper: wrapper() });

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "not-a-number" } });

    await screen.findByText(/Fee rate must be a whole number/i);

    // Trigger dirty state by changing another field
    fireEvent.click(await screen.findByTestId("autostart-hsd-checkbox"));

    // Save button should be disabled
    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveBtn).toBeDisabled();
  });

  it("enables Save button when input is valid", async () => {
    loadSettings("");
    render(<Settings />, { wrapper: wrapper() });

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "4000" } });

    // Save button should be enabled
    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveBtn).toBeEnabled();
  });

  it("sends update_setting call when saving valid fee rate", async () => {
    loadSettings("");
    render(<Settings />, { wrapper: wrapper() });

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "3500" } });

    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      const feeRateCall = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" &&
          (c[1] as { key?: string })?.key === "fee_rate_doos_per_kvb",
      );
      expect(feeRateCall?.[1]).toEqual({
        key: "fee_rate_doos_per_kvb",
        value: "3500",
      });
    });
  });

  it("allows empty fee rate (clears the override)", async () => {
    loadSettings("5000");
    render(<Settings />, { wrapper: wrapper() });

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "" } });

    // No error should appear for empty input
    expect(screen.queryByText(/Fee rate must be a whole number/i)).not.toBeInTheDocument();

    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      const feeRateCall = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" &&
          (c[1] as { key?: string })?.key === "fee_rate_doos_per_kvb",
      );
      expect(feeRateCall?.[1]).toEqual({
        key: "fee_rate_doos_per_kvb",
        value: "",
      });
    });
  });
});
