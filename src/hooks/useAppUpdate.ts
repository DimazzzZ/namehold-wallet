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
  /**
   * DEV ONLY. True when the current `available` update was seeded by
   * `simulateUpdateFlow()` rather than a real Rust-side pending update. When
   * set, `install()` runs a fake download loop instead of calling the real
   * `install_update` command (which would fail with `NoPendingUpdate`).
   */
  simulated: boolean;

  check: (opts?: { silent?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
  reset: () => void;
  /**
   * DEV ONLY. Seed the store with the last GitHub release (or a synthetic
   * bumped version when offline) so the banner + Settings card show the
   * "available" notice — WITHOUT auto-installing. Clicking "Install now"
   * then runs a fake download via `install()`. No-op in production. See the
   * dev panel in `Settings.tsx`.
   */
  simulateUpdateFlow: () => Promise<void>;
}

const DISMISS_KEY = "namehold.update.dismissedVersion";

function loadDismissed(): string | null {
  try {
    return localStorage.getItem(DISMISS_KEY);
  } catch {
    return null;
  }
}

/** Bump the patch segment of a semver-ish string so a simulated update always
 *  looks newer than the running version. Falls back to a `-dev.1` suffix when
 *  the version doesn't parse as dotted numbers. */
function bumpPatch(v: string): string {
  const parts = (v ?? "").split(".");
  const last = Number(parts[parts.length - 1]);
  if (parts.length >= 2 && Number.isInteger(last)) {
    parts[parts.length - 1] = String(last + 1);
    return parts.join(".");
  }
  return `${v || "0.0.0"}-dev.1`;
}

/** Metadata returned by the dev-only `fetch_latest_release_meta` command. */
interface DevReleaseMeta {
  version: string;
  notes: string | null;
  date: string | null;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export const useAppUpdate = create<AppUpdateState>((set, get) => ({
  phase: "idle",
  available: null,
  progress: null,
  error: null,
  dismissedVersion: loadDismissed(),
  simulated: false,

  check: async (opts) => {
    // Never run two checks at once, and don't restart a check mid-install.
    const { phase } = get();
    if (phase === "checking" || phase === "installing") return;

    set({ phase: "checking", error: null });
    try {
      const update = await invoke<UpdateMetadata | null>("check_for_update");
      if (update) {
        set({ phase: "available", available: update, simulated: false });
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
    // A dev `simulateUpdateFlow()` seeds `available` with no real pending
    // Rust-side update, so we can't call `install_update` (it would error
    // with `NoPendingUpdate`). When `simulated` is set, run a fake download
    // loop instead — driven by the same "Install now" click as a real update.
    if (get().simulated && import.meta.env.DEV) {
      if (get().phase !== "available") return;
      set({ phase: "installing", progress: 0, error: null });
      for (let i = 1; i <= 10; i++) {
        await sleep(120);
        // Re-check each tick so reset()/dismiss() aborts cleanly.
        if (get().phase !== "installing") return;
        set({ progress: i / 10 });
      }
      set({ phase: "installed", progress: 1 });
      return;
    }

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

  reset: () =>
    set({ phase: "idle", available: null, progress: null, error: null, simulated: false }),

  simulateUpdateFlow: async () => {
    // Hard prod guard so this action is a no-op even if it slips into a
    // release bundle. The dev panel that calls it is already tree-shaken by
    // `import.meta.env.DEV`, but belt-and-braces is cheap here.
    if (!import.meta.env.DEV || !isTauri()) return;

    const { phase } = get();
    if (phase === "checking" || phase === "installing") return;

    // Reuse the same command `useCurrentVersion` wraps, so the "current"
    // string matches what the Settings card shows.
    let currentVersion = "0.0.0";
    try {
      currentVersion = await invoke<string>("current_version");
    } catch {
      // Non-fatal: we'll still simulate against the fallback "0.0.0".
    }

    let meta: DevReleaseMeta;
    try {
      const real = await invoke<DevReleaseMeta>("fetch_latest_release_meta");
      // If GitHub's latest happens to be the version we're already running,
      // bump it so the "available" state is meaningful in the simulation.
      meta =
        real.version === currentVersion
          ? {
              version: bumpPatch(currentVersion),
              notes: real.notes,
              date: real.date ?? new Date().toISOString(),
            }
          : real;
    } catch {
      // Offline / rate-limited / GitHub error: synthesize a plausible bump so
      // the button still exercises the UI.
      meta = {
        version: bumpPatch(currentVersion),
        notes:
          "Simulated release notes (dev build). This is a preview of the update UI — no real download will happen.",
        date: new Date().toISOString(),
      };
    }

    set({
      phase: "available",
      available: {
        version: meta.version,
        currentVersion,
        notes: meta.notes,
        date: meta.date,
      },
      progress: null,
      error: null,
      simulated: true,
    });
    // Intentionally stop here: the developer sees the "available" notice in
    // the banner + Settings card. The fake download only runs when they click
    // "Install now" (handled in `install()` via the `simulated` flag).
  },
}));

/** Trigger the post-install relaunch. Separate from the store so the store has
 *  no side effects on import and stays easy to test. */
export { relaunchApp };
