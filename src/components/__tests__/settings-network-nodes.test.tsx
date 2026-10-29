/**
 * Settings — the local node of the active wallet's network (spec 2026-10-24
 * R7, R8, R10).
 *
 * Full or SPV and "start automatically" are settings of the network, stored
 * as `node_mode.<network>` / `autostart_hsd.<network>` (`main` for mainnet,
 * as the backend's `Network::as_str` spells it); a network with no key of its
 * own shows the app-wide value. Start and Stop act on that network's node and
 * name it, and a node Namehold did not start cannot be stopped.
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
import { makeProfile } from "../../test/fixtures/wallet";
import { makeNodeStatus } from "../../test/fixtures/nodeStatus";
import type { NodeStatus } from "../../queries/node";

function route(network: "mainnet" | "regtest", status: Partial<NodeStatus> = {}) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "list_wallet_profiles") {
      return Promise.resolve([makeProfile({ id: "p1", network, active: true })]);
    }
    if (cmd === "node_status") {
      return Promise.resolve(
        makeNodeStatus({ network: network === "mainnet" ? "main" : network, ...status }),
      );
    }
    return routeSettingsCommand(cmd);
  });
}

function savedSetting(key: string): unknown {
  const call = invokeMock.mock.calls.find(
    (c) => c[0] === "update_setting" && (c[1] as { key?: string })?.key === key,
  );
  return call?.[1];
}

beforeEach(() => {
  invokeMock.mockReset();
  loadSettings();
});

describe("Settings — the active wallet's network node", () => {
  it("shows and saves mode and autostart as settings of the wallet's network", async () => {
    route("mainnet");
    loadSettings({ node_mode: "full", autostart_hsd: "true", "node_mode.main": "spv" });
    renderSettings();

    const select = (await screen.findByTestId("chain-source-select")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("local_spv"));
    fireEvent.click(screen.getByTestId("settings-section-node"));
    const box = await screen.findByTestId("autostart-hsd-checkbox");
    expect(box).toBeChecked(); // mainnet has no key of its own: the app-wide value
    expect(screen.getByText(/Start the mainnet node automatically/i)).toBeInTheDocument();

    fireEvent.click(box);
    fireEvent.click(await screen.findByTestId("settings-save"));

    await waitFor(() =>
      expect(savedSetting("autostart_hsd.main")).toEqual({
        key: "autostart_hsd.main",
        value: "false",
      }),
    );
    expect(savedSetting("node_mode.main")).toEqual({ key: "node_mode.main", value: "spv" });
    expect(savedSetting("autostart_hsd")).toBeUndefined();
    expect(savedSetting("node_mode")).toBeUndefined();
  });

  it("names the network on Start and Stop", async () => {
    route("regtest");
    renderSettings("node");
    expect(await screen.findByRole("button", { name: "Start regtest node" })).toBeInTheDocument();

    route("mainnet", { connected: true, process_alive: true, ownership: "own" });
    renderSettings("node");
    expect(await screen.findByRole("button", { name: "Stop mainnet node" })).toBeEnabled();
  });

  it("disables Stop for a node Namehold did not start, and says why", async () => {
    route("mainnet", { connected: true, ownership: "outside" });
    renderSettings("node");
    const stop = await screen.findByTestId("node-stop-button");
    expect(stop).toBeDisabled();
    fireEvent.mouseEnter(stop.parentElement as HTMLElement);
    expect(
      await screen.findByText(
        "This node was not started by Namehold, so Namehold does not stop it.",
      ),
    ).toBeInTheDocument();
  });

  it("says the node is starting", async () => {
    route("regtest", { process_alive: true, ownership: "own", starting: true });
    renderSettings("node");
    expect(await screen.findByText("Starting regtest node…")).toBeInTheDocument();
  });
});
