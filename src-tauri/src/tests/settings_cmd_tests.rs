//! Command-level tests for the settings/audit secrecy guarantees (security
//! audit issue #7). These drive the REAL `#[tauri::command]` functions
//! (`get_settings`, `update_setting`, `get_audit_log`) through a managed
//! `AppState` over a fully-migrated in-memory DB, so the redaction / denylist
//! logic is exercised as shipped — not via a hand-copied replica that could
//! drift from the command body.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::settings::{get_audit_log, get_settings, update_setting};
use crate::db;
use crate::error::AppError;
use crate::AppState;

fn migrated_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

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

/// Read the raw stored value for a setting straight from the DB (bypasses
/// redaction) so tests can prove the secret is actually persisted.
fn raw_setting(app: &tauri::App<tauri::test::MockRuntime>, key: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    db::queries::get_settings(&conn).unwrap().get(key).cloned()
}

// --- get_settings redaction -----------------------------------------------

#[tokio::test]
async fn get_settings_redacts_namebase_cookie_and_marks_presence() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "super-secret-session").unwrap();
    let app = app_with(conn);

    let out = get_settings(app.state()).await.unwrap();
    assert!(
        out.get("namebase_cookie").is_none(),
        "raw cookie must not leak"
    );
    assert_eq!(out["__has_namebase_cookie"], "true");
    // The secret is still stored server-side.
    assert_eq!(
        raw_setting(&app, "namebase_cookie").as_deref(),
        Some("super-secret-session")
    );
}

#[tokio::test]
async fn get_settings_redacts_node_rpc_api_key_and_marks_presence() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "node_rpc_api_key", "hunter2").unwrap();
    let app = app_with(conn);

    let out = get_settings(app.state()).await.unwrap();
    assert!(out.get("node_rpc_api_key").is_none());
    assert_eq!(out["__has_node_rpc_api_key"], "true");
}

#[tokio::test]
async fn get_settings_passes_through_non_sensitive_keys() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "hsd_prefix", "true").unwrap();
    let app = app_with(conn);

    let out = get_settings(app.state()).await.unwrap();
    assert_eq!(out["hsd_prefix"], "true");
}

#[tokio::test]
async fn get_settings_omits_presence_markers_when_unset() {
    let conn = migrated_conn();
    let app = app_with(conn);

    let out = get_settings(app.state()).await.unwrap();
    // The migrations insert default rows for node_rpc_api_key and hsd_api_key
    // (with empty values), so the markers ARE present. Only namebase_cookie
    // is never seeded, so its marker should be absent.
    assert!(out.get("__has_namebase_cookie").is_none());
    assert_eq!(out["__has_node_rpc_api_key"], "true");
    // hsd_api_key is deleted by migration 010, so no marker.
    assert!(out.get("__has_hsd_api_key").is_none());
}

// --- update_setting denylist ----------------------------------------------

#[tokio::test]
async fn update_setting_rejects_write_to_namebase_base_url() {
    let conn = migrated_conn();
    let app = app_with(conn);

    let err = update_setting(
        app.state(),
        "namebase_base_url".to_string(),
        "https://attacker.example".to_string(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
    // Nothing was written.
    assert_eq!(raw_setting(&app, "namebase_base_url"), None);
}

#[tokio::test]
async fn update_setting_rejects_write_to_namebase_cookie() {
    let conn = migrated_conn();
    let app = app_with(conn);

    let err = update_setting(
        app.state(),
        "namebase_cookie".to_string(),
        "forged".to_string(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
    assert_eq!(raw_setting(&app, "namebase_cookie"), None);
}

#[tokio::test]
async fn update_setting_allows_non_denylisted_keys() {
    let conn = migrated_conn();
    let app = app_with(conn);

    update_setting(app.state(), "hsd_prefix".to_string(), "true".to_string())
        .await
        .unwrap();
    assert_eq!(raw_setting(&app, "hsd_prefix").as_deref(), Some("true"));
}

// --- update_setting audit redaction ----------------------------------------

#[tokio::test]
async fn update_setting_writes_star_star_star_for_sensitive_key() {
    let conn = migrated_conn();
    let app = app_with(conn);

    // node_rpc_api_key is sensitive but NOT write-denied, so the write path
    // (and its audit entry) is reachable from the renderer.
    update_setting(
        app.state(),
        "node_rpc_api_key".to_string(),
        "leaky-secret".to_string(),
    )
    .await
    .unwrap();

    let log = get_audit_log(app.state(), Some(10)).await.unwrap();
    let detail = log.as_array().unwrap()[0]["detail"].as_str().unwrap();
    assert!(detail.contains("\"***\""), "audit must redact: {detail}");
    assert!(
        !detail.contains("leaky-secret"),
        "raw secret leaked: {detail}"
    );
}

#[tokio::test]
async fn update_setting_writes_raw_value_for_non_sensitive_key() {
    let conn = migrated_conn();
    let app = app_with(conn);

    update_setting(app.state(), "hsd_prefix".to_string(), "true".to_string())
        .await
        .unwrap();

    let log = get_audit_log(app.state(), Some(10)).await.unwrap();
    let detail = log.as_array().unwrap()[0]["detail"].as_str().unwrap();
    assert!(detail.contains("hsd_prefix"));
    assert!(detail.contains("true"));
}

// --- get_audit_log defense-in-depth ----------------------------------------

#[tokio::test]
async fn get_audit_log_redacts_legacy_plaintext_secret() {
    let conn = migrated_conn();
    // Simulate a row written BEFORE redaction existed: raw secret in detail.
    conn.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
        [serde_json::json!({"key": "namebase_cookie", "value": "OLD-PLAINTEXT"}).to_string()],
    )
    .unwrap();
    let app = app_with(conn);

    let log = get_audit_log(app.state(), Some(10)).await.unwrap();
    let detail = log.as_array().unwrap()[0]["detail"].as_str().unwrap();
    assert!(
        detail.contains("\"***\""),
        "legacy secret must be re-redacted: {detail}"
    );
    assert!(
        !detail.contains("OLD-PLAINTEXT"),
        "legacy secret leaked: {detail}"
    );
}

#[tokio::test]
async fn get_audit_log_leaves_non_sensitive_details_untouched() {
    let conn = migrated_conn();
    conn.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
        [serde_json::json!({"key": "hsd_prefix", "value": "true"}).to_string()],
    )
    .unwrap();
    let app = app_with(conn);

    let log = get_audit_log(app.state(), Some(10)).await.unwrap();
    let detail = log.as_array().unwrap()[0]["detail"].as_str().unwrap();
    assert!(detail.contains("hsd_prefix"));
    assert!(detail.contains("true"));
}

#[tokio::test]
async fn get_audit_log_survives_malformed_detail_json() {
    let conn = migrated_conn();
    conn.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
        ["this is not json"],
    )
    .unwrap();
    let app = app_with(conn);

    // Must not panic; the non-JSON detail passes through verbatim.
    let log = get_audit_log(app.state(), Some(10)).await.unwrap();
    let detail = log.as_array().unwrap()[0]["detail"].as_str().unwrap();
    assert_eq!(detail, "this is not json");
}
