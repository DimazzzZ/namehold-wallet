/**
 * Settings — Exact Authorization header is write-only.
 *
 * The backend redacts `hsrd_authorization` on `get_settings` and instead emits a
 * `__has_hsrd_authorization: "true"` presence marker. The Settings UI must:
 *   - render a masked placeholder when a key is stored,
 *   - NOT clobber the stored secret when the user Saves with the field blank,
 *   - persist a new value when the user actually types one.
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

// Load a settings map that mirrors what `get_settings` returns AFTER redaction:
// no `hsrd_authorization` value, only the `__has_hsrd_authorization` marker.
function loadWithStoredKey() {
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
      // The presence marker the redacted `get_settings` emits when a key is
      // stored server-side. Not part of the Settings type — cast at read time.
      __has_hsrd_authorization: "true",
    } as unknown as ReturnType<typeof useSettingsStore.getState>["settings"],
  });
}

function loadWithoutStoredKey() {
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
    },
  });
}

function apiKeyInput(): HTMLInputElement {
  // The Authorization value Input renders as <input type="password"> with the exact label.
  const label = screen.getByText("Exact Authorization header");
  const input = label.parentElement?.querySelector<HTMLInputElement>('input[type="password"]');
  if (!input) throw new Error("Exact Authorization header input not found");
  return input;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
});

describe("Settings — Exact Authorization header (write-only)", () => {
  it("does not send hsrd_authorization on save when field is blank and key is stored", async () => {
    loadWithStoredKey();
    render(<Settings />, { wrapper: wrapper() });
    // Toggle a checkbox to trigger the dirty state so the Save button appears
    // (without touching the Authorization value field, which is the subject under test).
    fireEvent.click(await screen.findByTestId("autostart-hsrd-checkbox"));
    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      // Some update_setting call must have fired (other fields are saved).
      expect(
        invokeMock.mock.calls.some((c) => c[0] === "update_setting"),
      ).toBe(true);
    });

    // None of the update_setting calls should be for the Authorization value key.
    const apiKeyCall = invokeMock.mock.calls.find(
      (c) =>
        c[0] === "update_setting" &&
        (c[1] as { key?: string })?.key === "hsrd_authorization",
    );
    expect(apiKeyCall).toBeUndefined();
  });

  it("sends hsrd_authorization on save when user typed a new value", async () => {
    loadWithStoredKey();
    render(<Settings />, { wrapper: wrapper() });

    fireEvent.change(apiKeyInput(), { target: { value: "new-secret" } });

    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const apiKeyCall = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" &&
          (c[1] as { key?: string })?.key === "hsrd_authorization",
      );
      expect(apiKeyCall?.[1]).toEqual({
        key: "hsrd_authorization",
        value: "new-secret",
      });
    });
  });

  it("sends hsrd_authorization on save when no key is stored yet (empty field, no marker)", async () => {
    // When neither the value nor the marker are set, the field submits the
    // current empty value (baseline; no drop-on-blank logic applies).
    loadWithoutStoredKey();
    render(<Settings />, { wrapper: wrapper() });
    // Trigger dirty state without touching the Authorization value field.
    fireEvent.click(await screen.findByTestId("autostart-hsrd-checkbox"));
    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const apiKeyCall = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" &&
          (c[1] as { key?: string })?.key === "hsrd_authorization",
      );
      expect(apiKeyCall?.[1]).toEqual({ key: "hsrd_authorization", value: "" });
    });
  });

  it("renders masked placeholder when key is stored", async () => {
    loadWithStoredKey();
    render(<Settings />, { wrapper: wrapper() });
    // The Authorization value input has a masked/'stored' placeholder rather than '(optional)'.
    await waitFor(() => {
      const ph = apiKeyInput().getAttribute("placeholder") ?? "";
      expect(ph).toMatch(/stored/i);
    });
  });
});
