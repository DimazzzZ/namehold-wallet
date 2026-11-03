/**
 * The real settings store's save path: protected settings
 * (lib/protectedSettings.ts) go to `update_protected_settings` in one call,
 * confirmed in the secure window, before any plain key is written with
 * `update_setting`; a declined confirmation saves nothing.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useSettingsStore } from "./settings";
import { loadSettings } from "../test/fixtures/settings";

const commands = () => invokeMock.mock.calls.map((c) => c[0] as string);

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  loadSettings({ node_rpc_url: "http://127.0.0.1:12037", hsd_path: "" });
});

describe("useSettingsStore.saveAll", () => {
  it("sends the changed protected settings in one call before the plain ones", async () => {
    const saved = await useSettingsStore.getState().saveAll({
      node_rpc_url: "https://node.example.com",
      hsd_path: "",
      close_to_tray: "0",
    });

    expect(saved).toBe(true);
    expect(invokeMock.mock.calls).toEqual([
      [
        "update_protected_settings",
        { changes: [{ key: "node_rpc_url", value: "https://node.example.com" }] },
      ],
      ["update_setting", { key: "close_to_tray", value: "0" }],
    ]);
    expect(useSettingsStore.getState().settings?.node_rpc_url).toBe("https://node.example.com");
  });

  it("makes no protected call when no protected setting changed", async () => {
    const saved = await useSettingsStore.getState().saveAll({
      node_rpc_url: "http://127.0.0.1:12037",
      close_to_tray: "0",
    });

    expect(saved).toBe(true);
    expect(commands()).toEqual(["update_setting"]);
  });

  it("a declined confirmation saves nothing and keeps the stored settings", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "update_protected_settings"
        ? Promise.reject("Confirmation declined")
        : Promise.resolve(null),
    );

    const saved = await useSettingsStore.getState().saveAll({
      hsd_path: "/tmp/hsd",
      close_to_tray: "0",
    });

    expect(saved).toBe(false);
    expect(commands()).toEqual(["update_protected_settings"]);
    expect(useSettingsStore.getState().settings?.hsd_path).toBe("");
    expect(useSettingsStore.getState().settings?.close_to_tray).toBe("1");
  });

  it("any other refusal rejects and saves nothing else", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "update_protected_settings"
        ? Promise.reject("Invalid input: the hsd program must be given by its full path")
        : Promise.resolve(null),
    );

    await expect(
      useSettingsStore.getState().saveAll({ hsd_path: "hsd", close_to_tray: "0" }),
    ).rejects.toMatch(/full path/);
    expect(commands()).toEqual(["update_protected_settings"]);
    expect(useSettingsStore.getState().settings?.hsd_path).toBe("");
  });

  it("saves nothing before the settings are loaded", async () => {
    useSettingsStore.setState({ settings: null, loaded: false });

    expect(await useSettingsStore.getState().saveAll({ hsd_path: "/x" })).toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
