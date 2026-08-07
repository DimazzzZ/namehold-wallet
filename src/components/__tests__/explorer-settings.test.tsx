/**
 * Settings — explorer base URL: config + validation + factory usage
 * (Task 11 / S1).
 *
 * The backend already builds explorer requests as `${explorer_api_url}/api/...`
 * (see `providers::explorer_client_from_settings`), so a value without an
 * `http(s)://` scheme would silently break every explorer call. This covers
 * the one bit of non-trivial UI logic here: client-side validation blocking
 * Save on a malformed URL, and a normal save persisting a normalized value.
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

import { Settings, validateExplorerUrl } from "../Settings";
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
      return Promise.resolve({ signerUnlocked: false, broadcasterAvailable: false, canWrite: false, reason: null });
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
      watchlist_notify_enabled: "false",
      watchlist_notify_bidding_soon_lead_blocks: "144",
      watchlist_notify_highest_bid_threshold_hns: "",
      background_sync_enabled: "1",
      node_mode: "full",
      explorer_fallback_url: "",
      chain_source: "local_node",
      ...over,
    },
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
  loadSettings();
});

describe("validateExplorerUrl (unit)", () => {
  it("accepts empty (falls back to the backend default)", () => {
    expect(validateExplorerUrl("")).toBeNull();
    expect(validateExplorerUrl("   ")).toBeNull();
  });

  it("accepts http:// and https:// URLs", () => {
    expect(validateExplorerUrl("https://e.hnsfans.com")).toBeNull();
    expect(validateExplorerUrl("http://127.0.0.1:8080")).toBeNull();
  });

  it("rejects a URL without a scheme", () => {
    expect(validateExplorerUrl("e.hnsfans.com")).toMatch(/http/i);
  });

  it("rejects a non-http(s) scheme", () => {
    expect(validateExplorerUrl("ftp://e.hnsfans.com")).toMatch(/http/i);
  });
});

describe("Settings — explorer base URL (Task 11 / S1)", () => {
  it("shows an inline error and disables Save for a malformed URL", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const input = await screen.findByTestId("explorer-url-input");

    fireEvent.change(input, { target: { value: "not-a-url" } });
    // Trigger `dirty` so the Save footer renders.
    expect(await screen.findByTestId("explorer-url-error")).toBeInTheDocument();
    const saveButton = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveButton).toBeDisabled();
  });

  it("saves a normalized (no trailing slash) URL and clears dirty state", async () => {
    render(<Settings />, { wrapper: wrapper() });
    const input = await screen.findByTestId("explorer-url-input");

    fireEvent.change(input, { target: { value: "https://my.explorer.example/" } });
    expect(screen.queryByTestId("explorer-url-error")).toBeNull();

    const saveButton = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveButton).not.toBeDisabled();
    fireEvent.click(saveButton);

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "update_setting" && c[1]?.key === "explorer_api_url",
      );
      expect(call?.[1]?.value).toBe("https://my.explorer.example");
    });
  });
});
