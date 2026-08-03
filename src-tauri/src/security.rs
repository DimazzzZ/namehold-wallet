//! Shared security constants and helpers.
//!
//! Centralizes the definition of sensitive setting keys and renderer-write
//! restrictions so that `get_settings`, `update_setting`, `get_audit_log`, and
//! the Namebase client all reference a single source of truth.

/// Setting keys whose values are secrets and MUST NOT be exposed to the
/// renderer (React webview) or logged in plaintext in the audit log.
pub const SENSITIVE_SETTING_KEYS: &[&str] = &[
    "namebase_cookie",
    "namebase_cookie_v1",
    "hsrd_authorization",
];

/// Setting keys that the renderer is NOT allowed to write via `update_setting`.
/// These are either security-critical host overrides (whose mutation could
/// redirect authenticated requests) or secrets that should only be written by
/// dedicated backend flows (e.g. `connect_namebase`).
pub const RENDERER_WRITE_DENYLIST: &[&str] =
    &["namebase_base_url", "namebase_cookie", "namebase_cookie_v1"];

/// Returns `true` if `key` is a sensitive setting that must be redacted.
pub fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_SETTING_KEYS.contains(&key)
}

/// Returns `true` if `key` must not be written from the renderer.
pub fn is_renderer_write_denied(key: &str) -> bool {
    RENDERER_WRITE_DENYLIST.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_keys_cover_known_secrets() {
        assert!(is_sensitive_key("namebase_cookie"));
        assert!(is_sensitive_key("namebase_cookie_v1"));
        assert!(is_sensitive_key("hsrd_authorization"));
        assert!(!is_sensitive_key("advanced_mode"));
        assert!(!is_sensitive_key("__has_namebase_cookie"));
    }

    #[test]
    fn write_denylist_blocks_host_override() {
        assert!(is_renderer_write_denied("namebase_base_url"));
        assert!(is_renderer_write_denied("namebase_cookie"));
        assert!(is_renderer_write_denied("namebase_cookie_v1"));
        assert!(!is_renderer_write_denied("hsrd_rpc_url"));
        assert!(!is_renderer_write_denied("explorer_api_url"));
    }
}
