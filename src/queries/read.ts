import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useSettingsStore } from "../stores/settings";
import {
  normalizeTransaction,
  parseStringArraySetting,
  writeBlockedReason,
} from "../lib/providerMode";
import type {
  HsdBalance,
  HsdName,
  ProviderStatus,
  ReadContext,
  WalletReadModel,
  WalletTransactionRow,
} from "../types";

/**
 * Provider-aware read query layer.
 *
 * These hooks call the backend `get_read_context` / `read_*` commands, which
 * transparently resolve the active provider (local hsd, remote hsd, or an
 * external read-only explorer). Components consume the same normalized shapes
 * regardless of the underlying source, plus a `ReadContext` describing the
 * active provider, its health, fallback state, and write permissions.
 *
 * Writes are never routed through this layer.
 */

const STALE_TIME = 15_000;

/** Resolve the active read context (provider, health, fallback, writes). */
export function useReadContext(): UseQueryResult<ReadContext> {
  return useQuery<ReadContext>({
    queryKey: ["read", "context"],
    queryFn: () => invoke<ReadContext>("get_read_context"),
    staleTime: STALE_TIME,
    refetchInterval: 20_000,
  });
}

/**
 * Convenience hook exposing just the active provider's health/status, derived
 * from the resolved read context. Returns `null` until the context resolves.
 */
export function useProviderHealth(): UseQueryResult<ProviderStatus | null> {
  return useQuery({
    queryKey: ["read", "context"],
    queryFn: () => invoke<ReadContext>("get_read_context"),
    staleTime: STALE_TIME,
    refetchInterval: 20_000,
    select: (ctx: ReadContext): ProviderStatus | null =>
      ctx?.activeReadProvider ?? null,
  });
}

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

/**
 * Aggregate read hook combining context + balance + names + transactions into a
 * single `WalletReadModel`. Useful for the Overview and Wallet views which need
 * a holistic snapshot plus the active provider context.
 */
export function useWalletReadModel(): UseQueryResult<WalletReadModel> {
  const settings = useSettingsStore((s) => s.settings);
  const watchAddresses = parseStringArraySetting(
    settings?.external_read_watch_addresses,
  );

  return useQuery<WalletReadModel>({
    queryKey: ["read", "model"],
    staleTime: STALE_TIME,
    refetchInterval: 20_000,
    queryFn: async () => {
      const context = await invoke<ReadContext>("get_read_context");

      const [balanceRaw, namesRaw, txsRaw] = await Promise.all([
        invoke<HsdBalance | null>("read_balance").catch(() => null),
        invoke<HsdName[] | null>("read_names").catch(() => null),
        invoke<unknown>("read_transactions").catch(() => null),
      ]);

      const transactions: WalletTransactionRow[] = Array.isArray(txsRaw)
        ? (txsRaw as unknown[]).map((tx, i) =>
            normalizeTransaction(tx as Record<string, unknown>, i),
          )
        : [];

      return {
        context,
        address: null,
        watchAddresses,
        balance: balanceRaw ?? null,
        names: Array.isArray(namesRaw) ? namesRaw : [],
        transactions,
        lastUpdatedAt: new Date().toISOString(),
        readOnlyReason: writeBlockedReason(context),
      };
    },
  });
}
