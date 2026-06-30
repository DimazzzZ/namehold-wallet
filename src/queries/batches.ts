import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { Batch, BatchWithAssets } from "../types";

export function useBatches() {
  return useQuery({
    queryKey: ["batches"],
    queryFn: () => invoke<Batch[]>("list_batches"),
  });
}

export function useBatchWithAssets(id: number) {
  return useQuery({
    queryKey: ["batches", id],
    queryFn: () => invoke<BatchWithAssets>("get_batch_with_assets", { id }),
    enabled: id > 0,
  });
}

export function useCreateBatch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      name: string;
      description?: string;
      asset_ids: number[];
    }) => invoke<number>("create_batch", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["batches"] }),
  });
}

export function useUpdateBatch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      id: number;
      name?: string;
      description?: string;
      status?: string;
    }) => invoke("update_batch", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["batches"] }),
  });
}

export function useDeleteBatch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => invoke("delete_batch", { id }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["batches"] }),
  });
}

export function useAddToBatch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { batch_id: number; asset_ids: number[] }) =>
      invoke("add_to_batch", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["batches"] }),
  });
}

export function useRemoveFromBatch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { batch_id: number; asset_ids: number[] }) =>
      invoke("remove_from_batch", args as Record<string, unknown>),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["batches"] }),
  });
}
