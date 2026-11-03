//! The rules for changing a trust-root setting
//! ([`crate::security::PROTECTED_SETTING_KEYS`]): what a new value may be,
//! which requested changes change anything, and the rows the secure window
//! shows for them. `commands::settings::update_protected_settings` asks the
//! user with these rows and writes; this module never writes.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::AppError;
use crate::security;

/// One change of a setting as the renderer sends it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SettingChange {
    pub key: String,
    pub value: String,
}

/// A change that differs from what is stored: `old` is the value in effect
/// now (the stored one, or the default a missing or empty one reads as),
/// `new` the validated value that will be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedChange {
    pub key: String,
    pub old: String,
    pub new: String,
}

/// Title of the secure window's confirmation.
pub const CONFIRM_TITLE: &str = "Change connection settings";

/// What the confirmation asks of the user.
pub const CONFIRM_MESSAGE: &str = "Confirm only if you made these changes yourself in Settings. \
These settings decide which program Namehold runs as hsd, which node and explorer it trusts, \
where your node API key is sent, and how long an unlocked wallet stays unlocked.";

/// The keys whose change can send an api-key somewhere else: the node URL,
/// the key itself, and the data directory whose `hsd.conf` supplies a key
/// when none is stored (`rpc::resolve_node_api_key`).
const NODE_TRANSPORT_KEYS: [&str; 3] = ["node_rpc_url", "node_rpc_api_key", "hsd_prefix"];

/// Validate and normalize the requested changes, then keep those that change
/// the value in effect. All or nothing: one key that is not a trust-root
/// setting, one key given twice or one invalid value refuses the whole
/// request. An empty result means there is nothing to ask and nothing to
/// write.
pub fn plan_changes(
    requested: &[SettingChange],
    stored: &HashMap<String, String>,
) -> Result<Vec<PlannedChange>, AppError> {
    let mut seen: Vec<&str> = Vec::with_capacity(requested.len());
    let mut planned = Vec::new();
    for change in requested {
        let key = change.key.as_str();
        if !security::is_protected_setting(key) {
            return Err(AppError::InvalidInput(format!(
                "setting '{key}' is not a connection setting"
            )));
        }
        if seen.contains(&key) {
            return Err(AppError::InvalidInput(format!(
                "setting '{key}' is given twice"
            )));
        }
        seen.push(key);
        let new = validate_value(key, &change.value)?;
        let old = effective_value(key, stored);
        if new != old {
            planned.push(PlannedChange {
                key: key.to_string(),
                old,
                new,
            });
        }
    }
    if planned
        .iter()
        .any(|c| NODE_TRANSPORT_KEYS.contains(&c.key.as_str()))
    {
        guard_node_transport(stored, &planned)?;
    }
    Ok(planned)
}

/// `rpc::guard_transport` over the settings as they will be: a node api-key
/// (stored, new, or read from the new data directory's `hsd.conf`) never goes
/// to a remote host over plaintext HTTP.
fn guard_node_transport(
    stored: &HashMap<String, String>,
    planned: &[PlannedChange],
) -> Result<(), AppError> {
    let after = settings_after(stored, planned);
    let key = crate::noncustodial::rpc::resolve_node_api_key(&after);
    crate::noncustodial::rpc::guard_transport(&node_url_in(&after), &key)
}

fn settings_after(
    stored: &HashMap<String, String>,
    planned: &[PlannedChange],
) -> HashMap<String, String> {
    let mut after = stored.clone();
    for c in planned {
        after.insert(c.key.clone(), c.new.clone());
    }
    after
}

/// The node URL `settings` reads as: the stored one, or the default.
fn node_url_in(settings: &HashMap<String, String>) -> String {
    settings
        .get("node_rpc_url")
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(crate::noncustodial::node_config::default_node_rpc_url)
}

/// The value of `key` in effect with `stored`: the stored value, or what a
/// missing or empty one reads as (`ChainSource::from_settings`,
/// `rpc::resolve_node_mode_for`, `rpc::remote_broadcast_allowed`). A network's
/// own node mode falls back to the app-wide one.
pub fn effective_value(key: &str, stored: &HashMap<String, String>) -> String {
    if let Some(v) = stored.get(key).filter(|v| !v.trim().is_empty()) {
        return v.clone();
    }
    if key.starts_with("node_mode.") {
        return effective_value("node_mode", stored);
    }
    match key {
        "chain_source" => "local_node",
        "node_mode" => "full",
        "allow_remote_broadcast" => "false",
        _ => "",
    }
    .to_string()
}

