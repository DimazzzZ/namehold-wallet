use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_db_path() -> std::path::PathBuf {
    let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("namehold_test_{}.db", n))
}

#[test]
fn test_open_file_db_with_wal() {
    // Opening a file-based DB should succeed with WAL + foreign keys enabled.
    let path = temp_db_path();
    let conn = crate::db::connection::open(&path).unwrap();
    let wal: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(wal.to_lowercase(), "wal");

    let fk: i32 = conn
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 1);

    // Clean up the temp file.
    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("wal"));
    let _ = std::fs::remove_file(path.with_extension("shm"));
}

#[test]
fn test_migrations_run_idempotent() {
    let conn = crate::tests::command_helpers::create_test_db();

    // Running migrations again should be a no-op (idempotent).
    crate::db::migrations::run(&conn).unwrap();

    // Verify all 12 migrations are recorded.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 12);
}

#[test]
fn test_migrations_create_tables() {
    // Use full migrations so all tables are created, including wallet_profiles (migration 006).
    let path = temp_db_path();
    let conn = crate::db::connection::open(&path).unwrap();
    crate::db::migrations::run(&conn).unwrap();

    // Spot-check that all expected tables exist.
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(tables.contains(&"assets".to_string()));
    assert!(tables.contains(&"batches".to_string()));
    assert!(tables.contains(&"settings".to_string()));
    assert!(tables.contains(&"audit_log".to_string()));
    assert!(tables.contains(&"wallet_profiles".to_string()));
    assert!(tables.contains(&"schema_version".to_string()));
    assert!(tables.contains(&"sync_cursors".to_string()));

    // Clean up the temp file.
    drop(conn);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("wal"));
    let _ = std::fs::remove_file(path.with_extension("shm"));
}
