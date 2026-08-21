//! Tests for `commands::watched_states` — the single `get_watched_states` command.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::watched_states::get_watched_states;
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

#[test]
fn empty_table_returns_empty_vec() {
    let app = app_with(migrated_conn());
    let result = get_watched_states(app.state()).unwrap();
    assert!(result.is_empty());
}

#[test]
fn returns_seeded_rows_ordered_by_polled_at_desc() {
    let conn = migrated_conn();
    conn.execute_batch(
        "INSERT INTO watched_name_states (name, last_phase, last_state_json, last_highest_doos, blocks_until_next, polled_at)
         VALUES ('alpha', 'BIDDING', '{\"state\":\"BIDDING\"}', 100000, 50, '2024-01-01T00:00:00Z');
         INSERT INTO watched_name_states (name, last_phase, last_state_json, last_highest_doos, blocks_until_next, polled_at)
         VALUES ('beta', 'CLOSED', NULL, NULL, NULL, '2024-06-15T12:00:00Z');",
    )
    .unwrap();

    let app = app_with(conn);
    let rows = get_watched_states(app.state()).unwrap();

    assert_eq!(rows.len(), 2);
    // Ordered by polled_at DESC: beta (June) before alpha (Jan).
    assert_eq!(rows[0].name, "beta");
    assert_eq!(rows[0].last_phase.as_deref(), Some("CLOSED"));
    assert!(rows[0].last_state_json.is_none());

    assert_eq!(rows[1].name, "alpha");
    assert_eq!(rows[1].last_phase.as_deref(), Some("BIDDING"));
    assert_eq!(rows[1].last_highest_doos, Some(100000));
    assert_eq!(rows[1].blocks_until_next, Some(50));
}

#[test]
fn null_fields_are_handled_correctly() {
    let conn = migrated_conn();
    conn.execute_batch(
        "INSERT INTO watched_name_states (name, last_phase, last_state_json, last_highest_doos, blocks_until_next, polled_at)
         VALUES ('gamma', NULL, NULL, NULL, NULL, '2024-03-01T08:00:00Z');",
    )
    .unwrap();

    let app = app_with(conn);
    let rows = get_watched_states(app.state()).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "gamma");
    assert!(rows[0].last_phase.is_none());
    assert!(rows[0].last_state_json.is_none());
    assert!(rows[0].last_highest_doos.is_none());
    assert!(rows[0].blocks_until_next.is_none());
}
