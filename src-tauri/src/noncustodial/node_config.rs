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
//! verbatim and never rewritten. A profile without one reaches its own
//! network's port over a default global URL, whatever its chain source (spec
//! 2026-10-24 R1).

use crate::error::AppError;
use crate::noncustodial::rpc::{resolve_node_api_key, ChainSource};

/// The built-in default node RPC URL when neither an override nor a global
/// setting supplies one: mainnet's default hsd port, from
/// [`Network::default_rpc_url`] rather than spelled out again here. In the
/// resolver only a profile whose network it cannot name ends up here: every
/// other profile takes its own network's port instead.
pub fn default_node_rpc_url() -> String {
    crate::noncustodial::network::Network::Main.default_rpc_url()
}

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
    /// True when the chain source is a local node (`local_node`, full or
    /// SPV): the profile is served by its network's local node.
    pub local_node: bool,
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

    // Whether the profile reads from its network's local node: the node a
    // wallet's activation starts (spec 2026-10-24 R4) is these wallets' alone.
    let local_node = !matches!(
        merged.get("chain_source").map(String::as_str),
        Some("remote_node" | "explorer")
    );
    // `None` is an unknown network (a spelling the parser does not know):
    // with no network to name a port, the URL falls back to the merged one.
    let profile_network = crate::db::queries::get_wallet_profile(conn, profile_id)?
        .and_then(|p| crate::noncustodial::network::Network::from_str_opt(&p.network));
    // The merged URL: the profile's own when it set one, else the app-wide.
    let url_setting = merged
        .get("node_rpc_url")
        .map(|u| u.trim())
        .filter(|u| !u.is_empty());
    let url_is_a_default =
        url_setting.is_none_or(crate::noncustodial::rpc::is_default_loopback_rpc_url);
    // A profile without a URL of its own, over a global URL that is only a
    // default, reaches its own network's port (ADR-0005, spec R1): that
    // default belongs to whichever network was started last, and following it
    // sent a mainnet wallet to a regtest node. This holds for every chain
    // source: the realign of 0.6.0 wrote that URL at every start whatever the
    // chain source was, so a remote-node wallet over such a leftover was sent
    // to another network's port too, and could not reach the node the
    // previous build had started for it (2026-10-28). A global URL that is not
    // a default (another host, a custom port) is the user's choice and is kept.
    let node_rpc_url = match profile_network {
        Some(network) if !chosen_here("node_rpc_url") && url_is_a_default => {
            network.default_rpc_url()
        }
        _ => url_setting
            .map(str::to_string)
            .unwrap_or_else(default_node_rpc_url),
    };
    let node_rpc_api_key = resolve_node_api_key(&merged);
    // Full or SPV is the profile's network's (spec 2026-10-24 R10), so the
    // chain source reads that network's mode rather than the app-wide one.
    let mode = crate::noncustodial::rpc::resolve_node_mode_for(&merged, profile_network);
    merged.insert("node_mode".to_string(), mode.as_str().to_string());
    let chain_source = ChainSource::from_settings(&merged);

    Ok(EffectiveNodeConfig {
        node_rpc_url,
        node_rpc_api_key,
        chain_source,
        from_override: NODE_CONFIG_KEYS.iter().any(|k| chosen_here(k)),
        local_node,
    })
}
