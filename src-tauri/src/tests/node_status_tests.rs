//! `node_status` must reflect the REAL node connection (RPC answers), not just
//! whether we spawned a child. With no node reachable, `connected` is false and
//! `process_alive` is false — and it never falsely reports a connection.

use crate::tests::command_helpers::set_profile_override;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::node::node_status;
use crate::db;
use crate::AppState;

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_children: std::sync::Mutex::new(std::collections::HashMap::new()),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// In-memory DB with the node RPC pointed at an unroutable address, so the probe
/// fails deterministically (no flakiness from a real node on 12037).
fn seeded_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    conn
}

#[tokio::test]
async fn node_status_reports_disconnected_when_no_node() {
    let app = app_with(seeded_conn());
    let v = node_status(app.state()).await.expect("node_status ok");

    // The authoritative signal: RPC did not answer → not connected.
    assert_eq!(v["connected"], serde_json::json!(false));
    // We never spawned a child in this test → not alive (and not a false green).
    assert_eq!(v["process_alive"], serde_json::json!(false));
    assert_eq!(v["height"], serde_json::Value::Null);

    // Shape the UI relies on is present.
    assert!(v["binary"].is_string());
    assert!(v["data_dir"].is_string());
    // No wallet in this DB: no network to name (spec 2026-10-24 R8).
    assert!(v["network"].is_null());
    // Sync-progress fields are always present (null when not connected).
    assert!(v.get("verification_progress").is_some());
    assert_eq!(v["verification_progress"], serde_json::Value::Null);
    assert!(v.get("headers").is_some());
    assert_eq!(v["headers"], serde_json::Value::Null);

    // read_source is always present and defaults to "explorer" when not connected.
    assert_eq!(v["read_source"], serde_json::json!("explorer"));
}

// --- is_node_ready_for_local_reads -------------------------------------------

use crate::commands::node_readiness::is_node_ready_for_local_reads;

#[tokio::test]
async fn local_reads_not_ready_when_not_connected() {
    let app = app_with(seeded_conn());
    let state = app.state::<AppState>();

    // Node is not connected → should not use local reads.
    assert!(!is_node_ready_for_local_reads(&state).await);
}

// --- node_ready_from_settings (the settings-based gate used by the background
//     sync thread, which has no State<AppState>) --------------------------------

use crate::commands::node_readiness::node_ready_from_settings;

/// Build a settings map pointing the node RPC at a mockito server URL.
fn settings_for_url(url: &str) -> std::collections::HashMap<String, String> {
    let mut s = std::collections::HashMap::new();
    s.insert("node_rpc_url".to_string(), url.to_string());
    s.insert("node_rpc_api_key".to_string(), "x".to_string());
    s
}

#[tokio::test]
async fn node_ready_from_settings_true_when_synced() {
    let mut server = mockito::Server::new_async().await;
    // getblockchaininfo → fully synced (progress ≥ 0.9999).
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 1000, "headers": 1000, "verification_progress": 1.0 },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    assert!(node_ready_from_settings(&settings_for_url(&server.url()), None).await);
}

#[tokio::test]
async fn node_ready_from_settings_false_while_syncing() {
    let mut server = mockito::Server::new_async().await;
    // Node answers but is far behind (low verification progress) → NOT ready,
    // so the explorer fallback must stay active.
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 80, "headers": 1000, "verification_progress": 0.08 },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    assert!(!node_ready_from_settings(&settings_for_url(&server.url()), None).await);
}

/// The gate the background sync, chain scanner and watched-name daemon all
/// use: a fully synced node that reports a different chain than the profile is
/// NOT authoritative. Without this, a regtest node's height would be written
/// into a mainnet profile's `sync_cursors` — the same cursor coin selection
/// reads to decide coinbase maturity.
#[tokio::test]
async fn node_ready_from_settings_false_when_node_is_on_another_chain() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": {
                    "chain": "regtest",
                    "blocks": 1000, "headers": 1000, "verification_progress": 1.0
                },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let settings = settings_for_url(&server.url());
    assert!(
        !node_ready_from_settings(&settings, Some("mainnet")).await,
        "a regtest node must not be authoritative for a mainnet profile"
    );
    assert!(
        node_ready_from_settings(&settings, Some("regtest")).await,
        "the same node IS authoritative for a regtest profile"
    );
}

#[tokio::test]
async fn node_ready_from_settings_false_when_unreachable() {
    // Unroutable node → probe fails → not ready.
    assert!(!node_ready_from_settings(&settings_for_url("http://127.0.0.1:1"), None).await);
}

// --- api-key resolution (talk to a node configured via hsd.conf) -------------

use crate::noncustodial::rpc::resolve_node_api_key;
use std::collections::HashMap;

#[test]
fn api_key_falls_back_to_hsd_conf_when_setting_empty() {
    let dir = std::env::temp_dir().join("namehold_apikey_conf_test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hsd.conf"), "api-key: deadbeef\ntx-index: true\n").unwrap();

    let mut s = HashMap::new();
    s.insert("hsd_prefix".to_string(), dir.to_string_lossy().to_string());
    s.insert("node_rpc_api_key".to_string(), String::new());

    assert_eq!(resolve_node_api_key(&s), "deadbeef");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn explicit_api_key_wins_over_hsd_conf() {
    let dir = std::env::temp_dir().join("namehold_apikey_explicit_test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hsd.conf"), "api-key: fromconf\n").unwrap();

    let mut s = HashMap::new();
    s.insert("hsd_prefix".to_string(), dir.to_string_lossy().to_string());
    s.insert("node_rpc_api_key".to_string(), "explicitkey".to_string());

    assert_eq!(resolve_node_api_key(&s), "explicitkey");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- hsd binary discovery (the Start-hsd button depends on this) -------------

use crate::commands::node::pick_hsd_path;

#[test]
fn pick_hsd_path_honors_explicit_override_verbatim() {
    // An explicit hsd_path is trusted as-is (even if it doesn't exist yet), and
    // wins over candidates.
    let candidates = vec!["/opt/homebrew/bin/hsd".to_string()];
    assert_eq!(
        pick_hsd_path(Some("/custom/hsd"), &candidates).as_deref(),
        Some("/custom/hsd")
    );
    // Blank/whitespace override is ignored (falls through to candidates).
    assert_eq!(pick_hsd_path(Some("   "), &[]), None);
}

