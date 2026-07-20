/**
 * Best-effort clipboard auto-clear for secret values copied from the secure
 * window (e.g. a revealed seed phrase).
 *
 * Kept in its own side-effect-free module (rather than inline in `main.ts`)
 * specifically so it can be unit tested: `main.ts` is a standalone
 * DOM-bootstrapping entry point (it calls `main()` at import time and reaches
 * for `document`/Tauri window APIs), so importing it in a test would run the
 * whole secure-window bootstrap. This module has no such side effects.
 */

export interface ClipboardOps {
  readText: () => Promise<string>;
  writeText: (text: string) => Promise<void>;
}

/**
 * Clear the clipboard, but ONLY if it still holds exactly the `expected`
 * value — so we don't clobber something the user copied elsewhere in the
 * meantime.
 *
 * If the platform can't read the clipboard back (`readText` throws — e.g.
 * permission denied, or a platform that doesn't support clipboard read),
 * this falls back to an UNCONDITIONAL overwrite with an empty string. That's
 * a deliberate tradeoff, not an oversight: leaving a seed phrase sitting in
 * the clipboard indefinitely is worse than occasionally clearing something
 * the user copied afterward.
 */
export async function clearClipboardIfUnchanged(
  expected: string,
  ops: ClipboardOps,
): Promise<void> {
  let shouldClear = true;
  try {
    const current = await ops.readText();
    shouldClear = current === expected;
  } catch {
    // Can't read back — fall through to the unconditional-clear fallback
    // documented above.
  }
  if (!shouldClear) return;
  try {
    await ops.writeText("");
  } catch {
    /* clipboard unavailable; ignore */
  }
}

/** Default delay before a copied secret is auto-cleared from the clipboard. */
export const CLIPBOARD_CLEAR_MS = 30_000;

/** Schedule the compare-and-clear above to run after `delayMs`. */
export function scheduleClipboardClear(
  expected: string,
  ops: ClipboardOps,
  delayMs: number = CLIPBOARD_CLEAR_MS,
): ReturnType<typeof setTimeout> {
  return setTimeout(() => {
    void clearClipboardIfUnchanged(expected, ops);
  }, delayMs);
}
