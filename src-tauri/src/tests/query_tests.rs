use rusqlite::Connection;

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        ",
    )
    .unwrap();
    let sql = include_str!("../sql/001_initial.sql");
    conn.execute_batch(sql).unwrap();
    conn
}

#[test]
fn test_settings_crud() {
    let conn = setup_db();
    let settings = crate::db::queries::get_settings(&conn).unwrap();
    assert!(settings.contains_key("hsd_wallet_api_url"));
    assert_eq!(settings["hsd_wallet_api_url"], "http://127.0.0.1:12039");

    crate::db::queries::set_setting(&conn, "hsd_wallet_api_url", "http://127.0.0.1:14039").unwrap();
    let settings = crate::db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["hsd_wallet_api_url"], "http://127.0.0.1:14039");

    crate::db::queries::set_setting(&conn, "new_key", "new_value").unwrap();
    let settings = crate::db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["new_key"], "new_value");
}

#[test]
fn test_asset_crud() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO assets (tld, status, is_staked, category, notes) VALUES ('test', 'not_started', 0, 'cat1', 'note1')",
        [],
    )
    .unwrap();

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].tld, "test");
    assert_eq!(assets[0].category.as_deref(), Some("cat1"));
    assert_eq!(assets[0].notes.as_deref(), Some("note1"));
    assert!(!assets[0].is_staked);

    conn.execute(
        "INSERT INTO assets (tld, status, is_staked) VALUES ('staked_tld', 'do_not_touch_staked', 1)",
        [],
    )
    .unwrap();

    let all = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(all.len(), 2);

    let staked = crate::db::queries::list_assets(&conn, None, Some(true), None, None, None).unwrap();
    assert_eq!(staked.len(), 1);
    assert_eq!(staked[0].tld, "staked_tld");

    let unstaked = crate::db::queries::list_assets(&conn, None, Some(false), None, None, None).unwrap();
    assert_eq!(unstaked.len(), 1);
    assert_eq!(unstaked[0].tld, "test");

    let by_status = crate::db::queries::list_assets(&conn, Some("not_started"), None, None, None, None).unwrap();
    assert_eq!(by_status.len(), 1);

    let by_search = crate::db::queries::list_assets(&conn, None, None, Some("cat1"), None, None).unwrap();
    assert_eq!(by_search.len(), 1);

    let by_search_miss = crate::db::queries::list_assets(&conn, None, None, Some("nonexistent"), None, None).unwrap();
    assert_eq!(by_search_miss.len(), 0);

    let sorted = crate::db::queries::list_assets(&conn, None, None, None, Some("tld"), Some("desc")).unwrap();
    assert_eq!(sorted[0].tld, "test");
    assert_eq!(sorted[1].tld, "staked_tld");
}

#[test]
fn test_asset_update() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('test', 'not_started')",
        [],
    )
    .unwrap();

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let id = assets[0].id;

    crate::db::queries::update_asset(&conn, id, Some("namebase_transfer_requested"), None, None, Some("updated note"), None, None, None).unwrap();

    let asset = crate::db::queries::get_asset(&conn, id).unwrap();
    assert_eq!(asset.status.as_str(), "namebase_transfer_requested");
    assert_eq!(asset.notes.as_deref(), Some("updated note"));
}

#[test]
fn test_bulk_update_status() {
    let conn = setup_db();

    for i in 0..5 {
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES (?1, 'not_started')",
            [format!("tld{}", i)],
        )
        .unwrap();
    }

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();

    let updated = crate::db::queries::bulk_update_status(&conn, &ids, "namebase_transfer_requested").unwrap();
    assert_eq!(updated, 5);

    let assets = crate::db::queries::list_assets(&conn, Some("namebase_transfer_requested"), None, None, None, None).unwrap();
    assert_eq!(assets.len(), 5);
}

#[test]
fn test_bulk_update_tags() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('test', 'not_started')",
        [],
    )
    .unwrap();

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();

    crate::db::queries::bulk_update_tags(&conn, &ids, r#"["high_value","test"]"#).unwrap();

    let asset = crate::db::queries::get_asset(&conn, ids[0]).unwrap();
    assert_eq!(asset.tags, vec!["high_value", "test"]);
}

#[test]
fn test_batch_crud() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('a', 'not_started')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('b', 'not_started')",
        [],
    )
    .unwrap();

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();

    let batch_id = crate::db::queries::create_batch(&conn, "Test Batch", Some("desc"), &ids).unwrap();
    assert!(batch_id > 0);

    let batches = crate::db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].name, "Test Batch");

    let batch_with = crate::db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch_with.assets.len(), 2);

    crate::db::queries::update_batch(&conn, batch_id, None, None, Some("in_progress")).unwrap();
    let batches = crate::db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches[0].status.as_str(), "in_progress");

    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('c', 'not_started')",
        [],
    )
    .unwrap();
    let new_assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let c_id = new_assets.iter().find(|a| a.tld == "c").unwrap().id;

    crate::db::queries::add_to_batch(&conn, batch_id, &[c_id]).unwrap();
    let batch_with = crate::db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch_with.assets.len(), 3);

    crate::db::queries::remove_from_batch(&conn, batch_id, &[c_id]).unwrap();
    let batch_with = crate::db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch_with.assets.len(), 2);

    crate::db::queries::delete_batch(&conn, batch_id).unwrap();
    let batches = crate::db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches.len(), 0);
}

#[test]
fn test_wallet_snapshot() {
    let conn = setup_db();

    let id = crate::db::queries::insert_wallet_snapshot(&conn, "primary", 1000000, Some("rs1qtest"), 5, None).unwrap();
    assert!(id > 0);

    let snap = crate::db::queries::get_latest_wallet_snapshot(&conn).unwrap();
    assert!(snap.is_some());
    let snap = snap.unwrap();
    assert_eq!(snap["wallet_name"], "primary");
    assert_eq!(snap["balance"], 1000000);
    assert_eq!(snap["name_count"], 5);
}

#[test]
fn test_audit_log() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('test_action', 'test_detail')",
        [],
    )
    .unwrap();

    let entries = crate::db::queries::get_recent_audit_log(&conn, 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["action"], "test_action");
}