#[test]
fn pick_hsd_path_finds_the_first_existing_candidate() {
    // A real temp file stands in for an installed hsd on a candidate path.
    let dir = std::env::temp_dir().join("namehold_hsd_discovery_test");
    std::fs::create_dir_all(&dir).unwrap();
    let real = dir.join("hsd");
    std::fs::write(&real, b"#!/bin/sh\n").unwrap();

    let candidates = vec![
        "/no/such/path/hsd".to_string(),
        real.to_string_lossy().to_string(),
    ];
    assert_eq!(
        pick_hsd_path(None, &candidates),
        Some(real.to_string_lossy().to_string())
    );

    // Nothing exists and no override → None (caller falls back to which/PATH).
    assert_eq!(
        pick_hsd_path(None, &["/no/such/path/hsd".to_string()]),
        None
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- start-failure surfacing (no more silent "Starting…") --------------------

use crate::commands::node::{chain_paths_for_network, node_start_error};
use crate::noncustodial::network::Network;

#[test]
fn node_start_error_flags_the_index_mismatch_with_guidance() {
    let dir = std::env::temp_dir().join("namehold_node_err_index");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("namehold-hsd.log"),
        "[info] (chaindb) Opening ChainDB...\nError: Cannot retroactively enable TX indexing.\n    at ChainDB.verifyFlags\n",
    )
    .unwrap();

    let (msg, mismatch) =
        node_start_error(&dir.to_string_lossy()).expect("should surface an error");
    assert!(
        mismatch,
        "index mismatch must be flagged so the UI offers a re-sync"
    );
    assert!(msg.contains("Re-sync"), "actionable guidance: {msg}");
    assert!(
        msg.contains("Cannot retroactively enable"),
        "includes the log tail: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn node_start_error_is_none_without_a_failing_log() {
    let dir = std::env::temp_dir().join("namehold_node_err_none");
    std::fs::create_dir_all(&dir).unwrap();
    // No log at all → None.
    assert!(node_start_error(&dir.to_string_lossy()).is_none());
    // A log with no error markers → None (don't cry wolf on a clean start).
    std::fs::write(
        dir.join("namehold-hsd.log"),
        "[info] (chain) Chain is loading.\n",
    )
    .unwrap();
    assert!(node_start_error(&dir.to_string_lossy()).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn node_start_error_ignores_benign_peer_socket_errors_while_syncing() {
    // Regression: hsd routinely logs benign peer-connection failures like
    // "(net) Error: Socket Error: ECONNREFUSED" while it is still syncing and
    // its RPC hasn't come up yet. These are NOT startup failures. Reporting
    // them as "hsd failed to start" (just because the log contains the
    // substring "Error") shows a fake failure over a node that is healthy and
    // mid-rescan. Captured verbatim from a real regtest run.
    let dir = std::env::temp_dir().join("namehold_node_err_benign_net");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("namehold-hsd.log"),
        "[info] (net) Connected to 5.161.64.49:12038.\n\
         [debug] (net) Error: Socket Error: ECONNREFUSED (173.255.209.126:12038)\n\
         [debug] (net) Error: Socket Error: ECONNREFUSED (74.207.247.120:12038)\n\
         [debug] (wallet) Adding block: 17222.\n\
         [info] (chain) Block 000000000000023319cc63c828c18ef22139cc4e327452632f620a2fb5944c63 (17222) added to chain (size=5842 txs=11 time=3.004958).\n",
    )
    .unwrap();
    assert!(
        node_start_error(&dir.to_string_lossy()).is_none(),
        "benign (net) socket errors during sync must not surface as a start failure"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- start_hsd refuses hsd below the minimum supported version (S3) ----------

use crate::commands::node::start_hsd;

#[cfg(unix)]
#[tokio::test]
async fn start_hsd_refuses_hsd_below_minimum_version() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join("namehold_start_hsd_min_version_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // A fake "hsd" that answers --version with an old release, so the
    // minimum-version gate in `start_hsd` trips before any real process is
    // spawned. This exercises the actual refusal path (not just the pure
    // parse/compare helpers) without depending on a real hsd binary.
    let fake_hsd = dir.join("hsd");
    std::fs::write(&fake_hsd, "#!/bin/sh\necho \"7.9.9\"\n").unwrap();
    let mut perms = std::fs::metadata(&fake_hsd).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_hsd, perms).unwrap();

    let data_dir = dir.join("data");

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "hsd_path", &fake_hsd.to_string_lossy()).unwrap();
    db::queries::set_setting(&conn, "hsd_prefix", &data_dir.to_string_lossy()).unwrap();
    // Unroutable RPC so the "adopt an already-running node" probe fails
    // deterministically and falls through to the version-gated spawn path.
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    // `start_hsd` refuses without an active profile (it will not guess which
    // network to launch), so seed one — this test is about the version gate.
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "Primary",
        "watch_only_xpub",
        "mainnet",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();

    let app = app_with(conn);
    let result = start_hsd(app.state()).await;

    let err = result.expect_err("hsd 7.9.9 must be refused");
    let msg = err.to_string();
    assert!(msg.contains("7.9.9"), "names the found version: {msg}");
    assert!(msg.contains("8.0.0"), "names the minimum version: {msg}");
    assert!(
        msg.to_lowercase().contains("upgrade"),
        "gives actionable guidance: {msg}"
    );

    // The gate must trip before `cmd.spawn()` — no child left behind.
    assert!(app
        .state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chain_paths_are_network_scoped() {
    // Mainnet keeps chain artifacts at the prefix root; other networks under a subdir.
    let main = chain_paths_for_network("/data", Network::Main);
    assert!(main.iter().any(|p| p.ends_with("blocks")));
    assert!(main.iter().any(|p| p.ends_with("chain")));
    assert!(main.iter().any(|p| p.ends_with("tree")));
    assert_eq!(
        chain_paths_for_network("/data", Network::Regtest),
        vec![std::path::PathBuf::from("/data/regtest")]
    );
    assert_eq!(
        chain_paths_for_network("/data", Network::Testnet),
        vec![std::path::PathBuf::from("/data/testnet")]
    );
}

// ---------------------------------------------------------------------------
// node_rpc_alive: the tray reads this flag to decide "Running" vs "Stopped".
// It must reflect the REAL RPC connection — not whether we spawned a child —
// so an ADOPTED node (RPC up, no child) still shows as running. This is
// the exact bug where the tray showed "Start Node" for a node that was up.
// ---------------------------------------------------------------------------

/// Adopted node: RPC answers `getblockchaininfo` but we never spawned a child
/// (no child in `hsd_children`). After a probe, `node_rpc_alive` must be true — this
/// is what makes the tray show "Running"/"Stop Node" for an adopted node.
#[tokio::test]
async fn node_rpc_alive_true_for_adopted_node() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":1000,"headers":1000,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();

    // Precondition: no child handle — this simulates the adoption path where
    // start_hsd found a running node and returned without storing a child.
    assert!(
        state.hsd_children.lock().unwrap().is_empty(),
        "adopted node must have no child handle"
    );

    // node_status probes RPC and updates the flag.
    let v = node_status(app.state()).await.expect("node_status ok");
    assert_eq!(
        v["connected"],
        serde_json::json!(true),
        "RPC answered → connected"
    );
    assert!(
        state
            .node_rpc_alive
            .load(std::sync::atomic::Ordering::Relaxed),
        "node_rpc_alive must be true when RPC answers, even with no child handle"
    );
}

/// No reachable node: `node_rpc_alive` must be false so the tray shows
/// "Stopped"/"Start Node".
#[tokio::test]
async fn node_rpc_alive_false_when_no_node() {
    let app = app_with(seeded_conn());
    let state = app.state::<AppState>();

    let _ = node_status(app.state()).await.expect("node_status ok");
    assert!(
        !state
            .node_rpc_alive
            .load(std::sync::atomic::Ordering::Relaxed),
        "node_rpc_alive must be false when no node is reachable"
    );
}

/// `probe_and_update` (used by the backend probe loop) sets the flag directly
/// without going through the full `node_status` command.
#[tokio::test]
async fn probe_and_update_sets_flag_true_when_node_answers() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":42,"headers":42,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();

    let alive = crate::commands::node::probe_and_update(&state).await;
    assert!(alive, "probe_and_update returns true when RPC answers");
    assert!(
        state
            .node_rpc_alive
            .load(std::sync::atomic::Ordering::Relaxed),
        "probe_and_update stores true on the flag"
    );
}

// --- node_tip_height_if_synced_from_settings_with_network ---------------------

use crate::commands::node_readiness::node_tip_height_if_synced_from_settings_with_network;

#[tokio::test]
async fn synced_with_matching_network_returns_height() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 500, "headers": 500, "verificationprogress": 1.0, "chain": "main" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let h = node_tip_height_if_synced_from_settings_with_network(
        &settings_for_url(&server.url()),
        Some("mainnet"),
    )
    .await;
    assert_eq!(h, Some(500));
}

#[tokio::test]
async fn synced_with_mismatched_network_returns_none() {
    let mut server = mockito::Server::new_async().await;
    // Node reports "regtest" but wallet profile is "mainnet".
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 42, "headers": 42, "verificationprogress": 1.0, "chain": "regtest" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let h = node_tip_height_if_synced_from_settings_with_network(
        &settings_for_url(&server.url()),
        Some("mainnet"),
    )
    .await;
    assert_eq!(
        h, None,
        "regtest node must NOT be authoritative for mainnet wallet"
    );
}

#[tokio::test]
async fn synced_with_no_expected_network_skips_check() {
    let mut server = mockito::Server::new_async().await;
    // Node is regtest, no expected network → gate passes (backward-compat).
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 10, "headers": 10, "verificationprogress": 1.0, "chain": "regtest" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let h = node_tip_height_if_synced_from_settings_with_network(
        &settings_for_url(&server.url()),
        None,
    )
    .await;
    assert_eq!(h, Some(10), "no expected network → network check skipped");
}

#[tokio::test]
async fn synced_with_no_chain_in_response_skips_check() {
    let mut server = mockito::Server::new_async().await;
    // Older hsd that doesn't report `chain` — gate passes conservatively.
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 100, "headers": 100, "verificationprogress": 1.0 },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let h = node_tip_height_if_synced_from_settings_with_network(
        &settings_for_url(&server.url()),
        Some("mainnet"),
    )
    .await;
    assert_eq!(
        h,
        Some(100),
        "missing chain in response → conservatively allow"
    );
}

// ===========================================================================
// node_status against a *connected* node — the read_source / node_synced
// decision tree (previously only the disconnected path was covered). The RPC
// is mocked with mockito so no live hsd is needed.
// ===========================================================================

// ===========================================================================
// Per-profile node config resolution for readiness probe (ADR-001)
// ===========================================================================

use crate::commands::node_readiness::node_tip_height_if_synced_from_profile_with_network;

/// Helper: create a temp file-backed DB (in-memory won't work because the
/// async probe re-opens the connection from a path — see the Send bound
/// on tauri's async runtime).
fn temp_db_conn() -> (String, rusqlite::Connection) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "namehold_probe_test_{}_{}.db",
        std::process::id(),
        n
    ));
    // Ensure a clean slate: remove any leftover file from a prior run.
    let _ = std::fs::remove_file(&path);
    let path_str = path.to_str().unwrap();
    let conn = crate::commands::sync::open_conn(path_str).unwrap();
    (path_str.to_string(), conn)
}

