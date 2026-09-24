//! Tests for per-profile node configuration resolution (ADR-001).
//!
//! Contract under test (`effective_node_config_for_profile`):
//!   1. Resolution order, per key: per-profile override -> global `settings`
//!      -> built-in default.
//!   2. A missing profile is [`AppError::NotFound`], never a silent default.
//!   3. An override sets `from_override = true` so callers skip realign; a
//!      pure global/default resolution sets it `false`.
//!   4. Resolution is per-key: an override for one key falls through to global
//!      for the others.
//!   5. The profile's network is never touched by resolution.

use rusqlite::Connection;

use crate::db::queries::get_profile_settings;
use crate::error::AppError;
use crate::noncustodial::node_config::{effective_node_config_for_profile, DEFAULT_NODE_RPC_URL};
use crate::noncustodial::rpc::ChainSource;

/// A migrated in-memory DB with the migrations the resolver depends on, plus
/// one `regtest` profile `p1`. Uses the real migration runner so schema drift
/// (including migration 027) is exercised end to end.
fn db_with_profile() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    crate::db::migrations::run(&conn).unwrap();
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
         VALUES ('p1', 'Test', 'watch_only_xpub', 'regtest', 'xpubPLACEHOLDER')",
        [],
    )
    .unwrap();
    conn
}

fn set_global(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .unwrap();
}

fn set_override(conn: &Connection, profile_id: &str, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO profile_settings (profile_id, key, value) VALUES (?1, ?2, ?3)
         ON CONFLICT(profile_id, key) DO UPDATE SET value = excluded.value",
        rusqlite::params![profile_id, key, value],
    )
    .unwrap();
}

#[test]
fn migration_027_creates_profile_settings_table() {
    let conn = db_with_profile();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='profile_settings'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "migration 027 must create profile_settings");
}

#[test]
fn deleting_profile_cascades_its_overrides() {
    let conn = db_with_profile();
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:14037");
    conn.execute("DELETE FROM wallet_profiles WHERE id = 'p1'", [])
        .unwrap();
    let left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM profile_settings WHERE profile_id = 'p1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0, "overrides must cascade-delete with the profile");
}

#[test]
fn missing_profile_is_not_found() {
    let conn = db_with_profile();
    let err = effective_node_config_for_profile(&conn, "does-not-exist").unwrap_err();
    assert!(
        matches!(err, AppError::NotFound(_)),
        "missing profile must be NotFound, got {err:?}"
    );
}

#[test]
fn falls_back_to_builtin_default_when_nothing_set() {
    let conn = db_with_profile();
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, DEFAULT_NODE_RPC_URL);
    assert_eq!(cfg.node_rpc_api_key, "");
    assert_eq!(cfg.chain_source, ChainSource::LocalNode);
    assert!(!cfg.from_override, "pure default must not be an override");
}

#[test]
fn uses_global_settings_when_no_override() {
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://global.example:12037");
    set_global(&conn, "node_rpc_api_key", "globalkey");
    set_global(&conn, "chain_source", "remote_node");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://global.example:12037");
    assert_eq!(cfg.node_rpc_api_key, "globalkey");
    assert_eq!(cfg.chain_source, ChainSource::RemoteNode);
    assert!(!cfg.from_override, "global-only must not be an override");
}

#[test]
fn override_takes_precedence_over_global() {
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://global.example:12037");
    set_global(&conn, "node_rpc_api_key", "globalkey");
    set_global(&conn, "chain_source", "remote_node");
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:14037");
    set_override(&conn, "p1", "node_rpc_api_key", "localkey");
    set_override(&conn, "p1", "chain_source", "local_node");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
    assert_eq!(cfg.node_rpc_api_key, "localkey");
    assert_eq!(cfg.chain_source, ChainSource::LocalNode);
    assert!(
        cfg.from_override,
        "any override key must flag from_override"
    );
}

#[test]
fn resolution_is_per_key() {
    // Override only the URL; api-key and chain_source must still come from
    // global. from_override is still true because one key was overridden.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_api_key", "globalkey");
    set_global(&conn, "chain_source", "remote_node");
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:14037");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
    assert_eq!(cfg.node_rpc_api_key, "globalkey");
    assert_eq!(cfg.chain_source, ChainSource::RemoteNode);
    assert!(cfg.from_override);
}

