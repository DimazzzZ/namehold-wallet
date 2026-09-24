use rusqlite::Connection;
use std::sync::Mutex;

pub fn create_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .unwrap();
    let sql = include_str!("../../../src-tauri/src/sql/001_initial.sql");
    conn.execute_batch(sql).unwrap();
    let sql2 = include_str!("../../../src-tauri/src/sql/002_hsd_prefix.sql");
    conn.execute_batch(sql2).unwrap();
    let sql3 = include_str!("../../../src-tauri/src/sql/003_provider_modes.sql");
    conn.execute_batch(sql3).unwrap();
    conn
}

pub fn create_test_state() -> crate::AppState {
    let conn = create_test_db();
    crate::AppState {
        db: Mutex::new(conn),
        signer: Mutex::new(None),
        secure_prompts: Mutex::new(std::collections::HashMap::new()),
        hsd_child: Mutex::new(None),
        node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
        sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::commands::sync::SyncStatus::default(),
        )),
    }
}

/// Set one per-profile node-config override (ADR-001), upserting.
///
/// Four test files and the watched-name daemon's own test module each carried
/// their own copy of this. Four were byte-identical; the fifth used
/// `INSERT OR REPLACE`, which differs on an existing row — it rewrites the
/// whole row rather than the value, so a future column would silently be
/// reset. One copy, and the shapes cannot drift again.
pub fn set_profile_override(conn: &Connection, profile_id: &str, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO profile_settings (profile_id, key, value) VALUES (?1, ?2, ?3)
         ON CONFLICT(profile_id, key) DO UPDATE SET value = excluded.value",
        rusqlite::params![profile_id, key, value],
    )
    .unwrap();
}