/// Helper: create a test profile with minimal required fields.
fn create_test_profile(conn: &rusqlite::Connection, profile_id: &str, network: &str) {
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub, account_index, receive_depth, change_depth, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, datetime('now'))",
        rusqlite::params![profile_id, format!("Test {}", profile_id), "watch_only_xpub", network, "xpub_test"],
    )
    .unwrap();
}

#[tokio::test]
async fn probe_uses_profile_override_over_global() {
    // Profile W1 has a per-profile node_rpc_url override pointing to a working server.
    // Global settings point to an unreachable URL.
    // The probe should use the override and succeed.
    let mut server_override = mockito::Server::new_async().await;
    let _m = server_override
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 500, "headers": 500, "verificationprogress": 1.0, "chain": "main" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    // Add a profile and set its override to the working server.
    create_test_profile(&conn, "W1", "mainnet");
    set_profile_override(&conn, "W1", "node_rpc_url", &server_override.url());
    drop(conn);

    let h =
        node_tip_height_if_synced_from_profile_with_network(&db_path, "W1", Some("mainnet")).await;
    assert_eq!(
        h,
        Some(500),
        "should use profile override, not unreachable global URL"
    );
}

#[tokio::test]
async fn probe_falls_back_to_global_when_no_override() {
    // Profile W1 has no per-profile override.
    // Global settings point to a working server.
    // The probe should fall back to global and succeed.
    let mut server_global = mockito::Server::new_async().await;
    let _m = server_global
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 200, "headers": 200, "verificationprogress": 1.0, "chain": "regtest" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    db::queries::set_setting(&conn, "node_rpc_url", &server_global.url()).unwrap();
    // Add a profile with no override.
    create_test_profile(&conn, "W1", "regtest");
    drop(conn);

    let h =
        node_tip_height_if_synced_from_profile_with_network(&db_path, "W1", Some("regtest")).await;
    assert_eq!(h, Some(200), "should fall back to global settings");
}

