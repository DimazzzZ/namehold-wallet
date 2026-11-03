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

// ---------------------------------------------------------------------------
// The key the app generated for the hsd it starts (`managed_node_api_key.<net>`)
// ---------------------------------------------------------------------------

use crate::commands::local_node::{network_scoped_data_dir, write_pid_file, PID_FILE};
use crate::noncustodial::network::Network;
use crate::noncustodial::node_config::{api_key_for_started_node, new_managed_node_api_key};
use crate::noncustodial::rpc::managed_node_api_key_setting;

/// Point `hsd_prefix` at a new directory and record this test process as
/// `network`'s own node there: a live pid with its real start time. Returns
/// that network's data dir, held by the returned guard.
fn own_node_running(
    conn: &Connection,
    network: Network,
) -> (crate::tests::command_helpers::TempDir, std::path::PathBuf) {
    let base = crate::tests::command_helpers::TempDir::new();
    set_global(conn, "hsd_prefix", base.path().to_str().unwrap());
    let dir = std::path::PathBuf::from(network_scoped_data_dir(
        base.path().to_str().unwrap(),
        network,
    ));
    std::fs::create_dir_all(&dir).unwrap();
    write_pid_file(&dir, std::process::id()).unwrap();
    (base, dir)
}

/// With no key configured, a profile that reaches the address of its network's
/// running own node talks to it with that network's generated key, the app and
/// the daemon alike (both read it here).
#[test]
fn a_profile_on_its_running_own_nodes_address_reads_the_generated_key() {
    let conn = db_with_profile(); // regtest, no URL: http://127.0.0.1:14037
    let key = new_managed_node_api_key(&conn, Network::Regtest).unwrap();
    let _own = own_node_running(&conn, Network::Regtest);
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
    assert_eq!(cfg.node_rpc_api_key, key);
    let client = crate::noncustodial::rpc::NodeRpcClient::for_profile(&conn, "p1").unwrap();
    assert_eq!(client.api_key(), key);
    // `localhost` is the same address: the client pins it to 127.0.0.1.
    set_override(&conn, "p1", "node_rpc_url", "http://localhost:14037");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_api_key, key);
}

/// The generated key goes to that node's address and nowhere else, with the
/// node running: not to a remote node, another port, TLS or another loopback
/// address on that port, which hsd does not bind and another process can.
#[test]
fn the_generated_key_is_never_sent_to_another_address() {
    for url in [
        "https://node.example.com:14037",
        "http://127.0.0.1:24037",
        "http://192.168.1.5:14037",
        "http://[::1]:14037",
        "http://127.0.0.2:14037",
        "https://127.0.0.1:14037",
        "http://localhost.evil.example:14037",
    ] {
        let conn = db_with_profile();
        new_managed_node_api_key(&conn, Network::Regtest).unwrap();
        let _own = own_node_running(&conn, Network::Regtest);
        set_override(&conn, "p1", "node_rpc_url", url);
        let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
        assert_eq!(cfg.node_rpc_api_key, "", "{url}");
    }
}

/// A stored key is no reason to send it: with no pid record, whatever answers
/// on the port is not the app's node (a node being adopted, a process that took
/// the port first).
#[test]
fn the_generated_key_is_not_sent_when_the_app_has_no_node_running() {
    let conn = db_with_profile();
    new_managed_node_api_key(&conn, Network::Regtest).unwrap();
    let base = crate::tests::command_helpers::TempDir::new();
    set_global(&conn, "hsd_prefix", base.path().to_str().unwrap());
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
    assert_eq!(cfg.node_rpc_api_key, "");
}

/// A record whose start time is another's (a reused pid, or a node long gone)
/// names nothing, so it earns no key.
#[test]
fn the_generated_key_is_not_sent_on_a_stale_pid_record() {
    let conn = db_with_profile();
    new_managed_node_api_key(&conn, Network::Regtest).unwrap();
    let (_base, dir) = own_node_running(&conn, Network::Regtest);
    std::fs::write(dir.join(PID_FILE), format!("{} 1\n", std::process::id())).unwrap();
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_api_key, "");
}

