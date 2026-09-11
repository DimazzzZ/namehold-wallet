/**
 * Settings — Fee rate override field (global default for all transactions).
 *
 * The fee-rate field validates input as a whole number of doos/kvB.
 * Empty input is valid (clears the override). Non-numeric input shows an error
 * and disables the Save button.
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

function feeRateInput(): HTMLInputElement {
  return screen.getByTestId("settings-fee-rate") as HTMLInputElement;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(routeSettingsCommand);
});

describe("Settings — Fee rate override", () => {
  it("renders fee-rate input with correct initial value", async () => {
    loadSettings({ fee_rate_doos_per_kvb: "5000" });
    renderSettings();

    const input = await screen.findByTestId("settings-fee-rate");
    expect(input).toHaveValue("5000");
  });

  it("shows error when input is non-numeric", async () => {
    loadSettings();
    renderSettings();

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "abc" } });

    // Error message should appear
    await waitFor(() => {
      expect(screen.getByText(/Fee rate must be a whole number/i)).toBeInTheDocument();
    });
  });

  it("clears error when input is valid", async () => {
    loadSettings();
    renderSettings();

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "abc" } });

    // Error appears
    await screen.findByText(/Fee rate must be a whole number/i);

    // Change to valid value
    fireEvent.change(input, { target: { value: "4000" } });

    // Error should disappear
    await waitFor(() => {
      expect(screen.queryByText(/Fee rate must be a whole number/i)).not.toBeInTheDocument();
    });
  });

  it("disables Save button when error is present", async () => {
    loadSettings();
    renderSettings();

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "not-a-number" } });

    await screen.findByText(/Fee rate must be a whole number/i);

    // Trigger dirty state by changing another field
    fireEvent.click(await screen.findByTestId("autostart-hsd-checkbox"));

    // Save button should be disabled
    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveBtn).toBeDisabled();
  });

  it("enables Save button when input is valid", async () => {
    loadSettings();
    renderSettings();

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "4000" } });

    // Save button should be enabled
    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    expect(saveBtn).toBeEnabled();
  });

  it("sends update_setting call when saving valid fee rate", async () => {
    loadSettings();
    renderSettings();

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "3500" } });

    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      const feeRateCall = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" && (c[1] as { key?: string })?.key === "fee_rate_doos_per_kvb",
      );
      expect(feeRateCall?.[1]).toEqual({
        key: "fee_rate_doos_per_kvb",
        value: "3500",
      });
    });
  });

  it("allows empty fee rate (clears the override)", async () => {
    loadSettings({ fee_rate_doos_per_kvb: "5000" });
    renderSettings();

    const input = feeRateInput();
    fireEvent.change(input, { target: { value: "" } });

    // No error should appear for empty input
    expect(screen.queryByText(/Fee rate must be a whole number/i)).not.toBeInTheDocument();

    const saveBtn = await screen.findByRole("button", { name: /Save settings/i });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      const feeRateCall = invokeMock.mock.calls.find(
        (c) =>
          c[0] === "update_setting" && (c[1] as { key?: string })?.key === "fee_rate_doos_per_kvb",
      );
      expect(feeRateCall?.[1]).toEqual({
        key: "fee_rate_doos_per_kvb",
        value: "",
      });
    });
  });
});
