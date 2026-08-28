//! Command-level tests for `commands::update_notify`.
//!
//! Drives the two thin `#[tauri::command]` functions through a managed
//! `AppState` over a fully-migrated in-memory DB, so the settings-read /
//! settings-write branches are exercised as shipped.
//!
//! The `#[tauri::command]` attribute line itself and the macro-generated IPC
//! wrapper it expands to are structurally impossible to cover from a plain
//! Rust unit test (they only fire under real Tauri IPC dispatch). Those
//! macro-attribute lines are the sole reason this file does not hit 100%
//! line coverage — every branch of the function bodies IS covered here.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::update_notify::{is_update_notify_enabled, set_update_notify_enabled};
use crate::db;
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

fn raw_setting(app: &tauri::App<tauri::test::MockRuntime>, key: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    db::queries::get_settings(&conn).unwrap().get(key).cloned()
}

// --- is_update_notify_enabled --------------------------------------------

#[test]
fn is_update_notify_enabled_defaults_to_false_when_unset() {
    let conn = migrated_conn();
    let app = app_with(conn);
    assert!(!is_update_notify_enabled(app.state()));
}

#[test]
fn is_update_notify_enabled_returns_true_when_set_to_true() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "update_notify_enabled", "true").unwrap();
    let app = app_with(conn);
    assert!(is_update_notify_enabled(app.state()));
}

#[test]
fn is_update_notify_enabled_returns_false_when_set_to_false() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "update_notify_enabled", "false").unwrap();
    let app = app_with(conn);
    assert!(!is_update_notify_enabled(app.state()));
}

#[test]
fn is_update_notify_enabled_returns_false_for_any_non_true_value() {
    // Anything other than the literal string "true" is treated as disabled.
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "update_notify_enabled", "yes").unwrap();
    let app = app_with(conn);
    assert!(!is_update_notify_enabled(app.state()));
}

// --- set_update_notify_enabled -------------------------------------------

#[test]
fn set_update_notify_enabled_true_persists_string_true() {
    let conn = migrated_conn();
    let app = app_with(conn);
    set_update_notify_enabled(true, app.state()).expect("set true");
    assert_eq!(raw_setting(&app, "update_notify_enabled").as_deref(), Some("true"));
    // Round-trips through the read command too.
    assert!(is_update_notify_enabled(app.state()));
}

#[test]
fn set_update_notify_enabled_false_persists_string_false() {
    let conn = migrated_conn();
    // Seed as enabled so we can prove the write flips it.
    db::queries::set_setting(&conn, "update_notify_enabled", "true").unwrap();
    let app = app_with(conn);
    assert!(is_update_notify_enabled(app.state()));

    set_update_notify_enabled(false, app.state()).expect("set false");
    assert_eq!(raw_setting(&app, "update_notify_enabled").as_deref(), Some("false"));
    assert!(!is_update_notify_enabled(app.state()));
}

#[test]
fn set_update_notify_enabled_overwrites_existing_value() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "update_notify_enabled", "garbage").unwrap();
    let app = app_with(conn);

    set_update_notify_enabled(true, app.state()).expect("set true");
    assert_eq!(raw_setting(&app, "update_notify_enabled").as_deref(), Some("true"));
}

// --- Error-path coverage (line 15: get_settings fails) ----------------------

/// When the `settings` table is missing (e.g. schema corruption), `get_settings`
/// returns `Err`. `is_update_notify_enabled` must gracefully fall back to `false`
/// rather than panicking.
#[test]
fn is_update_notify_enabled_returns_false_when_settings_table_missing() {
    // Open a bare in-memory DB without running migrations — no `settings` table.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let app = app_with(conn);
    // This exercises line 15: `Err(_) => false`.
    assert!(!is_update_notify_enabled(app.state()));
}
