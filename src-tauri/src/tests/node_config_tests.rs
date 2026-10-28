//! Tests for per-profile node configuration resolution (ADR-001).
//!
//! Contract under test (`effective_node_config_for_profile`):
//!   1. Resolution order, per key: per-profile override -> global `settings`
//!      -> built-in default.
//!   2. A missing profile is [`AppError::NotFound`], never a silent default.
//!   3. An override sets `from_override = true`; a
//!      pure global/default resolution sets it `false`.
//!   4. Resolution is per-key: an override for one key falls through to global
//!      for the others.
//!   5. The profile's network is never touched by resolution.

use rusqlite::Connection;

use crate::db::queries::get_profile_settings;
use crate::error::AppError;
use crate::noncustodial::node_config::effective_node_config_for_profile;
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
fn a_local_node_profile_with_nothing_set_reaches_its_networks_default_port() {
    let conn = db_with_profile(); // p1 is regtest
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
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
    // an empty string is not a meaningful endpoint choice. (A remote node: a
    // local node reaches its network's port whatever the global URL says.)
    let conn = db_with_profile();
    set_global(&conn, "chain_source", "remote_node");
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
fn for_profile_of_a_local_node_profile_talks_to_its_networks_port() {
    let conn = db_with_profile(); // p1 is regtest
                                  // The global URL points at the mainnet node started last; the key is
                                  // still the global one.
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_global(&conn, "node_rpc_api_key", "global-key");
    let client = NodeRpcClient::for_profile(&conn, "p1").unwrap();
    assert_eq!(client.node_url(), "http://127.0.0.1:14037");
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

// --- What counts as the profile's own URL (ADR-001) ---

#[test]
fn an_override_equal_to_global_is_still_the_users_choice() {
    // The flag used to mean "differs from global", so pinning a profile to the
    // value global happens to hold today read as no override at all. A URL
    // the profile chose is kept even when it is another network's default.
    let conn = db_with_profile(); // regtest
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:12037");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(cfg.from_override, "choosing a value is choosing it");
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:12037");
}

#[test]
fn overriding_the_chain_source_alone_still_reaches_the_networks_port() {
    // A profile that only chose its chain source has expressed no opinion
    // about the URL, so a default global URL still gives way to its port.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_override(&conn, "p1", "chain_source", "local_node");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(cfg.from_override, "the profile did choose something");
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
}

#[test]
fn a_per_profile_setting_outside_the_node_tuple_is_not_a_node_override() {
    // `profile_settings` is a general per-profile store. A key that says
    // nothing about which node this profile talks to must not make the node
    // config read as overridden.
    let conn = db_with_profile();
    set_override(&conn, "p1", "some_unrelated_preference", "yes");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert!(!cfg.from_override);
}

#[test]
fn an_empty_url_override_is_no_choice_of_url() {
    // An empty override is "no meaningful choice": the default global URL
    // still gives way to the network's port.
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_url", "http://127.0.0.1:12037");
    set_override(&conn, "p1", "node_rpc_url", "");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
}

// --- Local node per network (spec 2026-10-24 R1) ---------------------------

/// A `local_node` profile without a URL override reaches its own network's
/// node when the global URL is only a default: empty, or a loopback URL on some
/// network's default RPC port, which is what the network started last left
/// there. Another network's wallet must not follow it.
#[test]
fn a_local_node_profile_reaches_its_networks_port_over_a_default_global_url() {
    let conn = db_with_profile(); // p1 is regtest
    for global in [
        "",
        "http://127.0.0.1:12037",
        "http://localhost:13037/",
        "http://[::1]:15037",
        "https://127.0.0.1:12037",
    ] {
        set_global(&conn, "node_rpc_url", global);
        let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
        assert_eq!(
            cfg.node_rpc_url, "http://127.0.0.1:14037",
            "global {global:?}"
        );
    }
}

/// A global URL that is not a default (another host, or a port no network
/// uses by default) is the user's choice, and a `local_node` profile without
/// an override keeps it: a node on another machine of the LAN, or a node on a
/// custom port, is not silently replaced by 127.0.0.1.
#[test]
fn a_local_node_profile_keeps_a_global_url_that_is_not_a_default() {
    let conn = db_with_profile();
    for global in [
        "http://127.0.0.1:9999",
        "http://192.168.1.5:12037",
        "http://node.example:14037",
    ] {
        set_global(&conn, "node_rpc_url", global);
        let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
        assert_eq!(cfg.node_rpc_url, global);
    }
}

/// Spec 2026-10-24 R1 holds for every chain source: a loopback URL on
/// another network's default port is what the realign of older builds left
/// there, whatever the chain source was, so a remote-node or explorer
/// profile without its own URL reaches its own network's port over it too.
/// Before 2026-10-28 this held for local-node profiles only, and a mainnet
/// wallet over `remote_node` and a leftover `127.0.0.1:14037` could not reach
/// the mainnet node the previous build had started: `start_hsd` spawned a
/// second hsd over its data directory instead.
#[test]
fn a_remote_node_or_explorer_profile_over_another_networks_default_reaches_its_own_port() {
    let conn = db_with_profile(); // p1 is regtest
    for source in ["remote_node", "explorer"] {
        set_global(&conn, "chain_source", source);
        for global in [
            "",
            "http://127.0.0.1:12037",
            "http://localhost:13037/",
            "http://[::1]:15037",
        ] {
            set_global(&conn, "node_rpc_url", global);
            let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
            assert_eq!(
                cfg.node_rpc_url, "http://127.0.0.1:14037",
                "{source} over {global:?}"
            );
        }
    }
}

/// A remote-node profile's global URL that is not a default is its choice of
/// node and is kept as it is: another host, a custom port, or its own
/// network's default port.
#[test]
fn a_remote_node_profile_keeps_a_global_url_that_is_not_a_default() {
    let conn = db_with_profile();
    set_global(&conn, "chain_source", "remote_node");
    for global in [
        "http://127.0.0.1:9999",
        "http://10.0.0.5:12037",
        "http://node.lan:12037",
        "http://127.0.0.1:14037",
    ] {
        set_global(&conn, "node_rpc_url", global);
        let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
        assert_eq!(cfg.node_rpc_url, global, "{global}");
    }
}

/// The profile's own URL wins over the network's port, as ADR-001 says.
#[test]
fn a_profile_url_override_still_wins() {
    let conn = db_with_profile();
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:12037");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:12037");
}

/// Spec 2026-10-24 R10: a local node's full or SPV mode is its network's, so
/// a regtest wallet is SPV only when regtest's node is, whatever mainnet's is.
#[test]
fn a_profiles_chain_source_follows_its_networks_mode() {
    let conn = db_with_profile(); // regtest, local_node
    set_global(&conn, "node_mode.main", "spv");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.chain_source, ChainSource::LocalNode, "mainnet's mode");

    set_global(&conn, "node_mode.regtest", "spv");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.chain_source, ChainSource::SpvNode);

    // The network's own mode wins over the app-wide one.
    set_global(&conn, "node_mode", "spv");
    set_global(&conn, "node_mode.regtest", "full");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.chain_source, ChainSource::LocalNode);
}
