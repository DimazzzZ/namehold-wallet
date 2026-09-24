//! Whether the node is good enough to be believed, and how high the chain is.
//!
//! One question with several callers and several amounts of context: a Tauri
//! `State`, a settings map, a profile id and a DB path, or an injected client.
//! They all end at [`node_tip_height_if_synced_with_client`], so "synced" means
//! the same thing to the read gate, the background sync, the chain scanner and
//! the watched-name daemon — and a node on the wrong chain is refused by all of
//! them rather than by whichever happened to check.
//!
//! [`estimate_persisted_height`] is the other half: what to believe about the
//! chain height when no node answers. It lives here because it is the fallback
//! of the same question, and because the callers that need one usually need
//! both.
//!
//! This is a dedicated helper module rather than more surface on
//! `commands::read`, which CODING_STANDARDS names as a legacy shared layer not
//! to add to.

use tauri::State;

use crate::db::queries;
use rusqlite::OptionalExtension;

use crate::error::AppError;
use crate::AppState;

/// Check if the local hsd node is connected AND fully synced, making local
/// cached data the preferred read source. Returns `true` when the node RPC
/// answers and the chain is caught up (height ≥ headers, or progress ≥ 0.9999).
///
/// In SPV mode, always returns `false` — SPV nodes don't have `--index-address`
/// and can't serve UTXO queries, so all reads must go through the explorer.
pub(crate) async fn is_node_ready_for_local_reads(state: &State<'_, AppState>) -> bool {
    // SPV mode: node is never authoritative for reads.
    let (node_mode, expected_network) = {
        let db = match state.db.lock() {
            Ok(db) => db,
            Err(_) => return false,
        };
        let settings = match crate::db::queries::get_settings(&db) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let mode = crate::noncustodial::rpc::resolve_node_mode(&settings);
        // Resolve the active profile's network so we can reject a node on a
        // different chain (e.g. regtest node vs mainnet wallet).
        // A DB failure here degrades to "no network to compare" — the read
        // gate is a routing decision, not a security boundary, and the SPV /
        // sync gates below still apply. A caller that must not proceed on an
        // unknown network propagates the error instead of degrading.
        let net = queries::get_active_profile_network(&db).ok().flatten();
        (mode, net)
    };
    if node_mode.is_spv() {
        return false;
    }
    node_tip_height_if_synced_for_network(state, expected_network.as_deref())
        .await
        .is_some()
}

/// Like [`node_tip_height_if_synced`] but with an explicitly supplied
/// `expected_network`, for the one caller that has already resolved it
/// ([`is_node_ready_for_local_reads`]). Rejects the node when its reported
/// `chain` disagrees (e.g. a regtest node answering for a mainnet wallet).
/// `None` means "no network to compare" — see
/// [`node_tip_height_if_synced_with_client`] for why that is permissive.
pub(crate) async fn node_tip_height_if_synced_for_network(
    state: &State<'_, AppState>,
    expected_network: Option<&str>,
) -> Option<i64> {
    let settings = {
        let db = state.db.lock().ok()?;
        crate::db::queries::get_settings(&db).ok()?
    };
    node_tip_height_if_synced_from_settings_with_network(&settings, expected_network).await
}

/// The live node tip height, but ONLY when the node is connected, fully synced,
/// AND reporting the same chain as the active profile. `None` when the node is
/// unreachable, catching up, or on another network.
///
/// The expected network is resolved here rather than taken as an argument:
/// every `State`-based caller wants the active profile's chain, and a helper
/// that could be called without one is exactly how the cross-chain reads this
/// guard exists to prevent got in. Callers outside a `State` context use
/// [`node_tip_height_if_synced_from_settings_with_network`], which makes the
/// expected network an explicit argument they cannot forget.
pub(crate) async fn node_tip_height_if_synced(state: &State<'_, AppState>) -> Option<i64> {
    // A network this cannot read is a node it cannot vouch for: a DB failure
    // answers "not synced", not "nothing to compare". Only a genuinely absent
    // active profile (onboarding) skips the chain comparison.
    let expected_network = {
        let db = state.db.lock().ok()?;
        queries::get_active_profile_network(&db).ok()?
    };
    node_tip_height_if_synced_for_network(state, expected_network.as_deref()).await
}

