/**
 * Settings — Signer session timeout (Advanced).
 *
 * The timeout is a protected setting in the range the session uses
 * (noncustodial/session.rs::session_ttl_ms clamps anything else):
 * `update_protected_settings` refuses a value outside it and shows a valid
 * change in the secure window. The field refuses what the backend would, with
 * the same rule (`sessionTimeoutError`), so Save stays disabled until it is
 * fixed, and a valid change goes to the confirmed command, never `update_setting`.
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
import { MAX_SIGNER_SESSION_SECS, MIN_SIGNER_SESSION_SECS } from "../../lib/settingsDefaults";

const RANGE_ERROR = "Signer session timeout must be a whole number of seconds from 60 to 3600";

function timeoutInput(): HTMLInputElement {
  return screen.getByTestId("settings-session-timeout") as HTMLInputElement;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
});

describe("Settings — Signer session timeout", () => {
  it("the backend's range is 60 to 3600 seconds", () => {
    expect(MIN_SIGNER_SESSION_SECS).toBe(60);
    expect(MAX_SIGNER_SESSION_SECS).toBe(3600);
  });

  it.each(["0", "59", "3601", "18446744073709551615", "abc", "1.5", "-5"])(
    "refuses %s with the range and disables Save",
    async (value) => {
      loadSettings();
      renderSettings("advanced");
      fireEvent.change(timeoutInput(), { target: { value } });
      expect(await screen.findByTestId("settings-session-timeout-error")).toHaveTextContent(
        RANGE_ERROR,
      );
      expect(await screen.findByTestId("settings-save")).toBeDisabled();
      expect(screen.getByTestId("settings-save-blocked")).toHaveTextContent(RANGE_ERROR);
    },
  );

  it.each(["60", "300", "3600"])("sends %s to be confirmed in the secure window", async (value) => {
    loadSettings();
    renderSettings("advanced");
    fireEvent.change(timeoutInput(), { target: { value } });
    expect(screen.queryByTestId("settings-session-timeout-error")).not.toBeInTheDocument();
    fireEvent.click(await screen.findByTestId("settings-save"));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", {
        changes: [{ key: "signer_session_timeout_seconds", value }],
      }),
    );
    expect(
      invokeMock.mock.calls.some(
        (c) =>
          c[0] === "update_setting" &&
          (c[1] as { key?: string })?.key === "signer_session_timeout_seconds",
      ),
    ).toBe(false);
  });

  it("a stored timeout past the cap marks Advanced and says why Save is disabled", async () => {
    loadSettings({ signer_session_timeout_seconds: "86400" });
    renderSettings("advanced");
    expect(await screen.findByTestId("settings-session-timeout-error")).toBeVisible();
    expect(screen.getByTestId("settings-section-error-advanced")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("settings-fee-rate"), { target: { value: "2000" } });
    expect(await screen.findByTestId("settings-save")).toBeDisabled();
    expect(screen.getByTestId("settings-save-blocked")).toHaveTextContent(RANGE_ERROR);
  });
});
