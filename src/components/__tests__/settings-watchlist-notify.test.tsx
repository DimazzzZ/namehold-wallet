/**
 * Settings — Watchlist notifications section.
 *
 * Verifies the three keys (`watchlist_notify_enabled`,
 * `watchlist_notify_bidding_soon_lead_blocks`,
 * `watchlist_notify_highest_bid_threshold_hns`) round-trip through the
 * Settings form and the underlying `update_setting` command.
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
  requestPermission: vi.fn().mockResolvedValue("granted"),
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

describe("Settings — Watchlist notifications", () => {
  it("renders the enable toggle unchecked by default", async () => {
    renderSettings();
    const box = await screen.findByTestId("watchlist-notify-toggle");
    expect(box).not.toBeChecked();
  });

  it("shows the lead-time and threshold inputs only when enabled", async () => {
    loadSettings({ watchlist_notify_enabled: "true" });
    renderSettings();
    expect(await screen.findByTestId("watchlist-notify-bidding-lead-input")).toBeInTheDocument();
    expect(await screen.findByTestId("watchlist-notify-highbid-input")).toBeInTheDocument();
  });

  it("persists all three keys via update_setting when Save is clicked", async () => {
    renderSettings();

    // Enable
    const toggle = await screen.findByTestId("watchlist-notify-toggle");
    fireEvent.click(toggle);

    // Change the two numeric fields.
    const leadInput = await screen.findByTestId("watchlist-notify-bidding-lead-input");
    fireEvent.change(leadInput, { target: { value: "72" } });
    const highbidInput = await screen.findByTestId("watchlist-notify-highbid-input");
    fireEvent.change(highbidInput, { target: { value: "250" } });

    const save = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(save);

    await waitFor(() => {
      const findCall = (key: string) =>
        invokeMock.mock.calls.find(
          (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === key,
        );
      expect(findCall("watchlist_notify_enabled")?.[1]).toEqual({
        key: "watchlist_notify_enabled",
        value: "true",
      });
      expect(findCall("watchlist_notify_bidding_soon_lead_blocks")?.[1]).toEqual({
        key: "watchlist_notify_bidding_soon_lead_blocks",
        value: "72",
      });
      expect(findCall("watchlist_notify_highest_bid_threshold_hns")?.[1]).toEqual({
        key: "watchlist_notify_highest_bid_threshold_hns",
        value: "250",
      });
    });
  });
});