/// Validate a new value for the trust-root `key` and return it normalized
/// (trimmed; an hsd program as its canonical path). Refuses a value the
/// backend would misread or that points somewhere it must not.
pub fn validate_value(key: &str, raw: &str) -> Result<String, AppError> {
    let value = raw.trim();
    match key {
        "node_rpc_url" | "explorer_api_url" | "explorer_fallback_url" => {
            if !value.is_empty() {
                validate_http_url(key, value)?;
            }
            Ok(value.to_string())
        }
        "node_rpc_api_key" | "hsd_api_key" => Ok(value.to_string()),
        "chain_source" => one_of(key, value, &["local_node", "remote_node", "explorer"]),
        "allow_remote_broadcast" => one_of(key, value, &["true", "false"]),
        "hsd_path" => validate_hsd_path(value),
        "hsd_prefix" => validate_hsd_prefix(value),
        "signer_session_timeout_seconds" => {
            use crate::noncustodial::session::{MAX_SESSION_SECS, MIN_SESSION_SECS};
            let in_range = |n: &u64| (MIN_SESSION_SECS..=MAX_SESSION_SECS).contains(n);
            if value.is_empty() || value.parse::<u64>().is_ok_and(|n| in_range(&n)) {
                Ok(value.to_string())
            } else {
                Err(AppError::InvalidInput(format!(
                    "the signer session timeout must be a whole number of seconds from {MIN_SESSION_SECS} to {MAX_SESSION_SECS}"
                )))
            }
        }
        k if k == "node_mode" || k.starts_with("node_mode.") => {
            one_of(key, value, &["full", "spv"])
        }
        _ => Err(AppError::InvalidInput(format!(
            "setting '{key}' is not a connection setting"
        ))),
    }
}

fn one_of(key: &str, value: &str, allowed: &[&str]) -> Result<String, AppError> {
    if allowed.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(AppError::InvalidInput(format!(
            "setting '{key}' must be one of {}, not '{value}'",
            allowed.join(", ")
        )))
    }
}