#[tokio::test]
async fn probe_uses_builtin_default_when_no_override_or_global() {
    // Profile W1 has no per-profile override and no global settings.
    // The probe should use the built-in default (localhost:12037).
    // This will fail to connect, returning None.
    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    // Add a profile with no override and no global settings.
    create_test_profile(&conn, "W1", "regtest");
    drop(conn);

    let h =
        node_tip_height_if_synced_from_profile_with_network(&db_path, "W1", Some("regtest")).await;
    assert_eq!(
        h, None,
        "unreachable built-in default should return None, not panic"
    );
}

#[tokio::test]
async fn probe_respects_network_mismatch_with_profile_override() {
    // Profile W1 expects "mainnet" but the override points to a "regtest" node.
    // The probe should reject the node (return None) due to network mismatch.
    let mut server_override = mockito::Server::new_async().await;
    let _m = server_override
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 50, "headers": 50, "verificationprogress": 1.0, "chain": "regtest" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    // Add a profile expecting mainnet.
    create_test_profile(&conn, "W1", "mainnet");
    // Override points to a regtest node.
    set_profile_override(&conn, "W1", "node_rpc_url", &server_override.url());
    drop(conn);

    let h =
        node_tip_height_if_synced_from_profile_with_network(&db_path, "W1", Some("mainnet")).await;
    assert_eq!(
        h, None,
        "network mismatch (mainnet profile vs regtest node) must be rejected"
    );
}

// ===========================================================================
// Per-profile readiness gate (boolean wrapper)
// ===========================================================================

use crate::commands::node_readiness::node_ready_from_profile;

#[tokio::test]
async fn node_ready_from_profile_returns_true_when_synced_and_network_matches() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 500, "headers": 500, "verificationprogress": 1.0, "chain": "main" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    create_test_profile(&conn, "W1", "mainnet");
    drop(conn);

    let ready = node_ready_from_profile(&db_path, "W1", Some("mainnet")).await;
    assert!(
        ready,
        "node should be ready when synced and network matches"
    );
}

#[tokio::test]
async fn node_ready_from_profile_returns_false_when_network_mismatches() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "result": { "blocks": 50, "headers": 50, "verificationprogress": 1.0, "chain": "regtest" },
                "error": null, "id": null
            })
            .to_string(),
        )
        .create_async()
        .await;

    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    create_test_profile(&conn, "W1", "mainnet");
    drop(conn);

    let ready = node_ready_from_profile(&db_path, "W1", Some("mainnet")).await;
    assert!(!ready, "node should not be ready when network mismatches");
}

#[tokio::test]
async fn node_ready_from_profile_returns_false_when_node_unreachable() {
    let (_tf, conn) = temp_db_conn();
    let db_path = _tf;
    // No global settings, no override — will use built-in default which is unreachable.
    create_test_profile(&conn, "W1", "regtest");
    drop(conn);

    let ready = node_ready_from_profile(&db_path, "W1", Some("regtest")).await;
    assert!(!ready, "node should not be ready when unreachable");
}

/// A blank in-memory DB (no node_rpc_url yet — the caller sets it to the
/// mockito URL), so we can point the probe at a controllable server.
fn blank_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

/// Synced full node (verification_progress ≥ 0.9999) → read_source = "local".
#[tokio::test]
async fn node_status_synced_full_node_uses_local_read_source() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":1000,"headers":1000,"verificationprogress":0.9999},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["read_source"], serde_json::json!("local"));
    assert_eq!(v["height"], serde_json::json!(1000));
    assert_eq!(v["node_mode"], serde_json::json!("full"));
}

/// Partially synced (verification_progress below the 0.9999 gate) → the node
/// answers but reads still fall back to the explorer.
#[tokio::test]
async fn node_status_partial_sync_uses_explorer_read_source() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":500,"headers":1000,"verificationprogress":0.5},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["read_source"], serde_json::json!("explorer"));
    assert_eq!(v["verification_progress"], serde_json::json!(0.5));
}

/// verification_progress absent → falls back to `height >= headers` (synced).
#[tokio::test]
async fn node_status_no_progress_falls_back_to_headers_synced() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":1000,"headers":1000},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["read_source"], serde_json::json!("local"));
    assert_eq!(v["verification_progress"], serde_json::Value::Null);
}

/// verification_progress absent AND height < headers → still catching up, so
/// reads stay on the explorer.
#[tokio::test]
async fn node_status_height_below_headers_uses_explorer() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":100,"headers":1000},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["read_source"], serde_json::json!("explorer"));
}

/// Neither progress nor headers reported (regtest single miner) → assume synced.
#[tokio::test]
async fn node_status_no_metadata_assumes_synced() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":42},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["read_source"], serde_json::json!("local"));
    assert_eq!(v["height"], serde_json::json!(42));
}

/// SPV mode: even a fully-synced node reads via the explorer (no address index),
/// and node_synced is true purely because RPC answered.
#[tokio::test]
async fn node_status_spv_mode_always_explorer() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":1000,"headers":1000,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    db::queries::set_setting(&conn, "node_mode", "spv").unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["read_source"], serde_json::json!("explorer"));
    assert_eq!(v["node_mode"], serde_json::json!("spv"));
}

/// probe_and_update clears node_rpc_alive when RPC is unreachable (the false
/// branch of the probe, complementing the true-branch test above).
#[tokio::test]
async fn probe_and_update_clears_flag_when_unreachable() {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();
    // Pretend it was alive; the probe must flip it false.
    state
        .node_rpc_alive
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let alive = crate::commands::node::probe_and_update(&state).await;
    assert!(!alive);
    assert!(!state
        .node_rpc_alive
        .load(std::sync::atomic::Ordering::Relaxed));
}

// ===========================================================================
// stop_hsd: the mockable half (RPC stop() + node_rpc_alive flip + audit log).
// The child.kill()/child.wait() path is documented as an IO shell in node.rs
// and exercised only by integration tests with a real hsd.
// ===========================================================================

use crate::commands::node::stop_hsd;

