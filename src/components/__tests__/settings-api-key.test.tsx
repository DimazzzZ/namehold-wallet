/**
 * Settings — Node RPC api-key is write-only.
 *
 * The backend redacts `node_rpc_api_key` on `get_settings` and instead emits a
 * `__has_node_rpc_api_key: "true"` presence marker. The Settings UI must:
 *   - render a masked placeholder when a key is stored,
 *   - NOT clobber the stored secret when the user Saves with the field blank,
 *   - send a new value, when the user actually types one, to
 *     `update_protected_settings` (confirmed in the secure window), and
 *     treat a declined confirmation as "not saved" rather than an error.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { screen, waitFor, fireEvent } from "@testing-library/react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
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
import { useUiStore } from "../../stores/ui";

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
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("Settings — Node RPC api-key (write-only)", () => {
  it("does not send node_rpc_api_key on save when field is blank and key is stored", async () => {
    loadWithStoredKey();
    renderSettings();
    // Toggle a checkbox to trigger the dirty state so the Save button appears
    // (without touching the api-key field, which is the subject under test).
    fireEvent.click(screen.getByTestId("settings-section-node"));
    fireEvent.click(await screen.findByTestId("autostart-hsd-checkbox"));
    const save = await screen.findByTestId("settings-save");
    fireEvent.click(save);

    await waitFor(() => {
      // Some update_setting call must have fired (other fields are saved).
      expect(invokeMock.mock.calls.some((c) => c[0] === "update_setting")).toBe(true);
    });

    // Nothing protected changed, so nothing goes to the secure window, and
    // the key is never written through the plain command.
    expect(invokeMock).not.toHaveBeenCalledWith("update_protected_settings", expect.anything());
    const apiKeyCall = invokeMock.mock.calls.find(
      (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "node_rpc_api_key",
    );
    expect(apiKeyCall).toBeUndefined();
  });

  it("sends a typed node_rpc_api_key to be confirmed in the secure window", async () => {
    loadWithStoredKey();
    renderSettings();

    fireEvent.change(apiKeyInput(), { target: { value: "new-secret" } });

    const save = await screen.findByTestId("settings-save");
    fireEvent.click(save);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", {
        changes: [{ key: "node_rpc_api_key", value: "new-secret" }],
      }),
    );
    const apiKeyCall = invokeMock.mock.calls.find(
      (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "node_rpc_api_key",
    );
    expect(apiKeyCall).toBeUndefined();
  });

  it("does not send an API key field left empty when none is stored", async () => {
    // Unchanged from what is stored (an empty key), so there is nothing to
    // confirm: the key is not sent at all.
    loadWithoutStoredKey();
    renderSettings();
    // Trigger dirty state without touching the api-key field.
    fireEvent.click(screen.getByTestId("settings-section-node"));
    fireEvent.click(await screen.findByTestId("autostart-hsd-checkbox"));
    const save = await screen.findByTestId("settings-save");
    fireEvent.click(save);

    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "update_setting")).toBe(true),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("update_protected_settings", expect.anything());
    expect(
      invokeMock.mock.calls.some(
        (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "node_rpc_api_key",
      ),
    ).toBe(false);
  });

  it("a declined confirmation saves nothing, shows no error and keeps the form unsaved", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "update_protected_settings"
        ? Promise.reject("Confirmation declined")
        : routeSettingsCommand(cmd),
    );
    loadWithStoredKey();
    renderSettings();

    fireEvent.change(apiKeyInput(), { target: { value: "new-secret" } });
    fireEvent.click(await screen.findByTestId("settings-save"));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", expect.anything()),
    );
    await waitFor(() => expect(screen.getByTestId("settings-save")).not.toBeDisabled());
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
    expect(useUiStore.getState().toastQueue.map((t) => t.message)).toEqual([]);
  });

  it("any other refusal is shown as an error and saves nothing else", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "update_protected_settings"
        ? Promise.reject("Invalid input: setting 'node_rpc_url' must be an http:// or https:// URL")
        : routeSettingsCommand(cmd),
    );
    loadWithStoredKey();
    renderSettings();

    fireEvent.change(apiKeyInput(), { target: { value: "new-secret" } });
    fireEvent.click(await screen.findByTestId("settings-save"));

    await waitFor(() =>
      expect(useUiStore.getState().toastQueue.map((t) => t.message)).toEqual([
        "Failed to save: Invalid input: setting 'node_rpc_url' must be an http:// or https:// URL",
      ]),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
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
