/**
 * Settings — Chain source selector.
 *
 * Offers the same four choices as onboarding (local full / SPV / remote /
 * explorer) and writes chain_source + node_mode together, so SPV is no longer
 * a separate "Node mode" dropdown that could disagree with chain_source.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { screen, fireEvent, waitFor } from "@testing-library/react";

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
});

describe("Settings — Chain source (connection mode)", () => {
  it("shows SPV as the selected mode when node_mode is spv", async () => {
    loadSettings({ chain_source: "local_node", node_mode: "spv" });
    renderSettings();
    const select = (await screen.findByTestId("chain-source-select")) as HTMLSelectElement;
    expect(select.value).toBe("local_spv");
    // The old standalone Node-mode dropdown is gone.
    expect(screen.queryByTestId("node-mode-select")).toBeNull();
  });

  it("selecting Remote node saves chain_source=remote_node AND node_mode=full", async () => {
    loadSettings({ chain_source: "local_node", node_mode: "spv" });
    renderSettings();
    fireEvent.change(await screen.findByTestId("chain-source-select"), {
      target: { value: "remote_node" },
    });
    // The remote-only opt-in appears.
    expect(screen.getByTestId("allow-remote-broadcast-checkbox")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^save/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_setting", {
        key: "chain_source",
        value: "remote_node",
      }),
    );
    expect(invokeMock).toHaveBeenCalledWith("update_setting", { key: "node_mode", value: "full" });
  });

  it("selecting SPV saves chain_source=local_node AND node_mode=spv, and hides the remote opt-in", async () => {
    loadSettings({
      chain_source: "remote_node",
      node_mode: "full",
      allow_remote_broadcast: "true",
    });
    renderSettings();
    fireEvent.change(await screen.findByTestId("chain-source-select"), {
      target: { value: "local_spv" },
    });
    expect(screen.queryByTestId("allow-remote-broadcast-checkbox")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /^save/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_setting", { key: "node_mode", value: "spv" }),
    );
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "chain_source",
      value: "local_node",
    });
  });
});