/// A regtest wallet, so stop_hsd has a network to act on, with its hsd data
/// under a temporary prefix of this process: a test must never read, stop or
/// change a node of the developer's real `~/.hsd`. Tests using it carry
/// `#[serial(hsd_home)]`, the key of every test that resolves hsd data dirs.
fn conn_with_active_regtest(node_url: &str) -> rusqlite::Connection {
    let conn = blank_conn();
    let prefix =
        std::env::temp_dir().join(format!("namehold_status_regtest_{}", std::process::id()));
    std::fs::create_dir_all(prefix.join("regtest")).unwrap();
    db::queries::set_setting(&conn, "hsd_prefix", prefix.to_str().unwrap()).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "P",
        "watch_only_xpub",
        "regtest",
        "xpub",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();
    crate::tests::command_helpers::set_profile_override(&conn, "p1", "node_rpc_url", node_url);
    conn
}

/// stop_hsd with no node of the network reachable still succeeds, flips
/// node_rpc_alive → false, and records an audit log row.
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn stop_hsd_soft_succeeds_when_no_node_reachable() {
    let conn = conn_with_active_regtest("http://127.0.0.1:1");
    let app = app_with(conn);
    let state = app.state::<AppState>();
    // Pretend it was alive; stop_hsd must flip it false regardless of RPC outcome.
    state
        .node_rpc_alive
        .store(true, std::sync::atomic::Ordering::Relaxed);

    stop_hsd(app.state())
        .await
        .expect("stop_hsd is best-effort");

    assert!(
        !state
            .node_rpc_alive
            .load(std::sync::atomic::Ordering::Relaxed),
        "node_rpc_alive must be cleared eagerly for the tray/UI to flip"
    );

    // The audit log row is written.
    let db = state.db.lock().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'stop_hsd'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "one stop_hsd audit row per stop");
}

/// Spec 2026-10-24 R7: a node that answers on the network's URL but that
/// Namehold did not start (no child of this session, no pid file) is never
/// stopped: Stop refuses with the reason and sends the node no `stop`.
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn stopping_an_outside_node_sends_nothing() {
    let mut server = mockito::Server::new_async().await;
    let _info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"chain":"regtest","blocks":5,"headers":5,"verificationprogress":1.0},"error":null,"id":1}"#)
        .create_async()
        .await;
    let stop = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex(r#""method":"stop""#.into()))
        .expect(0)
        .create_async()
        .await;
    let app = app_with(conn_with_active_regtest(&server.url()));

    let err = stop_hsd(app.state())
        .await
        .expect_err("an outside node is not stopped");

    assert_eq!(
        err.to_string(),
        format!(
            "Invalid input: {}",
            crate::commands::local_node::OUTSIDE_NODE_NOT_STOPPED
        )
    );
    stop.assert_async().await;
}

// ===========================================================================
// start_hsd: "adopt a running node" path — pure RPC, no child spawn. When the
// probe answers before we get to Command::spawn(), start_hsd returns the
// adopted-node shape. This exercises lines 409-424 without needing a real hsd.
// ===========================================================================

#[tokio::test]
async fn start_hsd_adopts_already_running_node() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":777,"headers":777,"verificationprogress":1.0,"chain":"regtest"},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    // A node is adopted *for* the active profile, so a profile must exist and
    // its network must match the chain the node reports. Seed a regtest profile
    // and let the mock report chain "regtest".
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "test",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();

    let v = start_hsd(app.state()).await.expect("adoption path");
    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["height"], serde_json::json!(777));
    // No child was spawned — the adoption path returns immediately.
    assert!(state.hsd_children.lock().unwrap().is_empty());
    // The alive flag was set as a side-effect.
    assert!(state
        .node_rpc_alive
        .load(std::sync::atomic::Ordering::Relaxed));
}

/// Adopt must REFUSE a running node whose chain disagrees with the active
/// profile's network — the guard that stops a regtest profile from adopting a
/// mainnet node answering on the same port. Regression cover for the bug where
/// a mainnet chain ended up driving a regtest wallet.
#[tokio::test]
async fn start_hsd_refuses_to_adopt_node_on_wrong_chain() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":2798,"headers":2798,"verificationprogress":1.0,"chain":"main"},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    // Active profile is regtest, but the node reports the main chain.
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "test",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();

    let err = start_hsd(app.state())
        .await
        .expect_err("must refuse mismatch");
    let msg = err.to_string();
    assert!(
        msg.contains("mismatch"),
        "expected mismatch error, got: {msg}"
    );
    // No child spawned, and we did not mark the (wrong) node alive.
    assert!(state.hsd_children.lock().unwrap().is_empty());
    assert!(!state
        .node_rpc_alive
        .load(std::sync::atomic::Ordering::Relaxed));
}

// ===========================================================================
// active_profile_network via node_status: with a real wallet profile seeded,
// the `Ok(Some(p))` arm of get_wallet_profile is exercised and the returned
// network flows into the JSON payload.
// ===========================================================================

#[tokio::test]
async fn node_status_reflects_seeded_profile_network_regtest() {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    // Seed a regtest profile and mark it active. This drives the
    // `active_profile_network` branch where get_wallet_profile returns
    // Some(p) and network_from_profile succeeds.
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "test",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();

    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");
    assert_eq!(v["network"], serde_json::json!("regtest"));
}

#[tokio::test]
async fn node_status_reflects_seeded_profile_network_testnet() {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "test",
        "watch_only_xpub",
        "testnet",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();

    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");
    assert_eq!(v["network"], serde_json::json!("testnet"));
}

// ===========================================================================
// resolve_data_dir HOME fallback: when hsd_prefix is unset, the code uses
// $HOME/.hsd (or "./.hsd" if HOME is also missing). Cover the HOME path.
// ===========================================================================

#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn node_status_data_dir_defaults_to_home_dot_hsd_when_prefix_unset() {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    // Do NOT set hsd_prefix — resolve_data_dir must fall back to $HOME/.hsd.
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");
    let data_dir = v["data_dir"].as_str().unwrap();
    // HOME is always set in the test env; the fallback path should end in ".hsd".
    assert!(
        data_dir.ends_with(".hsd"),
        "expected HOME/.hsd fallback, got: {data_dir}"
    );
}

#[tokio::test]
async fn node_status_data_dir_respects_hsd_prefix_setting() {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    db::queries::set_setting(&conn, "hsd_prefix", "/custom/hsd/data").unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");
    assert_eq!(v["data_dir"], serde_json::json!("/custom/hsd/data"));
}

