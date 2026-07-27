import { isBrowser } from "./runtime";

/**
 * The Handshake block explorer used for transaction links.
 * `shakeshift.com/transaction/<txid>` is the canonical human-facing tx page.
 */
export const EXPLORER_TX_BASE = "https://shakeshift.com/transaction";

/** Build the explorer URL for a transaction id. */
export function explorerTxUrl(txid: string): string {
  return `${EXPLORER_TX_BASE}/${txid}`;
}

/**
 * Open an external URL in the user's default system browser.
 *
 * In the browser (dev/preview/webqa mock) a plain `window.open` works. Inside
 * the Tauri webview, external navigation is blocked, so we route through the
 * `opener` plugin which hands the URL to the OS. Imported lazily so the plugin
 * is never pulled into the browser bundle path.
 */
export async function openExternal(url: string): Promise<void> {
  if (isBrowser()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}
