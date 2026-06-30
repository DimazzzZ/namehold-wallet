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
    // 001 initial, 002 hsd prefix, 003 provider modes, 004 wallet addresses,
    // 005 fix hnsfans api url.
    assert_eq!(count, 5);
}

#[test]
fn test_default_settings_seeded() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    let url: String = conn
        .query_row("SELECT value FROM settings WHERE key = 'hsd_wallet_api_url'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(url, "http://127.0.0.1:12039");

    let network: String = conn
        .query_row("SELECT value FROM settings WHERE key = 'hsd_network'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(network, "mainnet");
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
