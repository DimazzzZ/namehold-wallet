//! `node_status` must reflect the REAL node connection (RPC answers), not just
//! whether we spawned a child. With no node reachable, `connected` is false and
//! `process_alive` is false — and it never falsely reports a connection.

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
            hsd_child: std::sync::Mutex::new(None),
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
    assert!(v["network"].is_string());
    // Sync-progress fields are always present (null when not connected).
    assert!(v.get("verification_progress").is_some());
    assert_eq!(v["verification_progress"], serde_json::Value::Null);
    assert!(v.get("headers").is_some());
    assert_eq!(v["headers"], serde_json::Value::Null);

    // read_source is always present and defaults to "explorer" when not connected.
    assert_eq!(v["read_source"], serde_json::json!("explorer"));
}

// --- is_node_ready_for_local_reads -------------------------------------------

use crate::commands::read::is_node_ready_for_local_reads;

#[tokio::test]
async fn local_reads_not_ready_when_not_connected() {
    let app = app_with(seeded_conn());
    let state = app.state::<AppState>();

    // Node is not connected → should not use local reads.
    assert!(!is_node_ready_for_local_reads(&state).await);
}

// --- node_ready_from_settings (the settings-based gate used by the background
//     sync thread, which has no State<AppState>) --------------------------------

use crate::commands::read::node_ready_from_settings;

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

    assert!(node_ready_from_settings(&settings_for_url(&server.url())).await);
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

    assert!(!node_ready_from_settings(&settings_for_url(&server.url())).await);
}

#[tokio::test]
async fn node_ready_from_settings_false_when_unreachable() {
    // Unroutable node → probe fails → not ready.
    assert!(!node_ready_from_settings(&settings_for_url("http://127.0.0.1:1")).await);
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
    assert!(app.state::<AppState>().hsd_child.lock().unwrap().is_none());

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
// so an ADOPTED node (RPC up, hsd_child None) still shows as running. This is
// the exact bug where the tray showed "Start Node" for a node that was up.
// ---------------------------------------------------------------------------

/// Adopted node: RPC answers `getblockchaininfo` but we never spawned a child
/// (`hsd_child` is None). After a probe, `node_rpc_alive` must be true — this
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
    // start_hsd found a running node and returned without setting hsd_child.
    assert!(
        state.hsd_child.lock().unwrap().is_none(),
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

// --- network_name_matches (the guard that prevents a regtest node from being
//     treated as authoritative for a mainnet wallet) ---------------------------

use crate::commands::read::network_name_matches;

#[test]
fn network_name_matches_same_network() {
    assert!(network_name_matches("main", "main"));
    assert!(network_name_matches("mainnet", "main"));
    assert!(network_name_matches("main", "mainnet"));
    assert!(network_name_matches("mainnet", "mainnet"));
    assert!(network_name_matches("testnet", "testnet"));
    assert!(network_name_matches("regtest", "regtest"));
    assert!(network_name_matches("simnet", "simnet"));
}

#[test]
fn network_name_matches_different_network() {
    assert!(!network_name_matches("mainnet", "regtest"));
    assert!(!network_name_matches("main", "regtest"));
    assert!(!network_name_matches("mainnet", "testnet"));
    assert!(!network_name_matches("testnet", "regtest"));
    assert!(!network_name_matches("regtest", "main"));
}

// --- node_tip_height_if_synced_from_settings_with_network ---------------------

use crate::commands::read::node_tip_height_if_synced_from_settings_with_network;

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
    assert_eq!(h, None, "regtest node must NOT be authoritative for mainnet wallet");
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
    assert_eq!(h, Some(100), "missing chain in response → conservatively allow");
}
