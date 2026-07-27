import { isBrowser } from "./runtime";

/**
 * The Handshake block explorer used for all human-facing links.
 * Shakeshift renders HTML pages only; the wallet's JSON read API is a
 * separate concern (see `explorer_api_url` setting).
 */
export const SHAKESHIFT_BASE = "https://shakeshift.com";

/**
 * Legacy alias for the tx page base. Retained so any out-of-tree callers keep
 * working; new code should use {@link explorerTxUrl}.
 */
export const EXPLORER_TX_BASE = `${SHAKESHIFT_BASE}/transaction`;

/** Build the explorer URL for a transaction id. */
export function explorerTxUrl(txid: string): string {
  return `${SHAKESHIFT_BASE}/transaction/${txid}`;
}

/**
 * Build the explorer URL for a Handshake name.
 *
 * `encodeURIComponent` handles emoji and non-ASCII names (Shakeshift accepts
 * both the raw punycode form `xn--…` and the percent-encoded UTF-8 form).
 */
export function explorerNameUrl(name: string): string {
  return `${SHAKESHIFT_BASE}/name/${encodeURIComponent(name)}`;
}

/** Build the explorer URL for a Handshake address (hs1…). */
export function explorerAddressUrl(address: string): string {
  return `${SHAKESHIFT_BASE}/address/${address}`;
}

/** Build the explorer URL for a block height. */
export function explorerBlockUrl(height: number): string {
  return `${SHAKESHIFT_BASE}/block/${height}`;
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
