use rusqlite::Connection;

#[test]
fn test_migration_runs_on_empty_db() {
    let conn = Connection::open_in_memory().unwrap();
    let result = crate::db::migrations::run(&conn);
    assert!(result.is_ok());

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(tables.contains(&"settings".to_string()));
    assert!(tables.contains(&"assets".to_string()));
    assert!(tables.contains(&"batches".to_string()));
    assert!(tables.contains(&"batch_assets".to_string()));
    assert!(tables.contains(&"wallet_snapshots".to_string()));
    assert!(tables.contains(&"audit_log".to_string()));
    assert!(tables.contains(&"schema_version".to_string()));
}

#[test]
fn test_migration_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    let r1 = crate::db::migrations::run(&conn);
    assert!(r1.is_ok());

    let r2 = crate::db::migrations::run(&conn);
    assert!(r2.is_ok());

    let version: String = conn
        .query_row("SELECT version FROM schema_version WHERE version = '001'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "001");
}

#[test]
fn test_schema_version_tracking() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    // 001..009, 010 (drop legacy settings), 011 (re-add hsd data dir),
    // 012 (tx-draft confirmation tracking).
    assert_eq!(count, 12);
}

#[test]
fn test_default_settings_seeded() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    // Non-custodial settings survive; the node RPC URL is seeded by 009.
    let node_url: String = conn
        .query_row("SELECT value FROM settings WHERE key = 'node_rpc_url'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(node_url, "http://127.0.0.1:12037");

    // Legacy keys are removed by migration 010.
    for key in ["hsd_wallet_api_url", "connection_mode", "write_mode", "chain_source"] {
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings WHERE key = ?1", [key], |row| row.get(0))
            .unwrap();
        assert_eq!(n, 0, "legacy setting '{key}' should be deleted");
    }

    // The hsd data directory is re-added by migration 011 (010 drops it, 011
    // brings it back), so the app can start hsd against a custom prefix.
    let hsd_prefix: String = conn
        .query_row("SELECT value FROM settings WHERE key = 'hsd_prefix'", [], |row| row.get(0))
        .expect("hsd_prefix should exist after the full migration chain");
    assert_eq!(hsd_prefix, "", "hsd_prefix defaults to empty (= hsd's own ~/.hsd)");
}

#[test]
fn test_tx_draft_confirmation_schema() {
    // Migration 012 adds the confirmation_height column and the 'confirmed' /
    // 'dropped' terminal statuses (recreating the table to change the CHECK).
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(wallet_tx_drafts)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        cols.iter().any(|c| c == "confirmation_height"),
        "confirmation_height column should exist"
    );

    // Exercise the status CHECK, not the FK to wallet_profiles — drop FK
    // enforcement so an arbitrary wallet_profile_id is allowed. id == status
    // keeps PKs unique across the calls below.
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    let insert = |status: &str| {
        conn.execute(
            "INSERT INTO wallet_tx_drafts
                (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json, summary_json, status)
             VALUES (?1, 'p', 'send_hns', '00', '{}', '{}', ?2)",
            rusqlite::params![status, status],
        )
    };
    assert!(insert("confirmed").is_ok(), "'confirmed' must be accepted");
    assert!(insert("dropped").is_ok(), "'dropped' must be accepted");
    assert!(insert("broadcasted").is_ok(), "existing statuses still accepted");
    assert!(insert("bogus").is_err(), "an unknown status must be rejected by the CHECK");
}

#[test]
fn test_connection_open() {
    let dir = std::env::temp_dir().join("namehold_test_db");
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");

    let result = crate::db::connection::open(&db_path);
    assert!(result.is_ok());

    let conn = result.unwrap();
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");

    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 1);

    let _ = std::fs::remove_dir_all(&dir);
}
