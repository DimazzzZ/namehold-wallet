import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { SyncResult, SyncReport } from "../types";

export function useSyncReport() {
  return useQuery({
    queryKey: ["sync", "report"],
    queryFn: () => invoke<SyncReport>("get_sync_report"),
    enabled: false,
    retry: false,
  });
}

export function useSyncNames() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke<SyncResult>("sync_names"),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["assets"] });
      qc.invalidateQueries({ queryKey: ["sync"] });
      qc.invalidateQueries({ queryKey: ["wallet"] });
    },
  });
}
