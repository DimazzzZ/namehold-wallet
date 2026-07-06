/**
 * Runtime detection: are we running inside a real Tauri desktop shell, or in a
 * plain browser (dev-server / preview / static hosting)?
 *
 * Tauri v2 injects `window.__TAURI_INTERNALS__` before any user JS runs, so
 * checking for that object is the most reliable signal.
 */
export function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** Convenience alias — true when we're in a browser without Tauri. */
export function isBrowser(): boolean {
  return !isTauri();
}
