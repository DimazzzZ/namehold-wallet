import { useQuery, useMutation } from "@tanstack/react-query";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { invoke } from "../lib/invoke";
import { isTauri } from "../lib/runtime";

/** Metadata for an available update, returned by `check_for_update`. */
export interface UpdateMetadata {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
}

/** Progress events streamed from the backend during download+install. */
export type DownloadEvent =
  | { event: "Started"; data: { contentLength: number | null } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

/** The running app's own version, for display. Cached — it never changes. */
export function useCurrentVersion() {
  return useQuery<string>({
    queryKey: ["app-version"],
    queryFn: () => invoke<string>("current_version"),
    staleTime: Infinity,
    retry: false,
  });
}

/**
 * Check the release endpoint for a newer version. Resolves to the update
 * metadata, or `null` when already up to date. The backend stashes the pending
 * update so a subsequent `installUpdate()` can install it without re-checking.
 */
export function useCheckForUpdate() {
  return useMutation<UpdateMetadata | null>({
    mutationFn: () => invoke<UpdateMetadata | null>("check_for_update"),
  });
}

/**
 * Download + install the update found by the last `check_for_update`, reporting
 * progress via `onProgress`. Install only runs inside Tauri (the updater plugin
 * is desktop-only), so this bypasses the web-QA mock and talks to the real
 * backend directly, passing an IPC `Channel` the mock can't model.
 */
export function useInstallUpdate(opts?: {
  onProgress?: (event: DownloadEvent) => void;
}) {
  return useMutation<void>({
    mutationFn: async () => {
      if (!isTauri()) {
        throw new Error("Updates are only available in the desktop app.");
      }
      const channel = new Channel<DownloadEvent>();
      if (opts?.onProgress) {
        channel.onmessage = opts.onProgress;
      }
      await tauriInvoke("install_update", { onEvent: channel });
    },
  });
}

/** Restart the app to run the freshly-installed update (desktop only). */
export async function relaunchApp(): Promise<void> {
  if (!isTauri()) return;
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
