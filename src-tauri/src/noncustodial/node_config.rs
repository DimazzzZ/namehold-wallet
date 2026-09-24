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
//! skipped for an overridden URL — see
//! [`EffectiveNodeConfig::url_from_override`]).

use crate::error::AppError;
use crate::noncustodial::rpc::{resolve_node_api_key, ChainSource};

/// The built-in default node RPC URL when neither an override nor a global
/// setting supplies one. Mainnet's default hsd node port; realign fixes the
/// port for other networks when this comes from the global fallback.
pub const DEFAULT_NODE_RPC_URL: &str = "http://127.0.0.1:12037";

/// The `profile_settings` keys that make up the ADR-001 node-config tuple.
/// A profile may hold other per-profile settings; those say nothing about
/// which node this profile talks to, so they must not make the config read as
/// overridden.
const NODE_CONFIG_KEYS: [&str; 3] = ["node_rpc_url", "node_rpc_api_key", "chain_source"];

/// The resolved node configuration for one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNodeConfig {
    /// Resolved node RPC URL.
    pub node_rpc_url: String,
    /// Resolved node RPC api-key ("" when the node needs none).
    pub node_rpc_api_key: String,
    /// Resolved chain source.
    pub chain_source: ChainSource,
    /// True when the profile's own `profile_settings` supplied any key of the
    /// node-config tuple — whatever value it holds. "The user chose this here",
    /// not "this differs from global": a user who deliberately pins a profile
    /// to the value global happens to carry today has still chosen it, and
    /// global can change under them afterwards.
    pub from_override: bool,
    /// True when the profile's own `profile_settings` supplied the resolved
    /// `node_rpc_url`. This is the one realign must consult: it rewrites the
    /// URL and nothing else, so an override of `chain_source` alone is no
    /// reason to leave a stale global loopback port pointing at the wrong
    /// network (ADR-001, "Interaction with N11").
    pub url_from_override: bool,
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
    if !crate::db::queries::wallet_profile_exists(conn, profile_id)? {
        return Err(AppError::NotFound(format!("wallet profile {profile_id}")));
    }

    let global = crate::db::queries::get_settings(conn)?;
    let overrides = crate::db::queries::get_profile_settings(conn, profile_id)?;

    // Build a merged settings view (override -> global) so the existing
    // resolvers (`resolve_node_api_key`, `ChainSource::from_settings`) apply the
    // same precedence without re-implementing their rules. A non-empty override
    // wins; an empty override string is "no meaningful choice" and must not
    // blank out a good global value.
    let chosen_here = |key: &str| {
        overrides
            .get(key)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    };
    let mut merged = global.clone();
    for (k, v) in &overrides {
        if v.trim().is_empty() {
            continue;
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
        from_override: NODE_CONFIG_KEYS.iter().any(|k| chosen_here(k)),
        url_from_override: chosen_here("node_rpc_url"),
    })
}
