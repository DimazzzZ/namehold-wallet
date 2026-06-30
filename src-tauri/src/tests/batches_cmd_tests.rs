use crate::db;

#[test]
fn test_create_batch_empty() {
    let conn = crate::tests::command_helpers::create_test_db();
    let id = db::queries::create_batch(&conn, "Test Batch", Some("desc"), &[]).unwrap();
    assert!(id > 0);

    let batches = db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].name, "Test Batch");
    assert_eq!(batches[0].description.as_deref(), Some("desc"));
}

#[test]
fn test_create_batch_with_assets() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let ids: Vec<i64> = assets.iter().map(|a| a.id).collect();

    let batch_id = db::queries::create_batch(&conn, "Migration", None, &ids).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 2);
}

#[test]
fn test_update_batch_status() {
    let conn = crate::tests::command_helpers::create_test_db();
    let id = db::queries::create_batch(&conn, "Test", None, &[]).unwrap();

    db::queries::update_batch(&conn, id, None, None, Some("in_progress")).unwrap();
    let batches = db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches[0].status.as_str(), "in_progress");
}

#[test]
fn test_update_batch_name() {
    let conn = crate::tests::command_helpers::create_test_db();
    let id = db::queries::create_batch(&conn, "Old Name", None, &[]).unwrap();

    db::queries::update_batch(&conn, id, Some("New Name"), Some("new desc"), None).unwrap();
    let batches = db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches[0].name, "New Name");
    assert_eq!(batches[0].description.as_deref(), Some("new desc"));
}

#[test]
fn test_add_to_batch() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('c', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();

    let batch_id = db::queries::create_batch(&conn, "Test", None, &[assets[0].id]).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 1);

    db::queries::add_to_batch(&conn, batch_id, &[assets[1].id, assets[2].id]).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 3);
}

#[test]
fn test_remove_from_batch() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();

    let batch_id = db::queries::create_batch(&conn, "Test", None, &assets.iter().map(|a| a.id).collect::<Vec<_>>()).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 2);

    db::queries::remove_from_batch(&conn, batch_id, &[assets[0].id]).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 1);
}

#[test]
fn test_delete_batch() {
    let conn = crate::tests::command_helpers::create_test_db();
    let id = db::queries::create_batch(&conn, "Test", None, &[]).unwrap();
    assert_eq!(db::queries::list_batches(&conn).unwrap().len(), 1);

    db::queries::delete_batch(&conn, id).unwrap();
    assert_eq!(db::queries::list_batches(&conn).unwrap().len(), 0);
}

#[test]
fn test_list_batches_empty() {
    let conn = crate::tests::command_helpers::create_test_db();
    let batches = db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches.len(), 0);
}

#[test]
fn test_batch_asset_count() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();

    db::queries::create_batch(&conn, "Test", None, &assets.iter().map(|a| a.id).collect::<Vec<_>>()).unwrap();
    let batches = db::queries::list_batches(&conn).unwrap();
    assert_eq!(batches[0].asset_count, Some(2));
}
