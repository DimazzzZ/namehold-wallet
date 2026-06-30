use crate::db;

#[test]
fn test_list_assets_by_staked() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, is_staked, status) VALUES ('a', 1, 'do_not_touch_staked')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, is_staked, status) VALUES ('b', 0, 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, is_staked, status) VALUES ('c', 1, 'do_not_touch_staked')", []).unwrap();

    let staked = db::queries::list_assets(&conn, None, Some(true), None, None, None).unwrap();
    assert_eq!(staked.len(), 2);

    let unstaked = db::queries::list_assets(&conn, None, Some(false), None, None, None).unwrap();
    assert_eq!(unstaked.len(), 1);
}

#[test]
fn test_list_assets_combined_filters() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, is_staked, status, category) VALUES ('a', 0, 'not_started', 'Finance')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, is_staked, status, category) VALUES ('b', 0, 'finalized_owned', 'Finance')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, is_staked, status, category) VALUES ('c', 1, 'do_not_touch_staked', 'Tech')", []).unwrap();

    let found = db::queries::list_assets(&conn, Some("not_started"), Some(false), Some("Finance"), None, None).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].tld, "a");
}

#[test]
fn test_list_assets_sort_by_category() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status, category) VALUES ('a', 'not_started', 'Zebra')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status, category) VALUES ('b', 'not_started', 'Apple')", []).unwrap();

    let sorted = db::queries::list_assets(&conn, None, None, None, Some("category"), Some("asc")).unwrap();
    assert_eq!(sorted[0].category.as_deref(), Some("Apple"));
}

#[test]
fn test_update_asset_tags_json() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();

    db::queries::update_asset(&conn, assets[0].id, None, None, Some(r#"["a","b","c"]"#), None, None, None, None).unwrap();
    let asset = db::queries::get_asset(&conn, assets[0].id).unwrap();
    assert_eq!(asset.tags, vec!["a", "b", "c"]);
}

#[test]
fn test_bulk_update_empty_ids() {
    let conn = crate::tests::command_helpers::create_test_db();
    let updated = db::queries::bulk_update_status(&conn, &[], "not_started").unwrap();
    assert_eq!(updated, 0);
}

#[test]
fn test_create_batch_no_assets() {
    let conn = crate::tests::command_helpers::create_test_db();
    let id = db::queries::create_batch(&conn, "Empty", None, &[]).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, id).unwrap();
    assert_eq!(batch.assets.len(), 0);
}

#[test]
fn test_add_to_batch_duplicate() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let batch_id = db::queries::create_batch(&conn, "Test", None, &[assets[0].id]).unwrap();

    // Add same asset again - should not duplicate
    let added = db::queries::add_to_batch(&conn, batch_id, &[assets[0].id]).unwrap();
    assert_eq!(added, 0); // INSERT OR IGNORE

    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 1);
}

#[test]
fn test_wallet_snapshot_ordering() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::insert_wallet_snapshot(&conn, "primary", 100, None, 1, None).unwrap();
    db::queries::insert_wallet_snapshot(&conn, "primary", 200, None, 2, None).unwrap();
    db::queries::insert_wallet_snapshot(&conn, "primary", 300, None, 3, None).unwrap();

    let snapshots = db::queries::get_wallet_snapshots(&conn, 2).unwrap();
    assert_eq!(snapshots.len(), 2);
    // Should be ordered by id DESC (newest first)
    assert_eq!(snapshots[0]["balance"], 300);
    assert_eq!(snapshots[1]["balance"], 200);
}

#[test]
fn test_get_assets_by_tlds() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('c', 'not_started')", []).unwrap();

    let found = db::queries::get_assets_by_tlds(&conn, &["a".to_string(), "c".to_string()]).unwrap();
    assert_eq!(found.len(), 2);

    let found = db::queries::get_assets_by_tlds(&conn, &["nonexistent".to_string()]).unwrap();
    assert_eq!(found.len(), 0);
}
