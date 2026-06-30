use rusqlite::Connection;
use std::sync::Mutex;

pub fn create_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;").unwrap();
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
    }
}
