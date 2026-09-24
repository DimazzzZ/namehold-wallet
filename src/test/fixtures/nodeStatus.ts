import type { NodeStatus } from "../../queries/node";

/**
 * A complete `NodeStatus`: no node running, reads from the explorer. Tests
 * override the verdict they are about (`synced`, `read_source`) rather than
 * re-deriving one — the rule itself is pinned in Rust (`chain_synced`).
 */
export function makeNodeStatus(over: Partial<NodeStatus> = {}): NodeStatus {
  return {
    binary: "/usr/local/bin/hsd",
    binary_found: true,
    version: "hsd 8.0.0",
    data_dir: "/Volumes/WD/hsd-data",
    network: "main",
    process_alive: false,
    connected: false,
    height: null,
    verification_progress: null,
    headers: null,
    synced: false,
    last_error: null,
    index_mismatch: false,
    read_source: "explorer",
    node_mode: "full",
    ...over,
  };
}
