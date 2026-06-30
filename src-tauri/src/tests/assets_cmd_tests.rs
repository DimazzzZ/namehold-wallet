use crate::db;

#[test]
fn test_list_assets_empty() {
    let conn = crate::tests::command_helpers::create_test_db();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 0);
}

#[test]
fn test_list_assets_with_data() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status, is_staked) VALUES ('crypto', 'not_started', 0)", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status, is_staked) VALUES ('wallet', 'finalized_owned', 0)", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status, is_staked) VALUES ('defi', 'do_not_touch_staked', 1)", []).unwrap();

    let all = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(all.len(), 3);

    let unstaked = db::queries::list_assets(&conn, None, Some(false), None, None, None).unwrap();
    assert_eq!(unstaked.len(), 2);

    let staked = db::queries::list_assets(&conn, None, Some(true), None, None, None).unwrap();
    assert_eq!(staked.len(), 1);

    let by_status = db::queries::list_assets(&conn, Some("finalized_owned"), None, None, None, None).unwrap();
    assert_eq!(by_status.len(), 1);
    assert_eq!(by_status[0].tld, "wallet");
}

#[test]
fn test_list_assets_search() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status, category, notes) VALUES ('crypto', 'not_started', 'Finance', 'test note')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status, category) VALUES ('wallet', 'not_started', 'Tech')", []).unwrap();

    let found = db::queries::list_assets(&conn, None, None, Some("Finance"), None, None).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].tld, "crypto");

    let found = db::queries::list_assets(&conn, None, None, Some("test note"), None, None).unwrap();
    assert_eq!(found.len(), 1);

    let found = db::queries::list_assets(&conn, None, None, Some("nonexistent"), None, None).unwrap();
    assert_eq!(found.len(), 0);
}

#[test]
fn test_list_assets_sorting() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('zebra', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('apple', 'not_started')", []).unwrap();

    let sorted = db::queries::list_assets(&conn, None, None, None, Some("tld"), Some("asc")).unwrap();
    assert_eq!(sorted[0].tld, "apple");
    assert_eq!(sorted[1].tld, "zebra");

    let sorted = db::queries::list_assets(&conn, None, None, None, Some("tld"), Some("desc")).unwrap();
    assert_eq!(sorted[0].tld, "zebra");
    assert_eq!(sorted[1].tld, "apple");
}

#[test]
fn test_get_asset() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let id = assets[0].id;

    let asset = db::queries::get_asset(&conn, id).unwrap();
    assert_eq!(asset.tld, "test");
    assert_eq!(asset.status.as_str(), "not_started");
}

#[test]
fn test_update_asset_multiple_fields() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let id = assets[0].id;

    db::queries::update_asset(&conn, id, Some("finalized_owned"), Some("Premium"), None, Some("my note"), Some(1000000), Some("tx123"), Some("tx456")).unwrap();

    let asset = db::queries::get_asset(&conn, id).unwrap();
    assert_eq!(asset.status.as_str(), "finalized_owned");
    assert_eq!(asset.category.as_deref(), Some("Premium"));
    assert_eq!(asset.notes.as_deref(), Some("my note"));
    assert_eq!(asset.hns_received, Some(1000000));
    assert_eq!(asset.transfer_tx_hash.as_deref(), Some("tx123"));
    assert_eq!(asset.finalize_tx_hash.as_deref(), Some("tx456"));
}

#[test]
fn test_delete_asset() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let id = assets[0].id;

    db::queries::delete_asset(&conn, id).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 0);
}

#[test]
fn test_bulk_update_status_multiple() {
    let conn = crate::tests::command_helpers::create_test_db();
    for i in 0..5 {
        conn.execute("INSERT INTO assets (tld, status) VALUES (?1, 'not_started')", [format!("tld{}", i)]).unwrap();
    }
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();

    let updated = db::queries::bulk_update_status(&conn, &ids, "waiting_transfer_tx").unwrap();
    assert_eq!(updated, 5);

    let assets = db::queries::list_assets(&conn, Some("waiting_transfer_tx"), None, None, None, None).unwrap();
    assert_eq!(assets.len(), 5);
}

#[test]
fn test_bulk_update_tags() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('test', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();

    db::queries::bulk_update_tags(&conn, &[assets[0].id], r#"["high_value","premium"]"#).unwrap();

    let asset = db::queries::get_asset(&conn, assets[0].id).unwrap();
    assert_eq!(asset.tags, vec!["high_value", "premium"]);
}
