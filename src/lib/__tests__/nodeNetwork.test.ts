import { describe, it, expect } from "vitest";
import localNodeSource from "../../../src-tauri/src/commands/local_node.rs?raw";
import {
  networkDisplayName,
  networkNodeSettingKey,
  nodeActionLabel,
  OUTSIDE_NODE_NOT_STOPPED,
} from "../nodeNetwork";

describe("nodeNetwork (spec 2026-10-24)", () => {
  it("disables Stop with the very sentence the backend refuses it with (R7)", () => {
    expect(localNodeSource).toContain(`"${OUTSIDE_NODE_NOT_STOPPED}"`);
  });

  it("spells networks for people and setting keys as the backend does", () => {
    expect(networkDisplayName("main")).toBe("mainnet");
    expect(networkDisplayName("regtest")).toBe("regtest");
    // `Network::as_str`: mainnet is `main` in a key, the others as written.
    expect(networkNodeSettingKey("node_mode", "mainnet")).toBe("node_mode.main");
    expect(networkNodeSettingKey("autostart_hsd", "testnet")).toBe("autostart_hsd.testnet");
  });

  it("names the network on a node control, or none without a wallet (R8)", () => {
    expect(nodeActionLabel("Start", "main")).toBe("Start mainnet node");
    expect(nodeActionLabel("Stop", "regtest")).toBe("Stop regtest node");
    expect(nodeActionLabel("Starting", "testnet")).toBe("Starting testnet node…");
    expect(nodeActionLabel("Start", null)).toBe("Start node");
  });
});
