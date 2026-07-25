import { useEffect, useRef } from "react";
import { useNodeLive } from "./node";
import { useStartFullSync, useSyncStatus } from "./sync";

/**
 * How often to auto-kick a background sync while the node is live and idle.
 * A sync refreshes the local chain cache (balances, owned names, transactions);
 * the completion edge (`useSyncStatus`) then invalidates the `["read"]` /
 * `["wallet"]` caches so every view reflects the freshest node state without a
 * manual Refresh.
 */
export const AUTO_SYNC_INTERVAL_MS = 60_000;

/**
 * Keep the app's cached data fresh from the local node with no manual Refresh.
 *
 * Mount ONCE at the app root. While the node is the authoritative read source
 * (`useNodeLive()` — connected AND fully synced), this:
 *   - kicks one background sync on the node-becoming-live edge (and on mount if
 *     the node is already live), and
 *   - re-kicks a sync every `AUTO_SYNC_INTERVAL_MS` while it stays live.
 *
 * When the node is NOT live (down / still syncing) it does nothing — there's no
 * fresher source than the cache/explorer, so we avoid pointless churn; the
 * manual Refresh button still covers that case.
 *
 * `start_full_sync` is idempotent on the backend (a call while a run is in
 * flight is a clean no-op), and we additionally skip firing while a sync is
 * already `running`, so the interval can never stampede.
 */
export function useAutoSync(): void {
  const nodeLive = useNodeLive();
  const startSync = useStartFullSync();
  const running = useSyncStatus().data?.running ?? false;

  // Keep the latest `running` flag readable from the interval callback without
  // re-arming the timer every time it flips.
  const runningRef = useRef(running);
  runningRef.current = running;
  // Fire-and-forget kick that never throws (a rejected mutation is harmless —
  // the next tick/edge retries).
  const kick = () => {
    if (runningRef.current) return;
    startSync.mutate();
  };
  const kickRef = useRef(kick);
  kickRef.current = kick;

  // Edge + mount: sync once whenever the node transitions into (or starts in)
  // the live state.
  const wasLive = useRef(false);
  useEffect(() => {
    if (nodeLive && !wasLive.current) {
      kickRef.current();
    }
    wasLive.current = nodeLive;
  }, [nodeLive]);

  // Interval: while live, re-kick on a timer. Re-armed only when liveness
  // changes (not on every `running` flip), reading `running` via the ref.
  useEffect(() => {
    if (!nodeLive) return;
    const id = setInterval(() => kickRef.current(), AUTO_SYNC_INTERVAL_MS);
    return () => clearInterval(id);
  }, [nodeLive]);
}
