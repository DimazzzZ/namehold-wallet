/**
 * Settings — reconciling "Launch at login" with the OS when the page opens.
 *
 * The OS state wins and is written straight to the database, so opening
 * Settings is not an edit: the save bar stays hidden.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom";
import { fireEvent, screen, waitFor } from "@testing-library/react";

const invokeMock = vi.fn();
const isEnabledMock = vi.fn();
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
  isEnabled: () => isEnabledMock(),
}));

import { loadSettings } from "../../test/fixtures/settings";
import { renderSettings } from "../../test/fixtures/renderSettings";
import { routeSettingsCommand } from "../../test/fixtures/settingsRoute";

const writes = () =>
  invokeMock.mock.calls.filter(
    (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "launch_at_login",
  );

async function openSettings() {
  renderSettings("node");
  await screen.findByTestId("launch-at-login-checkbox");
  await waitFor(() => expect(isEnabledMock).toHaveBeenCalled());
  // Let the reconcile that follows the OS answer finish.
  await new Promise((r) => setTimeout(r, 20));
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
  isEnabledMock.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

afterEach(() => {
  vi.unstubAllEnvs();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe("Settings — launch at login on open", () => {
  it("does nothing when the OS and the database agree", async () => {
    loadSettings({ launch_at_login: "1" });
    isEnabledMock.mockResolvedValue(true);
    await openSettings();
    expect(screen.queryByTestId("settings-save")).not.toBeInTheDocument();
    expect(writes()).toEqual([]);
  });

  it("takes an enabled OS state without showing the save bar", async () => {
    loadSettings({ launch_at_login: "0" });
    isEnabledMock.mockResolvedValue(true);
    await openSettings();
    expect(screen.getByTestId("launch-at-login-checkbox")).toBeChecked();
    expect(writes().map((c) => c[1])).toEqual([{ key: "launch_at_login", value: "1" }]);
    expect(screen.queryByTestId("settings-save")).not.toBeInTheDocument();
  });

  it("takes a disabled OS state without showing the save bar", async () => {
    loadSettings({ launch_at_login: "1" });
    isEnabledMock.mockResolvedValue(false);
    await openSettings();
    expect(screen.getByTestId("launch-at-login-checkbox")).not.toBeChecked();
    expect(writes().map((c) => c[1])).toEqual([{ key: "launch_at_login", value: "0" }]);
    expect(screen.queryByTestId("settings-save")).not.toBeInTheDocument();
  });

  it("applies a toggle at once, with nothing left for Save", async () => {
    vi.stubEnv("DEV", false); // The checkbox is disabled in dev builds.
    loadSettings({ launch_at_login: "0" });
    isEnabledMock.mockResolvedValue(false);
    await openSettings();
    fireEvent.click(screen.getByTestId("launch-at-login-checkbox"));
    await waitFor(() =>
      expect(writes().map((c) => c[1])).toEqual([{ key: "launch_at_login", value: "1" }]),
    );
    expect(screen.getByTestId("launch-at-login-checkbox")).toBeChecked();
    expect(screen.queryByTestId("settings-save")).not.toBeInTheDocument();
  });

  it.each([
    ["background-sync-checkbox", "set_background_sync_enabled"],
    ["close-to-tray-checkbox", "set_close_to_tray_enabled"],
  ])("%s applies at once, with nothing left for Save", async (testId, command) => {
    loadSettings({ launch_at_login: "1" });
    isEnabledMock.mockResolvedValue(true);
    await openSettings();
    fireEvent.click(screen.getByTestId(testId));
    await waitFor(() => expect(invokeMock.mock.calls.some((c) => c[0] === command)).toBe(true));
    expect(screen.queryByTestId("settings-save")).not.toBeInTheDocument();
  });

  it("still shows the save bar for an edit", async () => {
    loadSettings({ launch_at_login: "1" });
    isEnabledMock.mockResolvedValue(true);
    await openSettings();
    fireEvent.click(screen.getByTestId("autostart-hsd-checkbox"));
    expect(await screen.findByTestId("settings-save")).toBeInTheDocument();
  });
});
