import { isBrowser } from "./runtime";

/**
 * The Handshake block explorer used for all human-facing links.
 * Shakeshift renders HTML pages only; the wallet's JSON read API is a
 * separate concern (see `explorer_api_url` setting).
 */
export const SHAKESHIFT_BASE = "https://shakeshift.com";

/** The GitHub repo this app is released from — the single source of truth for
 *  human-facing repo links (issues, release-notes doc links, etc.). */
export const GITHUB_REPO = "DimazzzZ/namehold-wallet";
export const GITHUB_REPO_URL = `https://github.com/${GITHUB_REPO}`;

/**
 * Resolve a link href found inside GitHub release notes to something the OS
 * browser can actually open.
 *
 * GitHub returns release bodies with **relative** links unchanged — e.g.
 * `[docs/RECOVER_LOST_BIDS.md](docs/RECOVER_LOST_BIDS.md)`. GitHub's own web
 * UI rewrites those to `…/blob/<ref>/<path>` at render time, but the raw API
 * body (which we render) keeps the bare relative path, so the link looks
 * clickable yet points nowhere useful. This rewrites a relative href to the
 * repo's `blob/<ref>/<path>` URL so it opens the file on GitHub.
 *
 * Absolute URLs (`https:`, `mailto:`, `#anchor`, protocol-relative `//`) are
 * returned unchanged. `ref` is the release tag (defaults to `HEAD` so a
 * relative link still resolves when we don't know the tag).
 */
export function resolveReleaseNotesHref(href: string, ref = "HEAD"): string {
  const h = (href ?? "").trim();
  if (!h) return h;
  // Absolute or non-path schemes: leave alone.
  if (/^[a-z][a-z0-9+.-]*:/i.test(h) || h.startsWith("//") || h.startsWith("#")) {
    return h;
  }
  // Strip a single leading "./" and any leading "/", then join under blob/<ref>.
  const clean = h.replace(/^\.?\//, "");
  return `${GITHUB_REPO_URL}/blob/${encodeURIComponent(ref)}/${clean}`;
}

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
