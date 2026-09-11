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

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
  loadSettings();
});

describe("Settings — Autostart HSD checkbox", () => {
  it("renders the checkbox checked by default (DEFAULT_SETTINGS.autostart_hsd = 'true')", async () => {
    renderSettings();
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    expect(box).toBeChecked();
    // The label text is visible next to the checkbox.
    expect(screen.getByText(/Autostart HSD when the app launches/i)).toBeInTheDocument();
  });

  it("renders unchecked when the setting is 'false'", async () => {
    loadSettings({ autostart_hsd: "false" });
    renderSettings();
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    expect(box).not.toBeChecked();
  });

  it("persists a toggle to 'false' via update_setting when Save is clicked", async () => {
    renderSettings();
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    fireEvent.click(box); // "true" -> "false"
    expect(box).not.toBeChecked();

    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "autostart_hsd",
      );
      expect(call?.[1]).toEqual({ key: "autostart_hsd", value: "false" });
    });
  });
});
