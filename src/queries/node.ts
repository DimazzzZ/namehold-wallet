import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";

export interface NodeStatus {
  running: boolean;
  wallet_connected: boolean;
  info?: unknown;
  error?: string;
  hsd_binary?: string;
  hsd_binary_found?: boolean;
  hsd_version?: string;
  chain?: Record<string, unknown>;
}

export function useNodeStatus() {
  return useQuery({
    queryKey: ["node", "status"],
    queryFn: () => invoke<NodeStatus>("get_node_status"),
    refetchInterval: 10_000,
    retry: false,
  });
}

export function useStopHsd() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke<string>("stop_hsd"),
    onSuccess: () => {
      setTimeout(() => qc.invalidateQueries({ queryKey: ["node"] }), 2000);
    },
  });
}

export function useStartHsd() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { prefix?: string; api_key?: string; network?: string }) =>
      invoke<string>("start_hsd", args as Record<string, unknown>),
    onSuccess: () => {
      setTimeout(() => qc.invalidateQueries({ queryKey: ["node"] }), 3000);
    },
  });
}
