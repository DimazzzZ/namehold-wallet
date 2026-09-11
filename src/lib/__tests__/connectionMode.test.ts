import { describe, it, expect } from "vitest";
import {
  CONNECTION_MODE_LABELS,
  fromConnectionMode,
  toConnectionMode,
  type ConnectionMode,
} from "../connectionMode";

const ALL: ConnectionMode[] = ["local_full", "local_spv", "remote_node", "explorer"];

describe("connectionMode", () => {
  it("round-trips every mode through settings and back", () => {
    for (const mode of ALL) {
      const { chain_source, node_mode } = fromConnectionMode(mode);
      expect(toConnectionMode(chain_source, node_mode)).toBe(mode);
    }
  });

  it("maps SPV to a local chain_source (the backend derives SpvNode from node_mode)", () => {
    expect(fromConnectionMode("local_spv")).toEqual({
      chain_source: "local_node",
      node_mode: "spv",
    });
  });

  it("forces node_mode=full for remote and explorer so a stale spv cannot make them read-only", () => {
    expect(fromConnectionMode("remote_node")).toEqual({
      chain_source: "remote_node",
      node_mode: "full",
    });
    expect(fromConnectionMode("explorer")).toEqual({ chain_source: "explorer", node_mode: "full" });
    // Legacy DB state: remote + spv still displays as Remote node.
    expect(toConnectionMode("remote_node", "spv")).toBe("remote_node");
  });

  it("has a label for every mode", () => {
    for (const mode of ALL) expect(CONNECTION_MODE_LABELS[mode]).toBeTruthy();
  });
});