/// Resolve the node's tip height from a settings map, returning `None` unless
/// the node is reachable and fully synced — and additionally rejecting when
/// its reported `chain` disagrees with `expected_network`. Set `expected_network` to the active profile's stored
/// network string — the schema allows only `"mainnet"`, `"testnet"` and
/// `"regtest"`; `"main"` and `"simnet"` are accepted defensively by the
/// comparison. Leave it `None` to skip the network check.
///
/// This is the guard that prevents a regtest node from being treated as
/// authoritative for a mainnet wallet (or any other cross-network mismatch).
/// The comparison normalizes both sides through
/// [`crate::noncustodial::network::network_name_matches`] so
/// `"mainnet"` (profile) and `"main"` (hsd) count as equal.
pub(crate) async fn node_tip_height_if_synced_from_settings_with_network(
    settings: &std::collections::HashMap<String, String>,
    expected_network: Option<&str>,
) -> Option<i64> {
    let client = crate::noncustodial::rpc::NodeRpcClient::from_settings(settings);
    node_tip_height_if_synced_with_client(&client, expected_network).await
}

/// Per-profile readiness probe: the live node tip height, but ONLY when the node
/// is connected, fully synced, AND reporting the same chain as the profile.
/// Returns None when the node is unreachable, catching up, on another network,
/// or the profile doesn't exist.
///
/// Per-profile node override routing (ADR-001): if the profile has a per-profile
/// override, it takes precedence; otherwise falls back to global settings; otherwise
/// uses the built-in default. This is the readiness probe for background daemons
/// (chain scanner, watched-name daemon) that operate on behalf of a specific profile.
pub(crate) async fn node_tip_height_if_synced_from_profile_with_network(
    db_path: &str,
    profile_id: &str,
    expected_network: Option<&str>,
) -> Option<i64> {
    let conn = match crate::db::connection::open(std::path::Path::new(db_path)) {
        Ok(c) => c,
        Err(_) => return None,
    };
    // ADR-001: a profile whose node config will not resolve is a configuration
    // error for that profile, not a fallback to global. This gate answers
    // "is this profile's node authoritative?", and global's node is not an
    // answer to that question — it may be another chain entirely.
    let client = crate::noncustodial::rpc::NodeRpcClient::for_profile(&conn, profile_id).ok()?;
    node_tip_height_if_synced_with_client(&client, expected_network).await
}

/// The client-injected core of [`node_tip_height_if_synced_from_settings_with_network`].
/// All the sync-progress + network-match logic lives here so it can be unit
/// tested against a `MockNodeRpc` without a live node. The settings-based
/// wrappers construct the real `NodeRpcClient` and delegate here.
pub(crate) async fn node_tip_height_if_synced_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    expected_network: Option<&str>,
) -> Option<i64> {
    let info = client.get_blockchain_info().await.ok()?;
    // Reject the node only on a POSITIVE mismatch. `network_check` returns
    // `None` when either side is unknown (no profile, or a node that doesn't
    // report `chain` — older builds); we conservatively allow that, and the
    // SPV gate and other checks still apply.
    if crate::noncustodial::network::network_check(expected_network, info.chain.as_deref())
        == Some(false)
    {
        return None;
    }
    // Connected — now check if synced. No sync metadata at all (e.g. regtest
    // with a single miner) counts as synced.
    info.is_synced(/* assume_when_unknown */ true)
        .then_some(info.blocks)
}

