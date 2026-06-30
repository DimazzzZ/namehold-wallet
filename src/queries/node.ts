import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";

/** Status of the hsd node. `connected` (RPC answers) is the authoritative signal. */
export interface NodeStatus {
  binary: string;
  binary_found: boolean;
  version: string | null;
  data_dir: string;
  network: string;
  /** The child process we spawned is still alive (not proof the RPC is up). */
  process_alive: boolean;
  /** The node's RPC actually answered — the truthful "node is up" signal. */
  connected: boolean;
  /** Chain height from the RPC probe, when connected. */
  height: number | null;
  /** Sync progress 0.0..=1.0, when the node reports it. */
  verification_progress: number | null;
  /** Peers' best header height (the sync target), when reported. */
  headers: number | null;
}

/** Poll the hsd node status (binary, data dir, connected, height). */
export function useNodeStatus() {
  return useQuery<NodeStatus>({
    queryKey: ["node-status"],
    queryFn: () => invoke<NodeStatus>("node_status"),
    refetchInterval: 3000,
    retry: false,
  });
}

// Start/stop affect node connectivity, which also gates sending — invalidate the
// node status AND the wallet queries (writeCapability/signer/balances) so every
// status surface updates together.
function invalidateNode(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: ["node-status"] });
  qc.invalidateQueries({ queryKey: ["wallet"] });
}

/** Start hsd against the configured data directory. */
export function useStartHsd() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke<NodeStatus & { message?: string }>("start_hsd"),
    onSuccess: () => invalidateNode(qc),
  });
}

/** Stop the app-managed hsd node. */
export function useStopHsd() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke("stop_hsd"),
    onSuccess: () => invalidateNode(qc),
  });
}
