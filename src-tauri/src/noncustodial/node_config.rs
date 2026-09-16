//! Per-profile node configuration resolution (ADR-001).
//!
//! The wallet stores node configuration in two places: the global `settings`
//! table (applies to every profile) and the per-profile `profile_settings`
//! table (applies only to one profile). This module resolves the two into a
//! single [`EffectiveNodeConfig`] for a given profile, applying the ADR-001
//! resolution order: per-profile override -> global setting -> built-in
//! default, evaluated independently per key.
//!
//! A per-profile override is an explicit user choice: when present it is used
//! verbatim and never silently rewritten (e.g. `realign_loopback_rpc_url` is
//! skipped for an overridden URL — see [`EffectiveNodeConfig::from_override`]).

use std::collections::HashMap;

use crate::error::AppError;
use crate::noncustodial::rpc::{resolve_node_api_key, ChainSource};

/// The built-in default node RPC URL when neither an override nor a global
/// setting supplies one. Mainnet's default hsd node port; realign fixes the
/// port for other networks when this comes from the global fallback.
pub const DEFAULT_NODE_RPC_URL: &str = "http://127.0.0.1:12037";

/// The resolved node configuration for one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNodeConfig {
    /// Resolved node RPC URL.
    pub node_rpc_url: String,
    /// Resolved node RPC api-key ("" when the node needs none).
    pub node_rpc_api_key: String,
    /// Resolved chain source.
    pub chain_source: ChainSource,
    /// True when at least one node-config key came from the profile's
    /// `profile_settings` override (as opposed to global/default). Callers use
    /// this to skip `realign_loopback_rpc_url`, which must only touch a URL that
    /// came from the global fallback.
    pub from_override: bool,
}

/// Read all `profile_settings` rows for a profile into a key/value map.
pub fn get_profile_settings(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<HashMap<String, String>, AppError> {
    let mut stmt =
        conn.prepare("SELECT key, value FROM profile_settings WHERE profile_id = ?1")?;
    let rows = stmt.query_map([profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (k, v) = row?;
        map.insert(k, v);
    }
    Ok(map)
}

/// Resolve the effective node configuration for `profile_id`.
///
/// Resolution order per key: per-profile override -> global `settings` ->
/// built-in default. Returns [`AppError::NotFound`] when the profile does not
/// exist. Does not mutate or normalize the profile's network; it only resolves
/// the node endpoint tuple `(node_rpc_url, node_rpc_api_key, chain_source)`.
pub fn effective_node_config_for_profile(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<EffectiveNodeConfig, AppError> {
    // A missing profile is a hard error, never a silent default (ADR-001).
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM wallet_profiles WHERE id = ?1",
        [profile_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(AppError::NotFound(format!("wallet profile {profile_id}")));
    }

    let global = crate::db::queries::get_settings(conn)?;
    let overrides = get_profile_settings(conn, profile_id)?;

    // Build a merged settings view (override -> global) so the existing
    // resolvers (`resolve_node_api_key`, `ChainSource::from_settings`) apply the
    // same precedence without re-implementing their rules. A non-empty override
    // wins; an empty override string is "no meaningful choice" and must not
    // blank out a good global value. `from_override` is set when any override
    // key actually changes the resolved value away from the global fallback.
    let mut merged = global.clone();
    let mut from_override = false;
    for (k, v) in &overrides {
        if v.trim().is_empty() {
            continue;
        }
        if merged.get(k).map(String::as_str) != Some(v.as_str()) {
            from_override = true;
        }
        merged.insert(k.clone(), v.clone());
    }

    let node_rpc_url = merged
        .get("node_rpc_url")
        .map(String::to_string)
        .unwrap_or_else(|| DEFAULT_NODE_RPC_URL.to_string());
    let node_rpc_api_key = resolve_node_api_key(&merged);
    let chain_source = ChainSource::from_settings(&merged);

    Ok(EffectiveNodeConfig {
        node_rpc_url,
        node_rpc_api_key,
        chain_source,
        from_override,
    })
}
