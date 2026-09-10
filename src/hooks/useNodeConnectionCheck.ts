import { useState } from "react";
import { invoke } from "../lib/invoke";
import type { NodeConnectionCheck } from "../types";

export interface NodeConnectionCheckState {
  /** A probe is in flight. */
  testing: boolean;
  /** Last probe result, or null before the first probe / after reset(). */
  result: NodeConnectionCheck | null;
  /** Human-readable failure: empty URL, unreachable node, or a thrown guard. */
  error: string | null;
  /** True only after a probe that actually reached the node. */
  ok: boolean;
  /** Probe `url` with an optional API key. An empty URL sets `error` without a backend call. */
  run: (url: string, apiKey?: string) => Promise<void>;
  /** Forget the last outcome — call whenever the URL or key being probed changes. */
  reset: () => void;
}

/**
 * State + action behind every "Test connection" button. Backed by the
 * `check_node_connection` Tauri command, which probes a candidate node RPC
 * without persisting anything. Shared by onboarding and Settings so the two
 * screens cannot drift in what "connected" means.
 */
export function useNodeConnectionCheck(): NodeConnectionCheckState {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<NodeConnectionCheck | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setResult(null);
    setError(null);
  };

  const run = async (url: string, apiKey?: string) => {
    const trimmed = url.trim();
    if (!trimmed) {
      setResult(null);
      setError("Enter a node RPC URL first");
      return;
    }
    setTesting(true);
    reset();
    try {
      const r = await invoke<NodeConnectionCheck>("check_node_connection", {
        url: trimmed,
        api_key: apiKey || undefined,
      });
      setResult(r);
      if (!r.reachable) setError(r.error || "Node unreachable");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTesting(false);
    }
  };

  return { testing, result, error, ok: result?.reachable === true, run, reset };
}
