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
      renderSettings("advanced");
      fireEvent.change(gapInput(), { target: { value } });
      expect(await screen.findByText(RANGE_ERROR)).toBeInTheDocument();
      const save = await screen.findByTestId("settings-save");
      expect(save).toBeDisabled();
    },
  );

  it.each(["1", "1000", "50"])("accepts %s and saves it", async (value) => {
    loadSettings();
    renderSettings("advanced");
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
    // Why: the field sits in the Advanced section; a stored 5000 made every
    // Save disabled with the reason out of sight. The section's nav entry
    // carries a red dot, and the save bar names the reason with a Show link.
    function advancedDot(): HTMLElement | null {
      return screen.queryByTestId("settings-section-error-advanced");
    }

    it("marks Advanced, shows the reason and says why Save is disabled", async () => {
      loadSettings({ address_gap_limit: "5000" });
      renderSettings("advanced");
      expect(await screen.findByText(RANGE_ERROR)).toBeVisible();
      expect(screen.getByTestId("settings-advanced")).toBeVisible();
      expect(advancedDot()).toBeInTheDocument();
      // Touch another field so the Save footer appears.
      fireEvent.change(screen.getByTestId("settings-fee-rate"), { target: { value: "2000" } });
      const save = await screen.findByTestId("settings-save");
      expect(save).toBeDisabled();
      expect(screen.getByTestId("settings-save-blocked")).toHaveTextContent(RANGE_ERROR);
      // Already in the section that holds it: no Show link.
      expect(screen.queryByTestId("settings-save-blocked-show")).not.toBeInTheDocument();
    });

    it("fixing the field enables Save and drops the reason", async () => {
      loadSettings({ address_gap_limit: "5000" });
      renderSettings("advanced");
      fireEvent.change(await screen.findByTestId("settings-gap-limit"), {
        target: { value: "1000" },
      });
      expect(await screen.findByTestId("settings-save")).toBeEnabled();
      expect(screen.queryByTestId("settings-save-blocked")).not.toBeInTheDocument();
    });

    it("keeps Advanced shown and drops its red dot after the field is fixed", async () => {
      loadSettings({ address_gap_limit: "5000" });
      renderSettings("advanced");
      fireEvent.change(await screen.findByTestId("settings-gap-limit"), {
        target: { value: "1000" },
      });
      await screen.findByTestId("settings-save");
      expect(screen.queryByText(RANGE_ERROR)).not.toBeInTheDocument();
      expect(screen.getByTestId("settings-advanced")).toBeVisible();
      expect(advancedDot()).not.toBeInTheDocument();
    });

    it("leads to Advanced from another section for an invalid stored fee rate too", async () => {
      loadSettings({ fee_rate_doos_per_kvb: "abc" });
      renderSettings("connections");
      await waitFor(() => expect(advancedDot()).toBeInTheDocument());
      // Touch a Connections field so the Save footer appears.
      fireEvent.change(await screen.findByTestId("explorer-fallback-url-input"), {
        target: { value: "https://fallback.example" },
      });
      expect(await screen.findByTestId("settings-save")).toBeDisabled();
      fireEvent.click(screen.getByTestId("settings-save-blocked-show"));
      expect(await screen.findByTestId("settings-fee-rate-error")).toBeVisible();
      expect(screen.getByTestId("settings-section-advanced")).toHaveAttribute(
        "aria-current",
        "page",
      );
      expect(screen.queryByTestId("settings-save-blocked-show")).not.toBeInTheDocument();
    });

    it("leaves Advanced unmarked when nothing in it is wrong", async () => {
      loadSettings();
      renderSettings("advanced");
      await screen.findByTestId("settings-gap-limit");
      expect(advancedDot()).not.toBeInTheDocument();
    });
  });
});