/// Settings-based readiness gate: `true` when the local node is connected, fully
/// synced, AND reporting `expected_network`. Mirrors
/// [`is_node_ready_for_local_reads`] for callers that only have settings/a DB
/// connection (the background sync thread, the chain scanner, the watched-name
/// daemon). Pass the active profile's stored network string; `None` skips the
/// comparison and should only be used where no profile exists.
pub async fn node_ready_from_settings(
    settings: &std::collections::HashMap<String, String>,
    expected_network: Option<&str>,
) -> bool {
    node_tip_height_if_synced_from_settings_with_network(settings, expected_network)
        .await
        .is_some()
}

/// Per-profile readiness gate: true when the node is connected, fully synced,
/// AND reporting the profile's network. Mirrors node_ready_from_settings for
/// callers that have a profile ID and a DB path (background daemons).
/// Returns false when the profile doesn't exist or the node is unreachable.
pub async fn node_ready_from_profile(
    db_path: &str,
    profile_id: &str,
    expected_network: Option<&str>,
) -> bool {
    node_tip_height_if_synced_from_profile_with_network(db_path, profile_id, expected_network)
        .await
        .is_some()
}

/// Best persisted estimate of the current chain height when no synced node is
/// available, extrapolated to "now" by elapsed wall time (~10-minute blocks).
/// Extrapolation matters for safety: a stale snapshot UNDERestimates the
/// height and therefore INFLATES days-until-expiry — the dangerous direction.
///
/// Candidates (max wins):
/// * per-name explorer/node stats snapshots persisted in
///   `tracked_name_states.raw_json` — `renewalPeriodEnd - blocksUntilExpire`
///   is the chain height the stats were computed at, aged by `updated_at`;
/// * `wallet_profiles.last_synced_height`, aged by `last_synced_at`.
pub(crate) fn estimate_persisted_height(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Option<i64>, AppError> {
    // Ageing a stored height by wall clock assumes blocks arrive on a schedule.
    // They do on main and testnet; on regtest and simnet they are mined on
    // demand, so the same arithmetic invents six blocks an idle hour never
    // produced and every renewal countdown drifts. There, report the stored
    // height as-is: stale but true.
    let ages_by_wall_clock =
        crate::commands::active_profile::profile_network_from_conn(conn, profile_id)?
            .has_wall_clock_block_timing();
    let age = |elapsed: i64| {
        if ages_by_wall_clock {
            elapsed.max(0)
        } else {
            0
        }
    };

    let mut best: Option<i64> = None;
    let mut consider = |h: Option<i64>| {
        if let Some(h) = h {
            best = Some(best.map_or(h, |b| b.max(h)));
        }
    };

    // Per-name stats snapshots. raw_json is either the explorer HsdName shape
    // (stats at the root) or the node getnameinfo result ({"info": {...}}).
    let mut stmt = conn.prepare(
        "SELECT raw_json,
                CAST((strftime('%s','now') - strftime('%s', updated_at)) / 600 AS INTEGER)
         FROM tracked_name_states
         WHERE wallet_profile_id = ?1 AND raw_json IS NOT NULL",
    )?;
    let rows = stmt.query_map(rusqlite::params![profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (raw, elapsed_blocks) = row?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let info = match v.get("info") {
            Some(i) if !i.is_null() => i,
            _ => &v,
        };
        let Some(stats) = info.get("stats").filter(|s| !s.is_null()) else {
            continue;
        };
        let end = stats.get("renewalPeriodEnd").and_then(|x| x.as_i64());
        let until = stats.get("blocksUntilExpire").and_then(|x| x.as_i64());
        if let (Some(end), Some(until)) = (end, until) {
            consider(Some(end - until + age(elapsed_blocks)));
        }
    }

    // Last node-synced height (stale, but still a floor), aged the same way.
    let profile_snapshot: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT last_synced_height,
                    CAST((strftime('%s','now') - strftime('%s', COALESCE(last_synced_at, datetime('now')))) / 600 AS INTEGER)
             FROM wallet_profiles WHERE id = ?1",
            rusqlite::params![profile_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((Some(h), elapsed_blocks)) = profile_snapshot {
        consider(Some(h + age(elapsed_blocks)));
    }

    Ok(best)
}
