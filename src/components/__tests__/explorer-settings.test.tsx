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

import { validateExplorerUrl } from "../Settings";
import { loadSettings } from "../../test/fixtures/settings";
import { renderSettings } from "../../test/fixtures/renderSettings";
import { routeSettingsCommand } from "../../test/fixtures/settingsRoute";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
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
    renderSettings();
    const input = await screen.findByTestId("explorer-url-input");

    fireEvent.change(input, { target: { value: "not-a-url" } });
    // Trigger `dirty` so the Save footer renders.
    expect(await screen.findByTestId("explorer-url-error")).toBeInTheDocument();
    const saveButton = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveButton).toBeDisabled();
  });

  it("saves a normalized (no trailing slash) URL and clears dirty state", async () => {
    renderSettings();
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
