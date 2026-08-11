import { create } from "zustand";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { invoke } from "../lib/invoke";
import { isTauri } from "../lib/runtime";
import type { UpdateMetadata, DownloadEvent } from "../queries/updates";
import { relaunchApp } from "../queries/updates";

/**
 * Update lifecycle phase. One shared machine drives BOTH the Settings card and
 * the global banner, so a check or install started in one surface is reflected
 * in the other — mirroring the single pending-update slot on the Rust side.
 */
export type UpdatePhase =
  | "idle"
  | "checking"
  | "upToDate"
  | "available"
  | "installing"
  | "installed"
  | "error";

interface AppUpdateState {
  phase: UpdatePhase;
  available: UpdateMetadata | null;
  /** Download progress 0..1 while installing, else null. */
  progress: number | null;
  error: string | null;
  /** Version the user dismissed from the banner (persisted), to avoid nagging. */
  dismissedVersion: string | null;

  check: (opts?: { silent?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
  reset: () => void;
}

const DISMISS_KEY = "namehold.update.dismissedVersion";

function loadDismissed(): string | null {
  try {
    return localStorage.getItem(DISMISS_KEY);
  } catch {
    return null;
  }
}

export const useAppUpdate = create<AppUpdateState>((set, get) => ({
  phase: "idle",
  available: null,
  progress: null,
  error: null,
  dismissedVersion: loadDismissed(),

  check: async (opts) => {
    // Never run two checks at once, and don't restart a check mid-install.
    const { phase } = get();
    if (phase === "checking" || phase === "installing") return;

    set({ phase: "checking", error: null });
    try {
      const update = await invoke<UpdateMetadata | null>("check_for_update");
      if (update) {
        set({ phase: "available", available: update });
      } else {
        // A silent auto-check that finds nothing returns to idle quietly; an
        // explicit user check shows the reassuring "up to date" state.
        set({ phase: opts?.silent ? "idle" : "upToDate", available: null });
      }
    } catch (e) {
      set({ phase: "error", error: String(e) });
    }
  },

  install: async () => {
    if (!isTauri()) {
      set({ phase: "error", error: "Updates are only available in the desktop app." });
      return;
    }
    if (get().phase !== "available") return;

    set({ phase: "installing", progress: 0, error: null });
    try {
      let total = 0;
      let downloaded = 0;
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = (msg) => {
        if (msg.event === "Started") {
          total = msg.data.contentLength ?? 0;
        } else if (msg.event === "Progress") {
          downloaded += msg.data.chunkLength;
          set({ progress: total > 0 ? Math.min(downloaded / total, 1) : null });
        } else if (msg.event === "Finished") {
          set({ progress: 1 });
        }
      };
      await tauriInvoke("install_update", { onEvent: channel });
      set({ phase: "installed", progress: 1 });
    } catch (e) {
      set({ phase: "error", error: String(e) });
    }
  },

  dismiss: () => {
    // Once an update is installed on disk, restart is the only sensible next
    // step — and no re-check will resurface it. Never let a dismiss strip the
    // restart affordance from the banner or the Settings card (both read this
    // shared phase), so dismissing is a no-op in the installed phase.
    if (get().phase === "installed") return;
    const v = get().available?.version;
    if (v) {
      try {
        localStorage.setItem(DISMISS_KEY, v);
      } catch {
        // Non-fatal: dismissal just won't persist across restarts.
      }
      set({ dismissedVersion: v });
    }
    // Collapse the banner without discarding the pending update, so the user
    // can still install from Settings.
    set({ phase: "idle" });
  },

  reset: () => set({ phase: "idle", available: null, progress: null, error: null }),
}));

/** Trigger the post-install relaunch. Separate from the store so the store has
 *  no side effects on import and stays easy to test. */
export { relaunchApp };
