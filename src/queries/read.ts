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
      // Pin the read to THIS wallet so a fetch can never return another
      // profile's balance (the active profile may flip mid-switch).
      const raw = await invoke<HsdBalance | null>("read_balance", {
        walletProfileId: profileId,
      });
      return raw ?? null;
    },
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}

/** Provider-aware list of owned / watched names, pinned to the active wallet. */
export function useReadNames(): UseQueryResult<HsdName[]> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<HsdName[]>({
    queryKey: ["read", "names", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<HsdName[] | null>("read_names", {
        walletProfileId: profileId,
      });
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

/** Provider-aware, normalized transaction history, pinned to the active wallet. */
export function useReadTransactions(): UseQueryResult<WalletTransactionRow[]> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<WalletTransactionRow[]>({
    queryKey: ["read", "transactions", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<unknown>("read_transactions", {
        walletProfileId: profileId,
      });
      const arr = Array.isArray(raw) ? (raw as unknown[]) : [];
      return arr.map((tx, i) =>
        normalizeTransaction(tx as Record<string, unknown>, i),
      );
    },
    staleTime: STALE_TIME,
  });
}