/// One key per network: each network generates its own, and a URL on one
/// network's port never reads another's, whichever own node runs.
#[test]
fn each_network_has_its_own_generated_key() {
    let conn = db_with_profile();
    let keys: Vec<String> = Network::ALL
        .into_iter()
        .map(|n| new_managed_node_api_key(&conn, n).unwrap())
        .collect();
    let distinct: std::collections::HashSet<&String> = keys.iter().collect();
    assert_eq!(distinct.len(), keys.len());

    // The regtest node runs; a URL on mainnet's port has no running own node.
    let _own = own_node_running(&conn, Network::Regtest);
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:12037");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_api_key, "");
}

/// The mainnet node's key stays home even when a mainnet node runs and only
/// regtest has no key: regtest's address never reads mainnet's.
#[test]
fn a_network_without_a_key_does_not_read_anothers() {
    let conn = db_with_profile();
    new_managed_node_api_key(&conn, Network::Main).unwrap();
    let (_base, dir) = own_node_running(&conn, Network::Main);
    // Regtest's own record too: the record alone must not unlock another's key.
    let regtest = dir.join("regtest");
    std::fs::create_dir_all(&regtest).unwrap();
    write_pid_file(&regtest, std::process::id()).unwrap();
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_url, "http://127.0.0.1:14037");
    assert_eq!(cfg.node_rpc_api_key, "");
}

/// A configured key wins over the generated one: it is the key the node is
/// started with too.
#[test]
fn a_configured_key_wins_over_the_generated_one() {
    let conn = db_with_profile();
    new_managed_node_api_key(&conn, Network::Regtest).unwrap();
    let _own = own_node_running(&conn, Network::Regtest);
    set_global(&conn, "node_rpc_api_key", "configured");
    let cfg = effective_node_config_for_profile(&conn, "p1").unwrap();
    assert_eq!(cfg.node_rpc_api_key, "configured");
}

/// A key is 32 random bytes, hex, stored as the network's key, and each call makes
/// another: whatever heard an earlier start's key must not hold the next node's.
#[test]
fn every_managed_key_is_new_and_replaces_the_stored_one() {
    let conn = db_with_profile();
    let first = new_managed_node_api_key(&conn, Network::Regtest).unwrap();
    assert_eq!(first.len(), 64, "{first}");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()), "{first}");
    let second = new_managed_node_api_key(&conn, Network::Regtest).unwrap();
    assert_ne!(second, first);
    let stored = crate::db::queries::get_settings(&conn).unwrap()
        [managed_node_api_key_setting(Network::Regtest)]
    .clone();
    assert_eq!(stored, second);
}

/// A started node gets its own network's key, even when the profile's URL
/// points at another network's running own node and reads that one's key.
#[test]
fn a_started_node_never_shares_another_networks_generated_key() {
    let conn = db_with_profile();
    let simnet = new_managed_node_api_key(&conn, Network::Simnet).unwrap();
    let _own = own_node_running(&conn, Network::Simnet);
    set_override(&conn, "p1", "node_rpc_url", "http://127.0.0.1:15037");
    assert_eq!(
        effective_node_config_for_profile(&conn, "p1")
            .unwrap()
            .node_rpc_api_key,
        simnet
    );
    let started = api_key_for_started_node(&conn, "p1", Network::Regtest).unwrap();
    assert_ne!(started, simnet);
    assert_eq!(
        started,
        crate::db::queries::get_settings(&conn).unwrap()
            [managed_node_api_key_setting(Network::Regtest)]
    );
}

/// A configured key is the one a started node gets; none is generated.
#[test]
fn a_started_node_gets_the_configured_key() {
    let conn = db_with_profile();
    set_global(&conn, "node_rpc_api_key", "configured");
    assert_eq!(
        api_key_for_started_node(&conn, "p1", Network::Regtest).unwrap(),
        "configured"
    );
    assert!(!crate::db::queries::get_settings(&conn)
        .unwrap()
        .contains_key(managed_node_api_key_setting(Network::Regtest)));
}