// ===========================================================================
// active_profile_network fallback: active profile ID set but the profile row
// doesn't exist → should fall back to Network::Main (node.rs L176).
// ===========================================================================

#[tokio::test]
async fn node_status_names_no_network_for_a_missing_profile_row() {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    // Set active profile to a non-existent ID — triggers Ok(None) from
    // get_wallet_profile: no wallet, so no network to name.
    db::queries::set_setting(&conn, "active_wallet_profile_id", "ghost_id").unwrap();
    let app = app_with(conn);
    let v = node_status(app.state()).await.expect("node_status ok");
    assert_eq!(
        v["network"],
        serde_json::Value::Null,
        "a missing profile names no network (spec 2026-10-24 R8)"
    );
}

// ===========================================================================
// probe_node routes through the active profile's effective node config (ADR-001)
//
// The tray/status probe (node_status -> probe_node, and the backend loop's
// probe_and_update) must resolve the active profile's effective node config
// (per-profile override -> global settings -> built-in default) so the status
// reflects the node for the active profile, not just global settings.
// ===========================================================================

#[tokio::test]
async fn probe_and_update_uses_active_profile_override_over_global() {
    // Active profile W1 overrides node_rpc_url to a working server; global is
    // unreachable. probe_and_update must answer true via the override.
    let mut server_override = mockito::Server::new_async().await;
    let _m = server_override
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":77,"headers":77,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    create_test_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    set_profile_override(&conn, "W1", "node_rpc_url", &server_override.url());

    let app = app_with(conn);
    let state = app.state::<AppState>();
    let alive = crate::commands::node::probe_and_update(&state).await;
    assert!(
        alive,
        "probe must use the active profile override, not the unreachable global URL"
    );
}

#[tokio::test]
async fn probe_and_update_falls_back_to_global_when_no_override() {
    // Active profile W1 has no override; probe must fall back to the global
    // node_rpc_url and answer true.
    let mut server_global = mockito::Server::new_async().await;
    let _m = server_global
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":88,"headers":88,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server_global.url()).unwrap();
    create_test_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();
    let alive = crate::commands::node::probe_and_update(&state).await;
    assert!(
        alive,
        "probe must fall back to global settings when no override"
    );
}

#[tokio::test]
async fn probe_and_update_uses_builtin_default_when_no_override_or_global() {
    // No override, no global node_rpc_url: probe resolves to the built-in
    // default (localhost:12037), which is unreachable in tests → false, no panic.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    create_test_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();
    let alive = crate::commands::node::probe_and_update(&state).await;
    assert!(
        !alive,
        "unreachable built-in default must yield false without panicking"
    );
}

#[tokio::test]
async fn probe_and_update_uses_global_when_no_active_profile() {
    // No active profile at all: probe must resolve global settings and succeed.
    let mut server_global = mockito::Server::new_async().await;
    let _m = server_global
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":99,"headers":99,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server_global.url()).unwrap();
    // No profile, no active profile.

    let app = app_with(conn);
    let state = app.state::<AppState>();
    let alive = crate::commands::node::probe_and_update(&state).await;
    assert!(
        alive,
        "probe must use global settings when there is no active profile"
    );
}

// --- Network isolation: scoped data dir + idempotent, non-destructive migration.
use crate::commands::local_node::network_scoped_data_dir;
use crate::commands::node::{
    dir_has_chain_root, migrate_network_prefix, plan_network_migration, PrefixMigration,
};

#[tokio::test]
async fn probe_refuses_a_profile_whose_node_config_will_not_resolve() {
    // ADR-001: "a per-profile override that fails is not a fallback — it is a
    // user-facing error". The probe used to swallow the failure and describe
    // whatever node global happens to name, which on another chain is a
    // confident answer about somebody else's node.
    let mut server_global = mockito::Server::new_async().await;
    let m = server_global
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":99,"headers":99,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect(0)
        .create_async()
        .await;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server_global.url()).unwrap();
    // An active id pointing at a profile that does not exist: resolution
    // returns NotFound, which is exactly the "will not resolve" case.
    db::queries::set_setting(&conn, "active_wallet_profile_id", "ghost").unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();
    assert!(
        !crate::commands::node::probe_and_update(&state).await,
        "an unresolvable profile config must read as no node, not as global's"
    );
    m.assert_async().await;
}

/// A unique scratch dir under the OS temp dir, cleaned on drop.
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("namehold-netiso-{tag}-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_chain_root(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("chain")).unwrap();
    std::fs::create_dir_all(dir.join("blocks")).unwrap();
    std::fs::create_dir_all(dir.join("tree")).unwrap();
}

#[test]
fn network_scoped_data_dir_keeps_mainnet_at_base() {
    // Mainnet is never scoped — existing mainnet data must stay at the root.
    assert_eq!(
        network_scoped_data_dir("/data/hsd", Network::Main),
        "/data/hsd"
    );
}

#[test]
fn network_scoped_data_dir_scopes_non_mainnet_under_base() {
    assert_eq!(
        network_scoped_data_dir("/data/hsd", Network::Regtest),
        "/data/hsd/regtest"
    );
    assert_eq!(
        network_scoped_data_dir("/data/hsd", Network::Testnet),
        "/data/hsd/testnet"
    );
    assert_eq!(
        network_scoped_data_dir("/data/hsd", Network::Simnet),
        "/data/hsd/simnet"
    );
}

#[test]
fn plan_network_migration_never_touches_mainnet() {
    // Mainnet: no-op regardless of what's on disk.
    assert_eq!(
        plan_network_migration(true, false, true),
        PrefixMigration::None
    );
    assert_eq!(
        plan_network_migration(true, true, true),
        PrefixMigration::None
    );
}

#[test]
fn plan_network_migration_does_nothing_when_scoped_root_populated() {
    // "do nothing if we already have": a populated scoped root is left alone,
    // even if a legacy subdir also exists.
    assert_eq!(
        plan_network_migration(false, true, false),
        PrefixMigration::None
    );
    assert_eq!(
        plan_network_migration(false, true, true),
        PrefixMigration::None
    );
}

#[test]
fn plan_network_migration_creates_or_adopts_when_scoped_root_empty() {
    assert_eq!(
        plan_network_migration(false, false, false),
        PrefixMigration::CreateScopedRoot
    );
    assert_eq!(
        plan_network_migration(false, false, true),
        PrefixMigration::AdoptLegacySubdir
    );
}

