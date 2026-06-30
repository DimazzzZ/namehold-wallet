import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { Asset, DashboardStats, ImportResult } from "../types";

interface ListAssetsParams {
  status?: string;
  is_staked?: boolean;
  search?: string;
  sort_by?: string;
  sort_dir?: "asc" | "desc";
}

export function useAssets(params: ListAssetsParams = {}) {
  return useQuery({
    queryKey: ["assets", params],
    queryFn: () => invoke<Asset[]>("list_assets", params as Record<string, unknown>),
  });
}

export function useAsset(id: number) {
  return useQuery({
    queryKey: ["assets", id],
    queryFn: () => invoke<Asset>("get_asset", { id }),
    enabled: id > 0,
  });
}

export function useUpdateAsset() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      id: number;
      status?: string;
      category?: string;
      tags?: string;
      notes?: string;
      hns_received?: number;
      transfer_tx_hash?: string;
      finalize_tx_hash?: string;
    }) => invoke("update_asset", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["assets"] }),
  });
}

export function useBulkUpdateStatus() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { ids: number[]; status: string }) =>
      invoke("bulk_update_status", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["assets"] }),
  });
}

export function useBulkUpdateTags() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { ids: number[]; tags: string }) =>
      invoke("bulk_update_tags", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["assets"] }),
  });
}

export function useDeleteAsset() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => invoke("delete_asset", { id }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["assets"] }),
  });
}

export function useDashboardStats() {
  return useQuery({
    queryKey: ["dashboard"],
    queryFn: () => invoke<DashboardStats>("list_assets", {
      sort_by: "tld",
    }).then(() => invoke<DashboardStats>("get_settings")),
    staleTime: 30_000,
  });
}

export function useImportCsv() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => invoke<ImportResult>("import_csv", { path }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["assets"] });
      qc.invalidateQueries({ queryKey: ["dashboard"] });
    },
  });
}

export function useExportCsv() {
  return useMutation({
    mutationFn: (args: {
      path: string;
      status?: string;
      is_staked?: boolean;
      search?: string;
    }) => invoke<number>("export_csv", args as Record<string, unknown>),
  });
}