#[test]
fn override_does_not_change_profile_network() {
    let conn = db_with_profile();
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:14037");
    let _ = effective_node_config_for_profile(&conn, "p1").unwrap();
    let network: String = conn
        .query_row(
            "SELECT network FROM wallet_profiles WHERE id = 'p1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(network, "regtest", "resolution must not touch the network");
}

#[test]
fn get_profile_settings_returns_only_that_profiles_rows() {
    let conn = db_with_profile();
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
         VALUES ('p2', 'Other', 'watch_only_xpub', 'mainnet', 'xpubOTHER')",
        [],
    )
    .unwrap();
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:14037");
    set_override(&conn, "p2", "node_rpc_url", "http://127.0.0.1:12037");
    let m = get_profile_settings(&conn, "p1").unwrap();
    assert_eq!(
        m.get("node_rpc_url").map(String::as_str),
        Some("http://127.0.0.1:14037")
    );
    assert_eq!(m.len(), 1, "must not leak other profiles' overrides");
}

#[test]
fn empty_override_value_is_ignored_for_url() {
    // An override row with an empty URL must not blank out a good global URL;
    // an empty string is not a meaningful endpoint choice.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://global.example:12037");
    set_override(&conn, "p1", "node_rpc_url", "");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://global.example:12037");
}

// --- Seam: NodeRpcClient constructed from the effective config -------------
//
// Step 2 (ADR-001) routes call sites through a profile-aware constructor
// rather than `NodeRpcClient::from_settings(&global)`. These tests pin that
// the constructor resolves override -> global -> default and carries the
// resolved url/key/source onto the client.

use crate::noncustodial::rpc::NodeRpcClient;

#[test]
fn for_profile_uses_override_url_key_and_source() {
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://global.example:12037");
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:14037");
    set_override(&conn, "p1", "node_rpc_api_key", "override-key");
    set_override(&conn, "p1", "chain_source", "remote_node");
    let client = NodeRpcClient::for_profile(&conn, "p1").unwrap();
    assert_eq!(client.node_url(), "http://127.0.0.1:14037");
    assert_eq!(client.api_key(), "override-key");
    assert_eq!(client.source(), ChainSource::RemoteNode);
}

#[test]
fn for_profile_falls_through_to_global_then_default() {
    let conn = db_with_profile();
    // No override, no global url -> built-in default; global key present.
    set_global(&conn, "node_rpc_api_key", "global-key");
    let client = NodeRpcClient::for_profile(&conn, "p1").unwrap();
    assert_eq!(client.node_url(), DEFAULT_NODE_RPC_URL);
    assert_eq!(client.api_key(), "global-key");
}

#[test]
fn for_profile_missing_profile_is_not_found() {
    let conn = db_with_profile();
    match NodeRpcClient::for_profile(&conn, "does-not-exist") {
        Err(AppError::NotFound(_)) => {}
        Err(other) => panic!("expected NotFound, got {other:?}"),
        Ok(_) => panic!("expected NotFound, got a client for a missing profile"),
    }
}

#[test]
fn from_effective_config_carries_resolved_fields() {
    let conn = db_with_profile();
    set_override(&conn, "p1", "node_rpc_url", "https://node.example:12037");
    set_override(&conn, "p1", "node_rpc_api_key", "k");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    let client = NodeRpcClient::from_effective_config(&cfg);
    assert_eq!(client.node_url(), "https://node.example:12037");
    assert_eq!(client.api_key(), "k");
}

// --- Which flag realign is allowed to consult (ADR-001, N11) ---

#[test]
fn an_override_equal_to_global_is_still_the_users_choice() {
    // The flag used to mean "differs from global", so pinning a profile to the
    // value global happens to hold today read as no override at all — and
    // realign would then rewrite a URL the user had deliberately set. Global
    // can also change under them afterwards.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:12037");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(cfg.from_override, "choosing a value is choosing it");
    assert!(cfg.url_from_override, "realign must leave this URL alone");
}

#[test]
fn overriding_the_chain_source_alone_leaves_the_url_realignable() {
    // Realign rewrites the URL and nothing else. A profile that only chose its
    // chain source has expressed no opinion about the port, so a stale global
    // loopback must still be fixable.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_override(&conn, "p1", "chain_source", "remote_node");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(cfg.from_override, "the profile did choose something");
    assert!(
        !cfg.url_from_override,
        "the URL still came from the global fallback"
    );
}

#[test]
fn a_per_profile_setting_outside_the_node_tuple_is_not_a_node_override() {
    // `profile_settings` is a general per-profile store. A key that says
    // nothing about which node this profile talks to must not make the node
    // config read as overridden — and so must not disable realign.
    let conn = db_with_profile();
    set_override(&conn, "p1", "some_unrelated_preference", "yes");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(!cfg.from_override);
    assert!(!cfg.url_from_override);
}

#[test]
fn an_empty_url_override_does_not_protect_the_url_from_realign() {
    // An empty override is "no meaningful choice" for resolution, so it must
    // be "no meaningful choice" for realign too, or the two disagree about
    // whose URL is in play.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_override(&conn, "p1", "node_rpc_url", "");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(!cfg.url_from_override);
}
