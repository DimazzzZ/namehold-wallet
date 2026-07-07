/**
 * Browser-safe dialog wrapper.
 *
 * In Tauri: delegates to `@tauri-apps/plugin-dialog`.
 * In browser: falls back to a simple prompt/confirm or returns null with a
 * console warning (native file pickers have no web equivalent).
 */
import { isTauri } from "./runtime";

export interface OpenDialogOptions {
  directory?: boolean;
  multiple?: boolean;
  title?: string;
  defaultPath?: string;
  filters?: { name: string; extensions: string[] }[];
}

export interface SaveDialogOptions {
  title?: string;
  defaultPath?: string;
  filters?: { name: string; extensions: string[] }[];
}

/**
 * Open a file/directory picker.
 *
 * In browser mode this always returns `null` — there is no equivalent to the
 * native OS picker. Callers should handle `null` gracefully (they already do,
 * because Tauri returns `null` when the user cancels).
 */
export async function open(
  options?: OpenDialogOptions,
): Promise<string | string[] | null> {
  if (isTauri()) {
    const { open: tauriOpen } = await import("@tauri-apps/plugin-dialog");
    return tauriOpen(options as Parameters<typeof tauriOpen>[0]);
  }
  console.warn(
    `[browser QA] File picker "${options?.title ?? "open"}" is not available — native dialog required.`,
  );
  return null;
}

/**
 * Open a save-file dialog.
 *
 * In browser mode returns `null` (same rationale as `open`).
 */
export async function save(
  options?: SaveDialogOptions,
): Promise<string | null> {
  if (isTauri()) {
    const { save: tauriSave } = await import("@tauri-apps/plugin-dialog");
    return tauriSave(options as Parameters<typeof tauriSave>[0]);
  }
  console.warn(
    `[browser QA] Save dialog "${options?.title ?? "save"}" is not available — native dialog required.`,
  );
  return null;
}
