/**
 * Browser-safe clipboard wrapper.
 *
 * In Tauri: delegates to `@tauri-apps/plugin-clipboard-manager`.
 * In browser: falls back to the Web Clipboard API (navigator.clipboard).
 */
import { isTauri } from "./runtime";

export async function writeText(text: string): Promise<void> {
  if (isTauri()) {
    const { writeText: tauriWrite } = await import(
      "@tauri-apps/plugin-clipboard-manager"
    );
    return tauriWrite(text);
  }
  return navigator.clipboard.writeText(text);
}

export async function readText(): Promise<string> {
  if (isTauri()) {
    const { readText: tauriRead } = await import(
      "@tauri-apps/plugin-clipboard-manager"
    );
    return tauriRead();
  }
  return navigator.clipboard.readText();
}
