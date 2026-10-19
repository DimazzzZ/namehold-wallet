/**
 * Settings — Address gap limit (Advanced).
 *
 * The backend reads the setting in 1..=MAX_GAP_LIMIT (secure_wallet.rs) and
 * clamps a larger value: the field refuses what the backend would not use as
 * typed, with the same range, and Save stays disabled until it is fixed.
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
import { MAX_ADDRESS_GAP_LIMIT } from "../../lib/settingsDefaults";

const RANGE_ERROR = "Address gap limit must be a whole number from 1 to 1000";

function gapInput(): HTMLInputElement {
  return screen.getByTestId("settings-gap-limit") as HTMLInputElement;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
});

describe("Settings — Address gap limit", () => {
  it("the backend's cap is 1000", () => {
    expect(MAX_ADDRESS_GAP_LIMIT).toBe(1000);
  });

  it.each(["0", "1001", "4294967296", "abc", "1.5", "-5"])(
    "refuses %s with the range and disables Save",
    async (value) => {
      loadSettings();
      renderSettings();
      fireEvent.change(gapInput(), { target: { value } });
      expect(await screen.findByText(RANGE_ERROR)).toBeInTheDocument();
      const save = await screen.findByRole("button", { name: /Save settings/i });
      expect(save).toBeDisabled();
    },
  );

  it.each(["1", "1000", "50"])("accepts %s and saves it", async (value) => {
    loadSettings();
    renderSettings();
    fireEvent.change(gapInput(), { target: { value } });
    expect(screen.queryByText(RANGE_ERROR)).not.toBeInTheDocument();
    const save = await screen.findByRole("button", { name: /Save settings/i });
    expect(save).toBeEnabled();
    fireEvent.click(save);
    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "address_gap_limit",
      );
      expect(call?.[1]).toEqual({ key: "address_gap_limit", value });
    });
  });
});
