/**
 * Settings — Shakedex experimental toggle. A regular form field: it is saved
 * through `update_setting` with the Save button, like the other plain toggles.
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

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
  loadSettings();
});

describe("Settings — Shakedex experimental toggle", () => {
  it("is off by default", async () => {
    renderSettings("advanced");
    expect(await screen.findByTestId("shakedex-experimental-checkbox")).not.toBeChecked();
  });

  it("names buying and selling, and what it does not gate", async () => {
    renderSettings("advanced");
    await screen.findByTestId("shakedex-experimental-checkbox");
    expect(
      screen.getByText("Allow buying and selling names on mainnet through Shakedex (experimental)"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Testnet and regtest are always allowed. Finalizing a purchase, and Finalize & sign, Cancel and Lower price on a listing you already have, work whatever this says.",
      ),
    ).toBeInTheDocument();
  });

  it("reflects a stored 'true'", async () => {
    loadSettings({ shakedex_experimental: "true" });
    renderSettings("advanced");
    expect(await screen.findByTestId("shakedex-experimental-checkbox")).toBeChecked();
  });

  it("toggles shakedex_experimental and saves it", async () => {
    renderSettings("advanced");
    const box = await screen.findByTestId("shakedex-experimental-checkbox");
    fireEvent.click(box);
    expect(box).toBeChecked();
    fireEvent.click(screen.getByTestId("settings-save"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_setting", {
        key: "shakedex_experimental",
        value: "true",
      }),
    );
  });
});
