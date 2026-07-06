use crate::db;
use crate::commands;
use crate::AppState;
use crate::tests::command_helpers::create_test_state;
use tauri::Manager;

// ── DB-query–layer tests (unchanged from existing) ──────────────────────

#[test]
fn test_get_settings_returns_defaults() {
    let conn = crate::tests::command_helpers::create_test_db();
    let settings = db::queries::get_settings(&conn).unwrap();

    assert_eq!(settings["hsd_wallet_api_url"], "http://127.0.0.1:12039");
    assert_eq!(settings["hsd_node_api_url"], "http://127.0.0.1:12037");
    assert_eq!(settings["hsd_wallet_id"], "primary");
    assert_eq!(settings["hsd_network"], "mainnet");
    assert_eq!(settings["write_mode"], "false");
}

#[test]
fn test_set_setting_new_key() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::set_setting(&conn, "custom_key", "custom_value").unwrap();

    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["custom_key"], "custom_value");
}

#[test]
fn test_set_setting_update_existing() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::set_setting(&conn, "hsd_network", "testnet").unwrap();

    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["hsd_network"], "testnet");
}

#[test]
fn test_set_setting_empty_value() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::set_setting(&conn, "hsd_api_key", "").unwrap();

    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["hsd_api_key"], "");
}

#[test]
fn test_wallet_snapshot_operations() {
    let conn = crate::tests::command_helpers::create_test_db();

    // Insert snapshots
    let id1 = db::queries::insert_wallet_snapshot(&conn, "primary", 1000000, Some("rs1q1"), 5, None).unwrap();
    let id2 = db::queries::insert_wallet_snapshot(&conn, "primary", 2000000, Some("rs1q1"), 10, None).unwrap();
    assert!(id2 > id1);

    // Get latest
    let latest = db::queries::get_latest_wallet_snapshot(&conn).unwrap().unwrap();
    assert_eq!(latest["balance"], 2000000);
    assert_eq!(latest["name_count"], 10);

    // Get list
    let snapshots = db::queries::get_wallet_snapshots(&conn, 5).unwrap();
    assert_eq!(snapshots.len(), 2);
}

#[test]
fn test_audit_log_operations() {
    let conn = crate::tests::command_helpers::create_test_db();

    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('import_csv', '{\"count\":5}')", []).unwrap();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('sync', '{\"matched\":3}')", []).unwrap();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('import_csv', '{\"count\":10}')", []).unwrap();

    let entries = db::queries::get_recent_audit_log(&conn, 10).unwrap();
    assert_eq!(entries.len(), 3);

    let entries = db::queries::get_recent_audit_log(&conn, 2).unwrap();
    assert_eq!(entries.len(), 2);
}

// ── Command-layer tests (cover src-tauri/src/commands/settings.rs) ──────

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

#[tokio::test]
async fn test_cmd_get_settings() {
    let state = create_test_state();
    let app = mock_app_with(state);

    let settings = commands::settings::get_settings(app.state::<AppState>()).await.unwrap();
    assert_eq!(settings["hsd_wallet_api_url"], "http://127.0.0.1:12039");
    assert_eq!(settings["hsd_network"], "mainnet");
}

#[tokio::test]
async fn test_cmd_update_setting() {
    let state = create_test_state();
    let app = mock_app_with(state);

    commands::settings::update_setting(
        app.state::<AppState>(),
        "hsd_network".to_string(),
        "testnet".to_string(),
    )
    .await
    .unwrap();

    let settings = commands::settings::get_settings(app.state::<AppState>()).await.unwrap();
    assert_eq!(settings["hsd_network"], "testnet");
}

#[tokio::test]
async fn test_cmd_update_setting_custom_key() {
    let state = create_test_state();
    let app = mock_app_with(state);

    commands::settings::update_setting(
        app.state::<AppState>(),
        "theme".to_string(),
        "dark".to_string(),
    )
    .await
    .unwrap();

    let settings = commands::settings::get_settings(app.state::<AppState>()).await.unwrap();
    assert_eq!(settings["theme"], "dark");
}

#[tokio::test]
async fn test_cmd_get_audit_log() {
    let state = create_test_state();
    // Seed audit log entries
    {
        let db = state.db.lock().unwrap();
        db.execute("INSERT INTO audit_log (action, detail) VALUES ('test_cmd', '{\"msg\":\"hello\"}')", []).unwrap();
    }

    let app = mock_app_with(state);

    let entries = commands::settings::get_audit_log(app.state::<AppState>(), None).await.unwrap();
    let arr = entries.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["action"], "test_cmd");
}

#[tokio::test]
async fn test_cmd_get_audit_log_with_limit() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db.execute("INSERT INTO audit_log (action, detail) VALUES ('a', '{}')", []).unwrap();
        db.execute("INSERT INTO audit_log (action, detail) VALUES ('b', '{}')", []).unwrap();
    }

    let app = mock_app_with(state);

    let entries = commands::settings::get_audit_log(app.state::<AppState>(), Some(1)).await.unwrap();
    let arr = entries.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[tokio::test]
async fn test_cmd_get_wallet_snapshots() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::insert_wallet_snapshot(&db, "wallet1", 1000000, None, 5, None).unwrap();
        db::queries::insert_wallet_snapshot(&db, "wallet1", 2000000, None, 8, None).unwrap();
    }

    let app = mock_app_with(state);

    let snapshots = commands::settings::get_wallet_snapshots(app.state::<AppState>(), None).await.unwrap();
    let arr = snapshots.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[tokio::test]
async fn test_cmd_get_wallet_snapshots_with_limit() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::insert_wallet_snapshot(&db, "w", 1, None, 1, None).unwrap();
        db::queries::insert_wallet_snapshot(&db, "w", 2, None, 2, None).unwrap();
        db::queries::insert_wallet_snapshot(&db, "w", 3, None, 3, None).unwrap();
    }

    let app = mock_app_with(state);

    let snapshots = commands::settings::get_wallet_snapshots(app.state::<AppState>(), Some(2)).await.unwrap();
    let arr = snapshots.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}
