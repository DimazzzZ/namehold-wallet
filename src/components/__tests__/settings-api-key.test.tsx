/**
 * Settings — Node RPC api-key is write-only.
 *
 * The backend redacts `node_rpc_api_key` on `get_settings` and instead emits a
 * `__has_node_rpc_api_key: "true"` presence marker. The Settings UI must:
 *   - render a masked placeholder when a key is stored,
 *   - NOT clobber the stored secret when the user Saves with the field blank,
 *   - persist a new value when the user actually types one.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { screen, waitFor, fireEvent } from "@testing-library/react";

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

import { loadSettings } from "../../test/fixtures/settings";
import { renderSettings } from "../../test/fixtures/renderSettings";
import { routeSettingsCommand } from "../../test/fixtures/settingsRoute";

// Load a settings map that mirrors what `get_settings` returns AFTER redaction:
// no `node_rpc_api_key` value, only the `__has_node_rpc_api_key` marker.
function loadWithStoredKey() {
  loadSettings({ __has_node_rpc_api_key: "true" });
}

function loadWithoutStoredKey() {
  loadSettings();
}

function apiKeyInput(): HTMLInputElement {
  // The api-key Input renders as <input type="password"> with the exact label.
  const label = screen.getByText("Node RPC API key");
  const input = label.parentElement?.querySelector<HTMLInputElement>('input[type="password"]');
  if (!input) throw new Error("Node RPC API key input not found");
  return input;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
});

describe("Settings — Node RPC api-key (write-only)", () => {
  it("does not send node_rpc_api_key on save when field is blank and key is stored", async () => {
    loadWithStoredKey();
    renderSettings();
    // Toggle a checkbox to trigger the dirty state so the Save button appears
    // (without touching the api-key field, which is the subject under test).
    fireEvent.click(await screen.findByTestId("autostart-hsd-checkbox"));
    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      // Some update_setting call must have fired (other fields are saved).
      expect(invokeMock.mock.calls.some((c) => c[0] === "update_setting")).toBe(true);
    });

    // None of the update_setting calls should be for the api-key key.
    const apiKeyCall = invokeMock.mock.calls.find(
      (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "node_rpc_api_key",
    );
    expect(apiKeyCall).toBeUndefined();
  });

  it("sends node_rpc_api_key on save when user typed a new value", async () => {
    loadWithStoredKey();
    renderSettings();

    fireEvent.change(apiKeyInput(), { target: { value: "new-secret" } });

    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const apiKeyCall = invokeMock.mock.calls.find(
        (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "node_rpc_api_key",
      );
      expect(apiKeyCall?.[1]).toEqual({
        key: "node_rpc_api_key",
        value: "new-secret",
      });
    });
  });

  it("sends node_rpc_api_key on save when no key is stored yet (empty field, no marker)", async () => {
    // When neither the value nor the marker are set, the field submits the
    // current empty value (baseline; no drop-on-blank logic applies).
    loadWithoutStoredKey();
    renderSettings();
    // Trigger dirty state without touching the api-key field.
    fireEvent.click(await screen.findByTestId("autostart-hsd-checkbox"));
    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const apiKeyCall = invokeMock.mock.calls.find(
        (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "node_rpc_api_key",
      );
      expect(apiKeyCall?.[1]).toEqual({ key: "node_rpc_api_key", value: "" });
    });
  });

  it("renders masked placeholder when key is stored", async () => {
    loadWithStoredKey();
    renderSettings();
    // The api-key input has a masked/'stored' placeholder rather than '(optional)'.
    await waitFor(() => {
      const ph = apiKeyInput().getAttribute("placeholder") ?? "";
      expect(ph).toMatch(/stored/i);
    });
  });
});
