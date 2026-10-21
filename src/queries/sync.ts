import { useEffect, useRef } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useActiveProfile } from "./wallet";

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

/**
 * Which profile the sync status describes. `SyncStatus` carries no profile
 * id, so `useSyncStatus` records the profile active when a run is first seen
 * running (the backend takes its profile at start; a switch mid-run does not
 * change it) and keeps that at the run's end. A run that ended before this
 * session read the status has no start to see: the first status read is
 * attributed to the profile active at first load. That is inherent, since
 * `SyncStatus` has no profile id.
 */
const SYNC_RAN_FOR_KEY = ["sync", "ranFor"] as const;

/** The profile active when the run (told apart by `startedAt`) was first seen running. */
const SYNC_STARTED_FOR_KEY = ["sync", "startedFor"] as const;
interface StartedFor {
  startedAt: string | null;
  profileId: string | null;
}

/**
 * `nodeCoinsNotSyncedReason` of the last sync, only while the profile that
 * sync ran for is still the active one: after a switch the reason belongs to
 * the previous profile, and reads `null` until the next sync ends.
 */
export function useNodeCoinsNotSyncedReason(status: SyncStatus | undefined): string | null {
  const activeId = useActiveProfile().data?.id ?? null;
  const { data: ranFor } = useQuery<string | null>({
    queryKey: SYNC_RAN_FOR_KEY,
    queryFn: () => null,
    enabled: false,
    staleTime: Infinity,
    gcTime: Infinity,
  });
  return activeId != null && ranFor === activeId ? nodeCoinsNotSyncedReason(status) : null;
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
 * lists refresh automatically without a manual reload. The lock-search
 * status is refreshed at the same edge by `useLockDiscovery`, which knows
 * whether its own search is about to do it.
 */
export function useSyncStatus() {
  const qc = useQueryClient();
  const wasRunning = useRef(false);
  const activeId = useActiveProfile().data?.id ?? null;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
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
    if (running) {
      // Several mounts see the same run: the first one to write wins.
      const startedAt = query.data?.startedAt ?? null;
      const seen = qc.getQueryData<StartedFor>(SYNC_STARTED_FOR_KEY);
      if (seen === undefined || seen.startedAt !== startedAt) {
        qc.setQueryData<StartedFor>(SYNC_STARTED_FOR_KEY, {
          startedAt,
          profileId: activeIdRef.current,
        });
      }
    }
    if (wasRunning.current && !running) {
      qc.invalidateQueries({ queryKey: ["read"] });
      qc.invalidateQueries({ queryKey: ["wallet"] });
      const started = qc.getQueryData<StartedFor>(SYNC_STARTED_FOR_KEY);
      qc.setQueryData(SYNC_RAN_FOR_KEY, started?.profileId ?? activeIdRef.current);
    }
    wasRunning.current = running;
  }, [query.data?.running, query.data?.startedAt, qc]);

  // A run that ended before this session read the status has no start to
  // record: the first status read belongs to the profile active then.
  const loaded = query.data != null;
  useEffect(() => {
    if (loaded && activeId != null && qc.getQueryData(SYNC_RAN_FOR_KEY) === undefined) {
      qc.setQueryData(SYNC_RAN_FOR_KEY, activeId);
    }
  }, [loaded, activeId, qc]);

  return query;
}
