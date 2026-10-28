import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useUiStore } from "../stores/ui";
import type { HsdName, WatchedName, WatchlistStatus } from "../types";

/** The watchlist (`list_watchlist`). */
export function useWatchlist() {
  return useQuery<WatchedName[]>({
    queryKey: ["watchlist"],
    queryFn: async () => (await invoke<WatchedName[] | null>("list_watchlist")) ?? [],
  });
}

/** Membership + cached state for every watched name. */
export function useWatchlistStatuses(names: string[]) {
  return useQuery<WatchlistStatus[]>({
    queryKey: ["watchlist", "status", names.join(",")],
    queryFn: () => invoke<WatchlistStatus[]>("get_watchlist_status", { names }),
    enabled: names.length > 0,
    staleTime: 30_000,
  });
}

/**
 * Full HsdName info for every watched name: `state` for the phase, `stats` for
 * countdown / expiry, `highest` for the highest bid. Per-name failures are
 * left out rather than failing the whole list.
 */
export function useWatchlistLiveInfo(names: string[]) {
  return useQuery<Record<string, HsdName>>({
    queryKey: ["watchlist", "liveInfo", names.join(",")],
    queryFn: async () => {
      const results = await Promise.all(
        names.map(async (name) => {
          try {
            const raw = await invoke<HsdName | null>("read_name_info", { name });
            return { name, info: raw ?? null };
          } catch {
            return { name, info: null };
          }
        }),
      );
      const out: Record<string, HsdName> = {};
      for (const r of results) {
        if (r.info) out[r.name] = r.info;
      }
      return out;
    },
    enabled: names.length > 0,
    staleTime: 30_000,
  });
}

/**
 * One name's membership. Distinct from the bulk `["watchlist","status",
 * <joined names>]` key — same shape but this returns a single row
 * (`rows[0]`), so sharing the key with the bulk query would corrupt the cached
 * shape when a user watches exactly one name and opens its info modal.
 */
export function useWatchlistMembership(name: string) {
  return useQuery<WatchlistStatus | undefined>({
    queryKey: ["watchlist", "toggle", name],
    queryFn: async () => {
      const rows = await invoke<WatchlistStatus[]>("get_watchlist_status", { names: [name] });
      return rows[0];
    },
  });
}

export function useAddToWatchlist() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  return useMutation({
    mutationFn: (name: string) => invoke("add_to_watchlist", { name }),
    onSuccess: (_, name) => {
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      showToast(`Added ${name} to watchlist`, "success");
    },
    onError: (err, name) => showToast(`Failed to add ${name}: ${err}`, "error"),
  });
}

export function useRemoveFromWatchlist() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  return useMutation({
    mutationFn: (name: string) => invoke("remove_from_watchlist", { name }),
    onSuccess: (_, name) => {
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      showToast(`Removed ${name} from watchlist`, "info");
    },
    onError: (err, name) => showToast(`Failed to remove ${name}: ${err}`, "error"),
  });
}

export function useUpdateWatchlistTags() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  return useMutation({
    mutationFn: ({ name, tags }: { name: string; tags: string }) =>
      invoke("update_watchlist_tags", { name, tags }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["watchlist"] }),
    onError: (err) => showToast(`Failed to update tags: ${err}`, "error"),
  });
}

/** Export the watchlist to a CSV the user picks. */
export async function exportWatchlistCsv(showToast: (m: string, t?: "success" | "error") => void) {
  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      defaultPath: "watchlist-export.csv",
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!path) return;
    const count = await invoke<number>("export_watchlist_csv", { path });
    showToast(`Exported ${count} name(s) to CSV`, "success");
  } catch (e) {
    showToast(`Export failed: ${e}`, "error");
  }
}

/** Import names from a CSV the user picks, then refresh the watchlist. */
export async function importWatchlistCsv(
  showToast: (m: string, t?: "success" | "error") => void,
  invalidate: () => void,
) {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      filters: [{ name: "CSV", extensions: ["csv"] }],
      multiple: false,
    });
    if (!selected) return;
    const result = await invoke<{ imported: number; skipped: number; errors: string[] }>(
      "import_watchlist_csv",
      { path: selected },
    );
    invalidate();
    showToast(
      `Imported ${result.imported}, skipped ${result.skipped}${
        result.errors.length > 0 ? `, ${result.errors.length} error(s)` : ""
      }`,
      "success",
    );
  } catch (e) {
    showToast(`Import failed: ${e}`, "error");
  }
}
