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
    assert_eq!(pick_hsd_path(None, &["/no/such/path/hsd".to_string()]), None);
    let _ = std::fs::remove_dir_all(&dir);
}
