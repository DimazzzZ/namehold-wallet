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
      const save = await screen.findByTestId("settings-save");
      expect(save).toBeDisabled();
    },
  );

  it.each(["1", "1000", "50"])("accepts %s and saves it", async (value) => {
    loadSettings();
    renderSettings();
    fireEvent.change(gapInput(), { target: { value } });
    expect(screen.queryByText(RANGE_ERROR)).not.toBeInTheDocument();
    const save = await screen.findByTestId("settings-save");
    expect(save).toBeEnabled();
    fireEvent.click(save);
    await waitFor(() => {
      const call = invokeMock.mock.calls.find(
        (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === "address_gap_limit",
      );
      expect(call?.[1]).toEqual({ key: "address_gap_limit", value });
    });
  });

  describe("a stored value outside the range", () => {
    // Why: the field sits in the collapsed Advanced section; a stored 5000 made
    // every Save disabled with the reason hidden.
    function advanced(): HTMLDetailsElement {
      return screen.getByTestId("settings-advanced").closest("details") as HTMLDetailsElement;
    }

    it("opens Advanced, shows the reason and says why Save is disabled", async () => {
      loadSettings({ address_gap_limit: "5000" });
      renderSettings();
      expect(await screen.findByText(RANGE_ERROR)).toBeVisible();
      expect(advanced().open).toBe(true);
      // Touch another field so the Save footer appears.
      fireEvent.change(screen.getByTestId("settings-fee-rate"), { target: { value: "2000" } });
      const save = await screen.findByTestId("settings-save");
      expect(save).toBeDisabled();
      expect(screen.getByTestId("settings-save-blocked")).toHaveTextContent(RANGE_ERROR);
    });

    it("fixing the field enables Save and drops the reason", async () => {
      loadSettings({ address_gap_limit: "5000" });
      renderSettings();
      fireEvent.change(await screen.findByTestId("settings-gap-limit"), {
        target: { value: "1000" },
      });
      expect(await screen.findByTestId("settings-save")).toBeEnabled();
      expect(screen.queryByTestId("settings-save-blocked")).not.toBeInTheDocument();
    });

    it("keeps Advanced open after the field is fixed", async () => {
      loadSettings({ address_gap_limit: "5000" });
      renderSettings();
      fireEvent.change(await screen.findByTestId("settings-gap-limit"), {
        target: { value: "1000" },
      });
      await screen.findByTestId("settings-save");
      expect(screen.queryByText(RANGE_ERROR)).not.toBeInTheDocument();
      expect(advanced().open).toBe(true);
    });

    it("opens Advanced for an invalid stored fee rate too", async () => {
      loadSettings({ fee_rate_doos_per_kvb: "abc" });
      renderSettings();
      await waitFor(() => expect(advanced().open).toBe(true));
      expect(screen.getByTestId("settings-fee-rate-error")).toBeVisible();
    });

    it("leaves Advanced collapsed when nothing in it is wrong", async () => {
      loadSettings();
      renderSettings();
      await screen.findByTestId("settings-gap-limit");
      expect(advanced().open).toBe(false);
    });
  });
});
