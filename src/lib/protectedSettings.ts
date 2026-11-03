import { MAX_SIGNER_SESSION_SECS, MIN_SIGNER_SESSION_SECS } from "./settingsDefaults";

/**
 * The trust-root settings: which program runs as hsd and with which data
 * directory, which node and explorer the wallet trusts, where a node API key
 * is sent, whether a send may go through a remote node, and how long an
 * unlocked wallet stays unlocked. The backend refuses them from
 * `update_setting`; they are saved only through `update_protected_settings`,
 * which shows each change in the secure window and writes nothing unless the
 * user confirms there.
 *
 * Must equal `PROTECTED_SETTING_KEYS` in `src-tauri/src/security.rs`; the
 * contract test in `__tests__/protectedSettings.test.ts` reads that list.
 */
export const PROTECTED_SETTING_KEYS: readonly string[] = [
  "node_rpc_url",
  "node_rpc_api_key",
  "hsd_api_key",
  "chain_source",
  "node_mode",
  "node_mode.main",
  "node_mode.testnet",
  "node_mode.regtest",
  "node_mode.simnet",
  "allow_remote_broadcast",
  "hsd_path",
  "hsd_prefix",
  "explorer_api_url",
  "explorer_fallback_url",
  "signer_session_timeout_seconds",
];

/** Whether `key` is saved only after a confirmation in the secure window. */
export function isProtectedSettingKey(key: string): boolean {
  return PROTECTED_SETTING_KEYS.includes(key);
}

/** One change as `update_protected_settings` takes it. */
export interface SettingChange {
  key: string;
  value: string;
}

/**
 * The error text of `AppError::UserRejected`: the user declined the secure
 * window. Not a failure to report, only a save that did not happen.
 */
export const CONFIRMATION_DECLINED = "Confirmation declined";

/** Whether `e` is the backend's declined-confirmation error and nothing else. */
export function isConfirmationDeclined(e: unknown): boolean {
  return (e instanceof Error ? e.message : String(e)) === CONFIRMATION_DECLINED;
}

/**
 * The session-timeout rule `update_protected_settings` applies: empty (the
 * default) or a whole number of seconds the session uses as typed.
 * Returns the reason the value is refused, or `null`.
 */
export function sessionTimeoutError(raw: string): string | null {
  const value = raw.trim();
  if (value === "") return null;
  if (
    /^\d+$/.test(value) &&
    Number(value) >= MIN_SIGNER_SESSION_SECS &&
    Number(value) <= MAX_SIGNER_SESSION_SECS
  ) {
    return null;
  }
  return `Signer session timeout must be a whole number of seconds from ${MIN_SIGNER_SESSION_SECS} to ${MAX_SIGNER_SESSION_SECS}`;
}