#[test]
fn dir_has_chain_root_detects_hsd_layout() {
    let s = Scratch::new("chainroot");
    assert!(
        !dir_has_chain_root(s.path()),
        "empty dir is not a chain root"
    );
    std::fs::create_dir_all(s.path().join("chain")).unwrap();
    assert!(!dir_has_chain_root(s.path()), "chain alone is not enough");
    std::fs::create_dir_all(s.path().join("blocks")).unwrap();
    assert!(
        dir_has_chain_root(s.path()),
        "chain + blocks is a chain root"
    );
}

fn app_with_prefix_and_regtest(base: &std::path::Path) -> tauri::App<tauri::test::MockRuntime> {
    let conn = blank_conn();
    db::queries::set_setting(&conn, "hsd_prefix", base.to_str().unwrap()).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "test",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();
    app_with(conn)
}

#[test]
fn migrate_creates_scoped_root_when_base_empty() {
    // Fresh base: regtest gets its own <base>/regtest root created.
    let s = Scratch::new("fresh");
    let app = app_with_prefix_and_regtest(s.path());
    let scoped = s.path().join("regtest");
    migrate_network_prefix(&app.state(), Network::Regtest, scoped.to_str().unwrap()).unwrap();
    assert!(scoped.is_dir(), "scoped regtest root created");
}

#[test]
fn migrate_leaves_mainnet_shaped_base_root_untouched() {
    // The exact broken case: a mainnet chain was synced into a regtest prefix's
    // ROOT (no --regtest flag). Migration must NOT touch that root; it just
    // gives regtest its own scoped root beside it.
    let s = Scratch::new("brokenroot");
    make_chain_root(s.path()); // mainnet-shaped chain at the base root
    let marker = s.path().join("chain").join("MARKER");
    std::fs::write(&marker, b"mainnet").unwrap();

    let app = app_with_prefix_and_regtest(s.path());
    let scoped = s.path().join("regtest");
    migrate_network_prefix(&app.state(), Network::Regtest, scoped.to_str().unwrap()).unwrap();

    assert!(scoped.is_dir(), "regtest got its own scoped root");
    assert!(
        marker.exists(),
        "mainnet data at the base root is untouched"
    );
    // The base root's chain was NOT moved into the scoped root.
    assert!(!scoped.join("chain").join("MARKER").exists());
}

#[test]
fn migrate_is_noop_when_scoped_root_already_has_chain() {
    // "do nothing if we already have": a populated scoped root is not disturbed.
    let s = Scratch::new("already");
    let scoped = s.path().join("regtest");
    make_chain_root(&scoped);
    let marker = scoped.join("chain").join("KEEP");
    std::fs::write(&marker, b"regtest").unwrap();

    let app = app_with_prefix_and_regtest(s.path());
    migrate_network_prefix(&app.state(), Network::Regtest, scoped.to_str().unwrap()).unwrap();

    assert!(
        marker.exists(),
        "existing scoped chain is left exactly as-is"
    );
}

#[test]
fn migrate_adopts_legacy_nested_subdir_into_scoped_root() {
    // Legacy layout: hsd ran with the bare base as --prefix and nested the
    // network's data at <base>/regtest/regtest. Migration lifts it up into the
    // scoped root <base>/regtest.
    let s = Scratch::new("legacy");
    let nested = s.path().join("regtest").join("regtest");
    make_chain_root(&nested);
    let marker = nested.join("chain").join("LEGACY");
    std::fs::write(&marker, b"regtest").unwrap();

    let app = app_with_prefix_and_regtest(s.path());
    let scoped = s.path().join("regtest");
    migrate_network_prefix(&app.state(), Network::Regtest, scoped.to_str().unwrap()).unwrap();

    assert!(
        scoped.join("chain").join("LEGACY").exists(),
        "legacy nested chain lifted into the scoped root"
    );
    assert!(
        !nested.join("chain").join("LEGACY").exists(),
        "moved, not copied"
    );
}

#[test]
fn migrate_of_a_partial_layout_never_overwrites_what_the_scoped_root_has() {
    // A half-migrated layout: the scoped root already has a `chain` dir (but no
    // blocks/tree, so it is not a chain root) while the legacy subdir still
    // holds a full chain. The entry already present is kept, the rest moves.
    let s = Scratch::new("partial");
    let scoped = s.path().join("regtest");
    let nested = scoped.join("regtest");
    make_chain_root(&nested);
    std::fs::write(nested.join("chain").join("LEGACY"), b"legacy").unwrap();
    std::fs::create_dir_all(scoped.join("chain")).unwrap();
    std::fs::write(scoped.join("chain").join("KEEP"), b"scoped").unwrap();

    let app = app_with_prefix_and_regtest(s.path());
    migrate_network_prefix(&app.state(), Network::Regtest, scoped.to_str().unwrap()).unwrap();

    assert!(scoped.join("chain").join("KEEP").exists(), "kept as it was");
    assert!(
        !scoped.join("chain").join("LEGACY").exists(),
        "the scoped chain dir is not overwritten by the legacy one"
    );
    assert!(
        nested.join("chain").join("LEGACY").exists(),
        "the legacy chain dir stays where it was"
    );
    assert!(
        scoped.join("blocks").is_dir() && !nested.join("blocks").exists(),
        "an entry the scoped root lacks is moved up"
    );
}

#[test]
fn migrate_reports_a_scoped_root_it_cannot_create() {
    // A file where the scoped root should be: creating the directory fails, and
    // the error names the data dir, for both plans that create it.
    let s = Scratch::new("blocked");
    let blocker = s.path().join("blocker");
    std::fs::write(&blocker, b"not a dir").unwrap();
    let scoped = blocker.join("regtest");
    let scoped_dir = scoped.to_str().unwrap();
    let app = app_with_prefix_and_regtest(s.path());

    // CreateScopedRoot: no legacy chain anywhere.
    let err = migrate_network_prefix(&app.state(), Network::Regtest, scoped_dir).unwrap_err();
    assert!(
        matches!(&err, crate::error::AppError::Other(m) if m.contains(&format!("cannot create data dir {scoped_dir}"))),
        "{err:?}"
    );

    // AdoptLegacySubdir: a legacy chain under the base, scoped root blocked.
    let nested = s.path().join("regtest").join("regtest");
    make_chain_root(&nested);
    let err = migrate_network_prefix(&app.state(), Network::Regtest, scoped_dir).unwrap_err();
    assert!(
        matches!(&err, crate::error::AppError::Other(m) if m.contains(&format!("cannot create data dir {scoped_dir}"))),
        "{err:?}"
    );
    assert!(nested.join("chain").is_dir(), "nothing was moved");
}

