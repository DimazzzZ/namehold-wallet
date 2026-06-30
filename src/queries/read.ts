import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { normalizeTransaction } from "../lib/providerMode";
import { useActiveProfile } from "./wallet";
import type { HsdBalance, HsdName, WalletTransactionRow } from "../types";

/**
 * Read query layer (explorer-backed, node-free). Balance + names come from the
 * HNSFans explorer over the active profile's addresses; transactions from the
 * local cache. Writes are never routed through here.
 */

const STALE_TIME = 15_000;

/**
 * Per-wallet balance. The cache is keyed by the active profile id so wallet B
 * never momentarily shows wallet A's number, and it does NOT auto-refetch —
 * each wallet shows its last-known balance (persisted server-side in the chain
 * cache, so it survives a restart) and only updates when the user hits Refresh
 * (which invalidates the `["read"]` prefix).
 */
export function useReadBalance(): UseQueryResult<HsdBalance | null> {
  const { data: profile } = useActiveProfile();
  const profileId = profile?.id ?? null;
  return useQuery<HsdBalance | null>({
    queryKey: ["read", "balance", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<HsdBalance | null>("read_balance");
      return raw ?? null;
    },
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}

/** Provider-aware list of owned / watched names. */
export function useReadNames(): UseQueryResult<HsdName[]> {
  return useQuery<HsdName[]>({
    queryKey: ["read", "names"],
    queryFn: async () => {
      const raw = await invoke<HsdName[] | null>("read_names");
      return Array.isArray(raw) ? raw : [];
    },
    staleTime: STALE_TIME,
  });
}

/** Provider-aware single-name lookup. */
export function useReadNameInfo(
  name: string | null | undefined,
): UseQueryResult<HsdName | null> {
  return useQuery<HsdName | null>({
    queryKey: ["read", "name", name ?? ""],
    enabled: Boolean(name && name.trim().length > 0),
    queryFn: async () => {
      const raw = await invoke<HsdName | null>("read_name_info", {
        name: name!.trim(),
      });
      return raw ?? null;
    },
    staleTime: STALE_TIME,
  });
}

/** Provider-aware, normalized transaction history. */
export function useReadTransactions(): UseQueryResult<WalletTransactionRow[]> {
  return useQuery<WalletTransactionRow[]>({
    queryKey: ["read", "transactions"],
    queryFn: async () => {
      const raw = await invoke<unknown>("read_transactions");
      const arr = Array.isArray(raw) ? (raw as unknown[]) : [];
      return arr.map((tx, i) =>
        normalizeTransaction(tx as Record<string, unknown>, i),
      );
    },
    staleTime: STALE_TIME,
  });
}
