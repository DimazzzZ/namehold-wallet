import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { normalizeTransaction } from "../lib/providerMode";
import type { HsdBalance, HsdName, WalletTransactionRow } from "../types";

/**
 * Read query layer (explorer-backed, node-free). Balance + names come from the
 * HNSFans explorer over the active profile's addresses; transactions from the
 * local cache. Writes are never routed through here.
 */

const STALE_TIME = 15_000;

/** Provider-aware balance (hsd or aggregated external watch addresses). */
export function useReadBalance(): UseQueryResult<HsdBalance | null> {
  return useQuery<HsdBalance | null>({
    queryKey: ["read", "balance"],
    queryFn: async () => {
      const raw = await invoke<HsdBalance | null>("read_balance");
      return raw ?? null;
    },
    staleTime: STALE_TIME,
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
