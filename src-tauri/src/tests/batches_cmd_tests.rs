use crate::db;
use crate::commands;
use crate::AppState;
use crate::tests::command_helpers::create_test_state;
use tauri::Manager;

// ── DB-query–layer tests (unchanged from existing) ──────────────────────

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

// ── Command-layer tests (cover src-tauri/src/commands/batches.rs) ───────

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

#[tokio::test]
async fn test_cmd_list_batches_empty() {
    let state = create_test_state();
    let app = mock_app_with(state);

    let batches = commands::batches::list_batches(app.state::<AppState>()).await.unwrap();
    assert!(batches.is_empty());
}

#[tokio::test]
async fn test_cmd_create_batch_and_list() {
    let state = create_test_state();
    let app = mock_app_with(state);

    let id = commands::batches::create_batch(
        app.state::<AppState>(),
        "My Batch".to_string(),
        Some("A test batch".to_string()),
        vec![],
    )
    .await
    .unwrap();
    assert!(id > 0);

    let batches = commands::batches::list_batches(app.state::<AppState>()).await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].name, "My Batch");
    assert_eq!(batches[0].description.as_deref(), Some("A test batch"));
}

#[tokio::test]
async fn test_cmd_get_batch_with_assets() {
    let state = create_test_state();
    let asset_ids: Vec<i64>;
    {
        let db = state.db.lock().unwrap();
        db.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
        db.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
        let assets = db::queries::list_assets(&db, None, None, None, None, None).unwrap();
        asset_ids = assets.iter().map(|a| a.id).collect();
    }

    let app = mock_app_with(state);

    let batch_id = commands::batches::create_batch(
        app.state::<AppState>(), "WithAssets".to_string(), None, asset_ids,
    )
    .await
    .unwrap();

    let batch = commands::batches::get_batch_with_assets(app.state::<AppState>(), batch_id).await.unwrap();
    assert_eq!(batch.assets.len(), 2);
}

#[tokio::test]
async fn test_cmd_update_batch() {
    let state = create_test_state();
    let app = mock_app_with(state);

    let id = commands::batches::create_batch(
        app.state::<AppState>(), "Old".to_string(), None, vec![],
    )
    .await
    .unwrap();

    commands::batches::update_batch(
        app.state::<AppState>(),
        id,
        Some("New".to_string()),
        Some("Updated desc".to_string()),
        Some("in_progress".to_string()),
    )
    .await
    .unwrap();

    let batches = commands::batches::list_batches(app.state::<AppState>()).await.unwrap();
    assert_eq!(batches[0].name, "New");
    assert_eq!(batches[0].description.as_deref(), Some("Updated desc"));
    assert_eq!(batches[0].status.as_str(), "in_progress");
}

#[tokio::test]
async fn test_cmd_delete_batch() {
    let state = create_test_state();
    let app = mock_app_with(state);

    let id = commands::batches::create_batch(
        app.state::<AppState>(), "DeleteMe".to_string(), None, vec![],
    )
    .await
    .unwrap();

    commands::batches::delete_batch(app.state::<AppState>(), id).await.unwrap();

    let batches = commands::batches::list_batches(app.state::<AppState>()).await.unwrap();
    assert!(batches.is_empty());
}

#[tokio::test]
async fn test_cmd_add_to_batch() {
    let state = create_test_state();
    let asset_ids: Vec<i64>;
    {
        let db = state.db.lock().unwrap();
        db.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
        db.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
        let assets = db::queries::list_assets(&db, None, None, None, None, None).unwrap();
        asset_ids = assets.iter().map(|a| a.id).collect();
    }

    let app = mock_app_with(state);

    let batch_id = commands::batches::create_batch(
        app.state::<AppState>(), "AddTest".to_string(), None, vec![asset_ids[0]],
    )
    .await
    .unwrap();

    let added = commands::batches::add_to_batch(
        app.state::<AppState>(), batch_id, vec![asset_ids[1]],
    )
    .await
    .unwrap();
    assert_eq!(added, 1);

    let batch = commands::batches::get_batch_with_assets(app.state::<AppState>(), batch_id).await.unwrap();
    assert_eq!(batch.assets.len(), 2);
}

#[tokio::test]
async fn test_cmd_remove_from_batch() {
    let state = create_test_state();
    let asset_ids: Vec<i64>;
    {
        let db = state.db.lock().unwrap();
        db.execute("INSERT INTO assets (tld, status) VALUES ('a', 'not_started')", []).unwrap();
        db.execute("INSERT INTO assets (tld, status) VALUES ('b', 'not_started')", []).unwrap();
        let assets = db::queries::list_assets(&db, None, None, None, None, None).unwrap();
        asset_ids = assets.iter().map(|a| a.id).collect();
    }

    let app = mock_app_with(state);

    let batch_id = commands::batches::create_batch(
        app.state::<AppState>(), "RemoveTest".to_string(), None, asset_ids.clone(),
    )
    .await
    .unwrap();

    let removed = commands::batches::remove_from_batch(
        app.state::<AppState>(), batch_id, vec![asset_ids[0]],
    )
    .await
    .unwrap();
    assert_eq!(removed, 1);

    let batch = commands::batches::get_batch_with_assets(app.state::<AppState>(), batch_id).await.unwrap();
    assert_eq!(batch.assets.len(), 1);
}
