import { useEffect, useRef } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { LOCK_SEARCH_STATUS_KEY } from "./shakedex";

export interface SyncStatus {
  running: boolean;
  step: string;
  progressLabel: string;
  repaired: number;
  /** Total repair backlog for this run (counted once up front), not a single
   * window's size — pair with `repairRemaining` for honest progress. */
  repairCandidates: number;
  /** Repair candidates still to check this run; converges to 0. */
  repairRemaining: number;
  discovered: number;
  namesSynced: number;
  errors: string[];
  startedAt: string | null;
  finishedAt: string | null;
  discoverAddressesTotal: number;
  discoverAddressesDone: number;
  discoverTxsScanned: number;
  discoverCandidates: number;
  discoverCurrentName: string;
  waiting: boolean;
  /** Set once `cancel_full_sync` has been called for the in-flight run. */
  cancelRequested: boolean;
}

/**
 * The phrase every reason carries that the backend's sync records when the
 * node step was not authoritative for the wallet's coins (no address index,
 * an hsd older than 7.0, or a node status it could not read): see
 * `NODE_COINS_NOT_SYNCED` in `src-tauri/src/commands/sync.rs`.
 */
export const NODE_COINS_NOT_SYNCED = "the node's coins were not synced";

/**
 * Why the last sync could not read the wallet's coins from the node, or
 * `null` when it could (or no run has recorded anything). A profile never
 * synced through an indexed node then reads 0 from its cache, and this is
 * the reason.
 */
export function nodeCoinsNotSyncedReason(status: SyncStatus | undefined): string | null {
  return status?.errors?.find((e) => e.includes(NODE_COINS_NOT_SYNCED)) ?? null;
}

/** Start a full sync in a background thread. */
export function useStartFullSync() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke("start_full_sync"),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["sync", "status"] });
    },
  });
}

/** Request that an in-flight background sync stop as soon as possible. */
export function useCancelFullSync() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke("cancel_full_sync"),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["sync", "status"] });
    },
  });
}

/**
 * Poll the current sync status (persistent across navigation). When a
 * background sync run finishes (`running` transitions true -> false), the
 * read/wallet caches are invalidated so the Owned Names / Active Auctions
 * lists refresh automatically without a manual reload.
 */
export function useSyncStatus() {
  const qc = useQueryClient();
  const wasRunning = useRef(false);
  const query = useQuery<SyncStatus>({
    queryKey: ["sync", "status"],
    queryFn: () => invoke<SyncStatus>("get_sync_status"),
    refetchInterval: (q) => {
      const d = q.state.data as SyncStatus | undefined;
      return d?.running ? 1500 : false;
    },
    staleTime: 0,
    retry: false,
  });

  useEffect(() => {
    const running = query.data?.running ?? false;
    if (wasRunning.current && !running) {
      qc.invalidateQueries({ queryKey: ["read"] });
      qc.invalidateQueries({ queryKey: ["wallet"] });
      // The sync may have grown the address window or found new history.
      qc.invalidateQueries({ queryKey: LOCK_SEARCH_STATUS_KEY });
    }
    wasRunning.current = running;
  }, [query.data?.running, qc]);

  return query;
}
