import { useRef, useState } from "react";
import { invoke } from "../lib/invoke";
import type { NodeConnectionCheck } from "../types";

export interface NodeConnectionCheckState {
  /** A probe is in flight. */
  testing: boolean;
  /** Last probe result, or null before the first probe / after reset(). */
  result: NodeConnectionCheck | null;
  /** Human-readable failure: empty URL, unreachable node, or a thrown guard. */
  error: string | null;
  /** True only after a probe that reached the node on the wallet's network. */
  ok: boolean;
  /**
   * Probe `url` with an optional API key. An empty URL sets `error` without a
   * backend call. G1: `expectedNetwork` is the network the user picked in the
   * onboarding UI (before any wallet profile exists); when passed, the backend
   * compares against it instead of falling back to the active profile's
   * network — which during onboarding is `None`, making every probe pass
   * regardless of the node's actual chain.
   */
  run: (url: string, apiKey?: string, expectedNetwork?: string) => Promise<void>;
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
  // Bumped by every run() and every reset() so a response can tell whether it
  // still belongs to the newest request. Without this, editing the URL/key
  // mid-probe (which calls reset()) doesn't stop an earlier response from
  // landing later and repopulating `result` for a URL that was never
  // re-probed — see the "editing mid-flight" regression test.
  const requestId = useRef(0);

  const reset = () => {
    // Invalidate any in-flight probe: its eventual response must be a no-op.
    requestId.current += 1;
    setResult(null);
    setError(null);
    // Also clear `testing` so an edit mid-probe doesn't leave the "Test
    // connection" button permanently disabled — the abandoned request's own
    // `finally` is guarded below and will no longer touch `testing` once its
    // id is stale.
    setTesting(false);
  };

  const run = async (url: string, apiKey?: string, expectedNetwork?: string) => {
    requestId.current += 1;
    const myRequestId = requestId.current;
    const trimmed = url.trim();
    if (!trimmed) {
      setResult(null);
      setError("Enter a node RPC URL first");
      return;
    }
    setTesting(true);
    setResult(null);
    setError(null);
    try {
      const r = await invoke<NodeConnectionCheck>("check_node_connection", {
        url: trimmed,
        api_key: apiKey || undefined,
        expected_network: expectedNetwork || undefined,
      });
      if (requestId.current !== myRequestId) return; // superseded by reset()/run()
      setResult(r);
      if (!r.reachable) setError(r.error || "Node unreachable");
    } catch (e) {
      if (requestId.current !== myRequestId) return; // superseded by reset()/run()
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestId.current === myRequestId) setTesting(false);
    }
  };

  // A reachable node on the WRONG network is not a usable node, so `ok` gates
  // on the mismatch flag too. `networkMatches` is null when there is nothing
  // to compare — no wallet profile yet (onboarding), or a node that didn't
  // report its chain — and null must not block.
  const ok = result?.reachable === true && result.networkMatches !== false;

  return { testing, result, error, ok, run, reset };
}