// --- Which hsd log lines mean the node failed to start ---

use crate::commands::node::is_fatal_startup_line;

#[test]
fn routine_sync_noise_is_not_a_startup_failure() {
    // The line that made this predicate necessary: hsd logs peer failures with
    // the word "Error" throughout a healthy sync.
    for benign in [
        "[debug] (net) Error: Socket Error: ECONNREFUSED (1.2.3.4:12038)",
        "[warning] (peer) Error: Peer timed out.",
        "[info] (chain) Block 000000 (1) added to chain.",
        "[debug] (mempool) Added transaction to mempool.",
    ] {
        assert!(
            !is_fatal_startup_line(benign),
            "should not read as a startup failure: {benign}"
        );
    }
}

#[test]
fn the_known_fatal_shapes_are_recognised() {
    for fatal in [
        "[error] (node) Cannot retroactively enable address indexing.",
        "Error: bind EADDRINUSE 0.0.0.0:12037",
        "Error: listen EADDRINUSE: address already in use :::12037",
        "Uncaught Error: Could not open database.",
        "[error] (chain) cannot open chain database",
    ] {
        assert!(
            is_fatal_startup_line(fatal),
            "should read as a startup failure: {fatal}"
        );
    }
}

#[test]
fn a_data_dir_path_that_merely_contains_bind_is_not_a_failure() {
    // `bind` used to be matched on its own, so a prefix like this — or any
    // "binding"/"rebinding" progress line — reported a healthy node as broken.
    // The address-in-use case it existed for arrives as `bind EADDRINUSE`,
    // which is still caught above.
    assert!(!is_fatal_startup_line(
        "[info] (node) Opening /Volumes/bind-drive/hsd-data"
    ));
    assert!(!is_fatal_startup_line(
        "[debug] (chain) Rebinding handlers."
    ));
}

#[test]
fn a_broad_matcher_is_documented_rather_than_quietly_wrong() {
    // `Cannot ` is deliberately broad: this predicate only runs when the RPC is
    // already unreachable, so over-reporting relabels "still starting" on a node
    // that is down either way, while under-reporting leaves a broken node
    // silent. This pins that it is a choice, not an oversight.
    assert!(is_fatal_startup_line(
        "[info] (chain) Cannot find checkpoint."
    ));
}

/// A background probe whose database cannot be opened has no profile to
/// route by, so no node is authoritative for it.
#[tokio::test]
async fn probe_of_a_database_that_cannot_be_opened_is_not_ready() {
    let missing = std::env::temp_dir()
        .join(format!("namehold_no_such_dir_{}", std::process::id()))
        .join("wallet.db");
    let path = missing.to_str().unwrap();
    assert_eq!(
        node_tip_height_if_synced_from_profile_with_network(path, "W1", Some("mainnet")).await,
        None
    );
    assert!(!node_ready_from_profile(path, "W1", Some("mainnet")).await);
}

// --- Spec 2026-10-24 R7/R11: whose node answers, and whether it is starting ---

fn regtest_info_mock_body() -> &'static str {
    r#"{"result":{"chain":"regtest","blocks":5,"headers":5,"verificationprogress":1.0},"error":null,"id":1}"#
}

/// An own process that has not answered yet is starting; one that answers is
/// own and not starting.
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn an_own_node_not_answering_yet_is_starting() {
    let app = app_with(conn_with_active_regtest("http://127.0.0.1:1"));
    let child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    app.state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .insert(crate::noncustodial::network::Network::Regtest, child);

    let v = node_status(app.state()).await.unwrap();
    assert_eq!(v["ownership"], "own");
    assert_eq!(v["starting"], true);

    let mut child = app
        .state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .remove(&crate::noncustodial::network::Network::Regtest)
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
}

/// A node that answers but that Namehold did not start is outside; nothing
/// answering and nothing of ours is none.
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn a_node_that_answers_but_is_not_ours_is_outside() {
    let mut server = mockito::Server::new_async().await;
    let _info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(regtest_info_mock_body())
        .create_async()
        .await;
    let app = app_with(conn_with_active_regtest(&server.url()));
    let v = node_status(app.state()).await.unwrap();
    assert_eq!(v["ownership"], "outside");
    assert_eq!(v["starting"], false);

    let app = app_with(conn_with_active_regtest("http://127.0.0.1:1"));
    let v = node_status(app.state()).await.unwrap();
    assert_eq!(v["ownership"], "none");
    assert_eq!(v["starting"], false);
}

/// Spec 2026-10-24 R7: a re-sync moves a node's chain aside, so it is refused
/// for a node Namehold did not start, before any file is touched.
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn resync_refuses_an_outside_node_and_moves_nothing() {
    let mut server = mockito::Server::new_async().await;
    let _info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(regtest_info_mock_body())
        .create_async()
        .await;
    let conn = conn_with_active_regtest(&server.url());
    let prefix = db::queries::get_settings(&conn).unwrap()["hsd_prefix"].clone();
    let chain = std::path::Path::new(&prefix).join("regtest").join("chain");
    std::fs::create_dir_all(&chain).unwrap();
    let app = app_with(conn);

    let err = crate::commands::node::resync_hsd_chain(app.state())
        .await
        .expect_err("an outside node's chain is not moved");

    assert_eq!(
        err.to_string(),
        format!(
            "Invalid input: {}",
            crate::commands::local_node::OUTSIDE_NODE_NOT_STOPPED
        )
    );
    assert!(chain.exists(), "the chain stays where it was");
}

/// An own node that answers is own and not starting: the answer, not the
/// process, ends "starting" (R11), and an answering node of ours is never
/// taken for an outside one (R7), which would disable its Stop.
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn an_own_node_that_answers_is_own_and_not_starting() {
    let mut server = mockito::Server::new_async().await;
    let _info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(regtest_info_mock_body())
        .create_async()
        .await;
    let app = app_with(conn_with_active_regtest(&server.url()));
    let child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    app.state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .insert(crate::noncustodial::network::Network::Regtest, child);

    let v = node_status(app.state()).await.unwrap();

    assert_eq!(v["ownership"], "own");
    assert_eq!(v["starting"], false);
    assert_eq!(v["connected"], true);
    let mut child = app
        .state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .remove(&crate::noncustodial::network::Network::Regtest)
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
}
