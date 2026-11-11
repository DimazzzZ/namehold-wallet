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
import { QueryClient } from "@tanstack/react-query";

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
});

/** The changes of the one `update_protected_settings` call, or null before it. */
function protectedChanges(): { key: string; value: string }[] | null {
  const calls = invokeMock.mock.calls.filter((c) => c[0] === "update_protected_settings");
  expect(calls.length).toBeLessThanOrEqual(1);
  return calls[0] ? (calls[0][1] as { changes: { key: string; value: string }[] }).changes : null;
}

/** The keys written through the plain `update_setting`. */
function plainKeys(): string[] {
  return invokeMock.mock.calls
    .filter((c) => c[0] === "update_setting")
    .map((c) => (c[1] as { key: string }).key);
}

describe("Settings — Chain source (connection mode)", () => {
  it("shows SPV as the selected mode when node_mode is spv", async () => {
    loadSettings({ chain_source: "local_node", node_mode: "spv" });
    renderSettings();
    const select = (await screen.findByTestId("chain-source-select")) as HTMLSelectElement;
    expect(select.value).toBe("local_spv");
    // The old standalone Node-mode dropdown is gone.
    expect(screen.queryByTestId("node-mode-select")).toBeNull();
  });

  it("selecting Remote node sends chain_source=remote_node AND node_mode=full to be confirmed", async () => {
    loadSettings({ chain_source: "local_node", node_mode: "spv" });
    renderSettings();
    fireEvent.change(await screen.findByTestId("chain-source-select"), {
      target: { value: "remote_node" },
    });
    // The remote-only opt-in appears.
    expect(screen.getByTestId("allow-remote-broadcast-checkbox")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^save/i }));
    await waitFor(() => expect(protectedChanges()).not.toBeNull());
    expect(protectedChanges()).toHaveLength(2);
    expect(protectedChanges()).toEqual(
      expect.arrayContaining([
        { key: "chain_source", value: "remote_node" },
        { key: "node_mode", value: "full" },
      ]),
    );
    expect(plainKeys()).not.toContain("chain_source");
    expect(plainKeys()).not.toContain("node_mode");
  });

  // QA 2026-11-11, Q10: the Exchange page kept the gate of the old chain
  // source for about half a minute after the save.
  it("a saved chain source makes the Exchange page read its gate again", async () => {
    const invalidate = vi.spyOn(QueryClient.prototype, "invalidateQueries");
    loadSettings({ chain_source: "local_node", node_mode: "full" });
    renderSettings();
    fireEvent.change(await screen.findByTestId("chain-source-select"), {
      target: { value: "remote_node" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^save/i }));
    await waitFor(() => expect(protectedChanges()).not.toBeNull());
    await waitFor(() => expect(invalidate).toHaveBeenCalledWith({ queryKey: ["exchange"] }));
    invalidate.mockRestore();
  });

  it("selecting SPV sends chain_source=local_node AND node_mode=spv to be confirmed, and hides the remote opt-in", async () => {
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
    await waitFor(() => expect(protectedChanges()).not.toBeNull());
    expect(protectedChanges()).toHaveLength(2);
    expect(protectedChanges()).toEqual(
      expect.arrayContaining([
        { key: "chain_source", value: "local_node" },
        { key: "node_mode", value: "spv" },
      ]),
    );
    expect(plainKeys()).not.toContain("chain_source");
    expect(plainKeys()).not.toContain("node_mode");
  });
});