/// An `https://` URL, or `http://` to this machine. The URL parser already
/// refuses either scheme without a host ("empty host"). The text the user
/// confirms has to name the host the app connects to, so only visible ASCII is
/// taken (the parser drops tabs and newlines and rewrites a look-alike Unicode
/// host), a user name or password is refused (`https://127.0.0.1:1@evil.example`
/// goes to evil.example), and the host must stand as written (a percent-encoded
/// or numeric spelling is rewritten by the parser).
fn validate_http_url(key: &str, value: &str) -> Result<(), AppError> {
    if !value.chars().all(|c| c.is_ascii_graphic()) {
        return Err(AppError::InvalidInput(format!(
            "setting '{key}' must be plain ASCII with no spaces; write an international host name in its xn-- form"
        )));
    }
    let parsed = url::Url::parse(value)
        .map_err(|e| AppError::InvalidInput(format!("setting '{key}' is not a valid URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::InvalidInput(format!(
            "setting '{key}' must be an http:// or https:// URL"
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::InvalidInput(format!(
            "setting '{key}' must not hold a user name or password"
        )));
    }
    let host = parsed.host_str().unwrap_or_default();
    let start = format!("{}://{host}", parsed.scheme());
    if !value
        .get(..start.len())
        .is_some_and(|v| v.eq_ignore_ascii_case(&start))
    {
        return Err(AppError::InvalidInput(format!(
            "setting '{key}' must spell its host the way it is used: {host}"
        )));
    }
    if parsed.scheme() == "http" && !crate::http::is_loopback_url(&parsed) {
        return Err(AppError::InvalidInput(format!(
            "setting '{key}' must use https:// for a host that is not this computer"
        )));
    }
    Ok(())
}

/// Whether the secure window could show `value` differently from what it is:
/// a control or formatting character (line breaks, bidi overrides, zero-width
/// marks) can make one path look like another.
fn refuse_unshowable_path(value: &str) -> Result<(), AppError> {
    let hidden = |c: char| {
        c.is_control()
            || matches!(c,
                '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
    };
    if value.chars().any(hidden) {
        return Err(AppError::InvalidInput(
            "a path must not hold control or invisible characters".to_string(),
        ));
    }
    Ok(())
}

/// The hsd program: empty means auto-detect. Otherwise an absolute path on
/// this machine that exists and resolves (links followed) to a regular file
/// that passes [`program_file_problem`]; the canonical path is returned, so
/// what the user confirms is the file that runs.
pub fn validate_hsd_path(value: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Ok(String::new());
    }
    refuse_unshowable_path(value)?;
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(AppError::InvalidInput(format!(
            "the hsd program must be given by its full path, not '{value}'"
        )));
    }
    refuse_network_path(value)?;
    let canonical = std::fs::canonicalize(path).map_err(|e| {
        AppError::InvalidInput(format!("the hsd program {value} cannot be found: {e}"))
    })?;
    let meta = std::fs::metadata(&canonical).map_err(|e| {
        AppError::InvalidInput(format!("the hsd program {value} cannot be read: {e}"))
    })?;
    if !meta.is_file() {
        return Err(AppError::InvalidInput(format!(
            "the hsd program {value} is not a file"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
        let me = unsafe { libc::geteuid() };
        if let Some(problem) = program_file_problem(meta.mode(), meta.uid(), me) {
            return Err(AppError::InvalidInput(format!(
                "the hsd program {value} {problem}"
            )));
        }
    }
    let canonical = canonical.to_str().ok_or_else(|| {
        AppError::InvalidInput(format!("the hsd program path {value} is not valid text"))
    })?;
    refuse_network_path(canonical)?;
    refuse_unshowable_path(canonical)?;
    Ok(canonical.to_string())
}

/// Why a file with this mode and owner must not run as hsd: it needs an execute
/// bit, and nobody but its owner may change it, since the path is confirmed
/// once and the file runs every time. The owner is root or the user running the
/// app (Homebrew, npm, Nix and system packages all install so).
#[cfg(unix)]
pub(crate) fn program_file_problem(mode: u32, owner: u32, me: u32) -> Option<&'static str> {
    if mode & 0o111 == 0 {
        Some("is not executable")
    } else if mode & 0o022 != 0 {
        Some("can be changed by other users; remove its group and other write permission")
    } else if owner != 0 && owner != me {
        Some("belongs to another user")
    } else {
        None
    }
}

/// The hsd data directory: empty means hsd's default. Otherwise an absolute
/// path that is a directory or does not exist yet (`start_hsd` creates it);
/// a file is refused.
pub fn validate_hsd_prefix(value: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Ok(String::new());
    }
    refuse_unshowable_path(value)?;
    if !Path::new(value).is_absolute() {
        return Err(AppError::InvalidInput(format!(
            "the hsd data directory must be a full path, not '{value}'"
        )));
    }
    refuse_network_path(value)?;
    match std::fs::metadata(value) {
        Ok(meta) if meta.is_dir() => Ok(value.to_string()),
        Ok(_) => Err(AppError::InvalidInput(format!(
            "the hsd data directory {value} is not a directory"
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(value.to_string()),
        Err(e) => Err(AppError::InvalidInput(format!(
            "the hsd data directory {value} cannot be used: {e}"
        ))),
    }
}

/// A Windows network path (`\\server\share`, `//server/share`, `\\?\UNC\…`)
/// would let a remote share supply the program or its data directory; it is
/// refused everywhere. Windows takes either slash and any case in the prefix.
pub(crate) fn refuse_network_path(value: &str) -> Result<(), AppError> {
    let path = value.replace('/', r"\").to_ascii_uppercase();
    if path.starts_with(r"\\?\UNC\") || (path.starts_with(r"\\") && !path.starts_with(r"\\?\")) {
        return Err(AppError::InvalidInput(format!(
            "{value} is a network path; only a path on this computer can be used"
        )));
    }
    Ok(())
}

/// The secure window's rows: one per change, labelled with the setting and
/// valued "old → new". An api-key's value is never shown: its row says set,
/// changed or cleared. A change that hands a key the wallet already has to a
/// new host adds a warning row naming that host. The button says what it does,
/// since nothing is signed.
pub fn confirmation_rows(
    changes: &[PlannedChange],
    stored: &HashMap<String, String>,
) -> serde_json::Value {
    let mut rows: Vec<serde_json::Value> = changes
        .iter()
        .map(|c| serde_json::json!({ "label": label(&c.key), "value": change_text(c) }))
        .collect();
    if let Some(host) = host_receiving_existing_key(changes, stored) {
        rows.push(serde_json::json!({
            "label": "Warning",
            "value": format!("Your node API key will be sent to {host}.")
        }));
    }
    for host in unknown_explorer_hosts(changes) {
        rows.push(serde_json::json!({
            "label": "Warning",
            "value": format!(
                "{host} is not an explorer Namehold knows. It sees your addresses and names, and \
                 what it answers is shown when your node cannot answer. Check it is the site you \
                 meant: fake explorers have been listed on trusted sites before."
            )
        }));
    }
    serde_json::json!({ "rows": rows, "confirmLabel": "Save settings" })
}

/// Explorers that serve this app's reads. Exact hosts: a look-alike or a
/// subdomain of another site is exactly what the warning is for.
const KNOWN_EXPLORER_HOSTS: &[&str] = &["e.hnsfans.com", "shakeshift.com"];

/// The hosts of explorer URLs in `changes` that are neither known nor this
/// machine. A warning, not a refusal: running your own explorer is legitimate.
fn unknown_explorer_hosts(changes: &[PlannedChange]) -> Vec<String> {
    let mut hosts: Vec<String> = changes
        .iter()
        .filter(|c| matches!(c.key.as_str(), "explorer_api_url" | "explorer_fallback_url"))
        .filter_map(|c| url::Url::parse(c.new.trim()).ok())
        .filter(|u| !crate::http::is_loopback_url(u))
        .filter_map(|u| u.host_str().map(str::to_string))
        .filter(|h| !KNOWN_EXPLORER_HOSTS.contains(&h.as_str()))
        .collect();
    hosts.dedup();
    hosts
}

/// The new host, when `changes` move the node to another host and a key the
/// user did not type in this same change will go there.
fn host_receiving_existing_key(
    changes: &[PlannedChange],
    stored: &HashMap<String, String>,
) -> Option<String> {
    let host_of = |url: &str| url::Url::parse(url).ok()?.host_str().map(str::to_string);
    let after = settings_after(stored, changes);
    let new_host = host_of(&node_url_in(&after))?;
    let typed_key = changes
        .iter()
        .any(|c| c.key == "node_rpc_api_key" && !c.new.is_empty());
    let moved = host_of(&node_url_in(stored)).as_deref() != Some(new_host.as_str());
    (moved && !typed_key && !crate::noncustodial::rpc::resolve_node_api_key(&after).is_empty())
        .then_some(new_host)
}

fn change_text(c: &PlannedChange) -> String {
    if security::is_sensitive_key(&c.key) {
        return match (c.old.trim().is_empty(), c.new.trim().is_empty()) {
            (true, _) => "set",
            (false, false) => "changed",
            (false, true) => "cleared",
        }
        .to_string();
    }
    format!("{} → {}", shown(&c.key, &c.old), shown(&c.key, &c.new))
}

fn label(key: &str) -> String {
    if let Some(net) = key.strip_prefix("node_mode.") {
        let net = if net == "main" { "mainnet" } else { net };
        return format!("Node mode ({net})");
    }
    match key {
        "node_rpc_url" => "Node RPC URL",
        "node_rpc_api_key" => "Node RPC API key",
        "hsd_api_key" => "hsd API key (legacy)",
        "chain_source" => "Chain source",
        "node_mode" => "Node mode",
        "allow_remote_broadcast" => "Sending through a remote node",
        "hsd_path" => "hsd program",
        "hsd_prefix" => "hsd data directory",
        "explorer_api_url" => "Explorer URL",
        "explorer_fallback_url" => "Explorer fallback URL",
        "signer_session_timeout_seconds" => "Signer session timeout",
        other => other,
    }
    .to_string()
}

/// A value as the window shows it: the words for a known choice or an empty
/// value, otherwise the value itself.
fn shown(key: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return match key {
            "hsd_path" => "auto-detect",
            "hsd_prefix" => "hsd's default (~/.hsd)",
            "explorer_api_url" => "the network's default explorer",
            "explorer_fallback_url" => "none",
            "signer_session_timeout_seconds" => "the default (900 s)",
            "node_rpc_url" => "the default node",
            _ => "(empty)",
        }
        .to_string();
    }
    let words = match (key, value) {
        ("chain_source", "local_node") => Some("local node"),
        ("chain_source", "remote_node") => Some("remote node"),
        ("chain_source", "explorer") => Some("explorer (read-only)"),
        ("allow_remote_broadcast", "true") => Some("allowed"),
        ("allow_remote_broadcast", "false") => Some("not allowed"),
        (k, "full") if k == "node_mode" || k.starts_with("node_mode.") => Some("full node"),
        (k, "spv") if k == "node_mode" || k.starts_with("node_mode.") => Some("SPV (headers only)"),
        _ => None,
    };
    if let Some(w) = words {
        return w.to_string();
    }
    if key == "signer_session_timeout_seconds" && value.parse::<u64>().is_ok() {
        return format!("{value} s");
    }
    value.to_string()
}
