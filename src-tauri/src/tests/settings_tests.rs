use rusqlite::Connection;

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;").unwrap();
    let sql = include_str!("../../../src-tauri/src/sql/001_initial.sql");
    conn.execute_batch(sql).unwrap();
    let sql2 = include_str!("../../../src-tauri/src/sql/002_hsd_prefix.sql");
    conn.execute_batch(sql2).unwrap();
    conn
}

#[test]
fn test_get_settings_returns_all_defaults() {
    let conn = setup_db();
    let settings = crate::db::queries::get_settings(&conn).unwrap();
    assert!(settings.contains_key("hsd_wallet_api_url"));
    assert!(settings.contains_key("hsd_node_api_url"));
    assert!(settings.contains_key("hsd_api_key"));
    assert!(settings.contains_key("hsd_wallet_id"));
    assert!(settings.contains_key("hsd_network"));
    assert!(settings.contains_key("hsd_prefix"));
    assert!(settings.contains_key("write_mode"));
}

#[test]
fn test_set_setting_creates_new() {
    let conn = setup_db();
    crate::db::queries::set_setting(&conn, "custom_key", "custom_value").unwrap();
    let settings = crate::db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["custom_key"], "custom_value");
}

#[test]
fn test_set_setting_updates_existing() {
    let conn = setup_db();
    crate::db::queries::set_setting(&conn, "hsd_network", "testnet").unwrap();
    let settings = crate::db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["hsd_network"], "testnet");
}

#[test]
fn test_update_asset_status() {
    let conn = setup_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let id = assets[0].id;
    crate::db::queries::update_asset(&conn, id, Some("finalized_owned"), None, None, None, None, None, None).unwrap();
    let asset = crate::db::queries::get_asset(&conn, id).unwrap();
    assert_eq!(asset.status.as_str(), "finalized_owned");
}

#[test]
fn test_update_asset_notes() {
    let conn = setup_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let id = assets[0].id;
    crate::db::queries::update_asset(&conn, id, None, None, None, Some("my note"), None, None, None).unwrap();
    let asset = crate::db::queries::get_asset(&conn, id).unwrap();
    assert_eq!(asset.notes.as_deref(), Some("my note"));
}

#[test]
fn test_bulk_update_multiple_assets() {
    let conn = setup_db();
    for i in 0..3 {
        conn.execute("INSERT INTO assets (tld, status) VALUES (?1, 'not_started')", [format!("tld{}", i)]).unwrap();
    }
    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();
    let updated = crate::db::queries::bulk_update_status(&conn, &ids, "waiting_transfer_tx").unwrap();
    assert_eq!(updated, 3);
    let assets = crate::db::queries::list_assets(&conn, Some("waiting_transfer_tx"), None, None, None, None).unwrap();
    assert_eq!(assets.len(), 3);
}

#[test]
fn test_batch_with_assets() {
    let conn = setup_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();
    let batch_id = crate::db::queries::create_batch(&conn, "Test", Some("desc"), &ids).unwrap();
    let batch = crate::db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 2);
    assert_eq!(batch.name, "Test");
}

#[test]
fn test_wallet_snapshot_roundtrip() {
    let conn = setup_db();
    let id = crate::db::queries::insert_wallet_snapshot(&conn, "primary", 5000000, Some("rs1qtest"), 10, None).unwrap();
    assert!(id > 0);
    let snap = crate::db::queries::get_latest_wallet_snapshot(&conn).unwrap();
    assert!(snap.is_some());
    let snap = snap.unwrap();
    assert_eq!(snap["balance"], 5000000);
    assert_eq!(snap["name_count"], 10);
}

#[test]
fn test_wallet_snapshots_list() {
    let conn = setup_db();
    for i in 0..5 {
        crate::db::queries::insert_wallet_snapshot(&conn, "primary", i * 1000, None, i, None).unwrap();
    }
    let snapshots = crate::db::queries::get_wallet_snapshots(&conn, 3).unwrap();
    assert_eq!(snapshots.len(), 3);
}

#[test]
fn test_audit_log_entries() {
    let conn = setup_db();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('test', 'detail1')", []).unwrap();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('test', 'detail2')", []).unwrap();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('other', 'detail3')", []).unwrap();
    let entries = crate::db::queries::get_recent_audit_log(&conn, 10).unwrap();
    assert_eq!(entries.len(), 3);
    let entries = crate::db::queries::get_recent_audit_log(&conn, 1).unwrap();
    assert_eq!(entries.len(), 1);
}
