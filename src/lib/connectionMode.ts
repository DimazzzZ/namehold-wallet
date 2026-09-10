import type { ChainSource, NodeMode, Settings } from "../types";

/**
 * What the user picks in the UI — "How do you want to connect?" in onboarding
 * and "Chain source" in Settings. The backend derives `ChainSource::SpvNode`
 * from `chain_source` + `node_mode`, so SPV is not a `chain_source` value;
 * this module is the one place on the frontend that knows the mapping.
 */
export type ConnectionMode = "local_full" | "local_spv" | "remote_node" | "explorer";

export const CONNECTION_MODE_LABELS: Record<ConnectionMode, string> = {
  local_full: "Local full node (this device runs hsd)",
  local_spv: "SPV — lightweight, read-only (headers only, explorer for data)",
  remote_node: "Remote node (point at someone else's hsd)",
  explorer: "Read-only (never send)",
};

export function toConnectionMode(chainSource: ChainSource, nodeMode: NodeMode): ConnectionMode {
  if (chainSource === "explorer") return "explorer";
  if (chainSource === "remote_node") return "remote_node";
  return nodeMode === "spv" ? "local_spv" : "local_full";
}

/**
 * Settings to persist for a mode. Remote and explorer always write
 * `node_mode: "full"` — a leftover `"spv"` would make the backend treat a
 * remote node as read-only (`("remote_node", "spv") → SpvNode`).
 */
export function fromConnectionMode(
  mode: ConnectionMode,
): Pick<Settings, "chain_source" | "node_mode"> {
  switch (mode) {
    case "local_full":
      return { chain_source: "local_node", node_mode: "full" };
    case "local_spv":
      return { chain_source: "local_node", node_mode: "spv" };
    case "remote_node":
      return { chain_source: "remote_node", node_mode: "full" };
    case "explorer":
      return { chain_source: "explorer", node_mode: "full" };
  }
}

/**
 * Settings booleans are persisted as the strings `"true"`/`"false"` (the DB
 * stores every setting as text). These two helpers are the single place that
 * converts between that on-the-wire form and a real boolean, so UI code isn't
 * littered with `=== "true"` / `? "true" : "false"` rewrapping.
 */
export const settingIsTrue = (value: string | undefined): boolean => value === "true";

export const boolToSetting = (value: boolean): "true" | "false" => (value ? "true" : "false");
