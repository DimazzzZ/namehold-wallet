//! Tests for the `import_csv` and `export_csv` Tauri commands.

use crate::AppState;
use std::io::Write;
use tauri::Manager;

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

fn setup_db() -> rusqlite::Connection {
    crate::tests::command_helpers::create_test_db()
}

fn setup_state() -> AppState {
    let conn = setup_db();
    AppState {
        db: std::sync::Mutex::new(conn),
        signer: std::sync::Mutex::new(None),
        secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
        hsrd_child: std::sync::Mutex::new(None),
        sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::commands::sync::SyncStatus::default(),
        )),
    }
}

fn write_temp_csv(content: &str, suffix: &str) -> String {
    let dir = std::env::temp_dir().join(format!("namehold_csv_cmd_test_{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.csv");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path.to_str().unwrap().to_string()
}

fn temp_out_path(suffix: &str) -> String {
    let dir = std::env::temp_dir().join(format!("namehold_csv_cmd_test_{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("out.csv").to_str().unwrap().to_string()
}

fn cleanup(suffix: &str) {
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("namehold_csv_cmd_test_{suffix}")),
    );
}

// --- import_csv tests ---

#[tokio::test]
async fn test_import_csv_basic() {
    let csv_content =
        "Name,Staked,Category,Notes\nalpha,true,Premium,Top\nbeta,false,Finance,Low\n";
    let path = write_temp_csv(csv_content, "imp_basic");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_basic");

    let res = result.unwrap();
    assert_eq!(res.imported, 2);
    assert_eq!(res.skipped, 0);
    assert!(res.errors.is_empty());

    // Verify assets were inserted
    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let assets = crate::db::queries::list_assets(&db, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 2);

    let alpha = assets.iter().find(|a| a.tld == "alpha").unwrap();
    assert!(alpha.is_staked);
    assert_eq!(alpha.status.as_str(), "do_not_touch_staked");

    let beta = assets.iter().find(|a| a.tld == "beta").unwrap();
    assert!(!beta.is_staked);
    assert_eq!(beta.status.as_str(), "not_started");
}

#[tokio::test]
async fn test_import_csv_skips_empty_tld() {
    let csv_content = "Name,Staked\nvalid,false\n,false\n  ,false\n";
    let path = write_temp_csv(csv_content, "imp_skip");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_skip");

    let res = result.unwrap();
    assert_eq!(res.imported, 1);
    assert_eq!(res.skipped, 2);
}

#[tokio::test]
async fn test_import_csv_handles_malformed_row() {
    let csv_content = "Name,Staked\ngood,false\n";
    let path = write_temp_csv(csv_content, "imp_malformed");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_malformed");

    let res = result.unwrap();
    assert_eq!(res.imported, 1);
    assert!(res.errors.is_empty());
}

#[tokio::test]
async fn test_import_csv_with_tags() {
    let csv_content = "Name,Tags\ntagged,\"tag1, tag2, tag3\"\n";
    let path = write_temp_csv(csv_content, "imp_tags");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_tags");

    let res = result.unwrap();
    assert_eq!(res.imported, 1);

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let assets = crate::db::queries::list_assets(&db, None, None, None, None, None).unwrap();
    let tagged = assets.iter().find(|a| a.tld == "tagged").unwrap();
    assert_eq!(tagged.tags, vec!["tag1", "tag2", "tag3"]);
}

#[tokio::test]
async fn test_import_csv_upsert_updates_existing() {
    let csv1 = "Name,Staked,Category\nupsert_test,true,Premium\n";
    let csv2 = "Name,Staked,Category\nupsert_test,false,Economy\n";
    let path1 = write_temp_csv(csv1, "imp_upsert1");
    let path2 = write_temp_csv(csv2, "imp_upsert2");
    let state = setup_state();
    let app = mock_app_with(state);

    let _ = crate::commands::csv::import_csv(app.state(), path1)
        .await
        .unwrap();
    let res2 = crate::commands::csv::import_csv(app.state(), path2)
        .await
        .unwrap();
    cleanup("imp_upsert1");
    cleanup("imp_upsert2");

    assert_eq!(res2.imported, 1);

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let assets = crate::db::queries::list_assets(&db, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 1);
    let a = &assets[0];
    assert!(!a.is_staked);
    // Status should have been downgraded from staked to not_started
    assert_eq!(a.status.as_str(), "not_started");
}

#[tokio::test]
async fn test_import_csv_with_status_column() {
    let csv_content = "Name,Status\nfinalized_one,finalized_owned\nstuck_one,failed_or_stuck\n";
    let path = write_temp_csv(csv_content, "imp_status");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_status");

    let res = result.unwrap();
    assert_eq!(res.imported, 2);

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let assets = crate::db::queries::list_assets(&db, None, None, None, None, None).unwrap();

    let fin = assets.iter().find(|a| a.tld == "finalized_one").unwrap();
    assert_eq!(fin.status.as_str(), "finalized_owned");

    let stuck = assets.iter().find(|a| a.tld == "stuck_one").unwrap();
    assert_eq!(stuck.status.as_str(), "failed_or_stuck");
}

#[tokio::test]
async fn test_import_csv_nonexistent_path_errors() {
    let state = setup_state();
    let app = mock_app_with(state);

    let result =
        crate::commands::csv::import_csv(app.state(), "/nonexistent/path.csv".into()).await;
    assert!(result.is_err());
}

// --- export_csv tests ---

#[tokio::test]
async fn test_export_csv_empty_table() {
    let out = temp_out_path("exp_empty");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::export_csv(app.state(), out.clone(), None, None, None).await;

    let count = result.unwrap();
    assert_eq!(count, 0);

    // File should exist with just the header
    let content = std::fs::read_to_string(&out).unwrap_or_default();
    cleanup("exp_empty");
    assert!(content.contains("Name"));
    assert!(content.contains("Status"));
}

#[tokio::test]
async fn test_export_csv_with_assets() {
    let csv_in =
        "Name,Staked,Category,Notes\nexport_a,true,Premium,Note A\nexport_b,false,,Note B\n";
    let in_path = write_temp_csv(csv_in, "exp_with");
    let out = temp_out_path("exp_with");
    let state = setup_state();
    let app = mock_app_with(state);

    // Import first
    let _ = crate::commands::csv::import_csv(app.state(), in_path)
        .await
        .unwrap();

    // Export all
    let result = crate::commands::csv::export_csv(app.state(), out.clone(), None, None, None).await;

    let count = result.unwrap();
    assert_eq!(count, 2);

    let content = std::fs::read_to_string(&out).unwrap_or_default();
    cleanup("exp_with");
    assert!(content.contains("export_a"));
    assert!(content.contains("export_b"));
    assert!(content.contains("true"));
}

#[tokio::test]
async fn test_export_csv_filter_by_staked() {
    let csv_in = "Name,Staked\nstaked_one,true\nunstaked_one,false\n";
    let in_path = write_temp_csv(csv_in, "exp_staked");
    let out = temp_out_path("exp_staked");
    let state = setup_state();
    let app = mock_app_with(state);

    let _ = crate::commands::csv::import_csv(app.state(), in_path)
        .await
        .unwrap();

    // Export only staked
    let result =
        crate::commands::csv::export_csv(app.state(), out.clone(), None, Some(true), None).await;

    let count = result.unwrap();
    assert_eq!(count, 1);

    let content = std::fs::read_to_string(&out).unwrap_or_default();
    cleanup("exp_staked");
    assert!(content.contains("staked_one"));
    assert!(!content.contains("unstaked_one"));
}

#[tokio::test]
async fn test_export_csv_filter_by_search() {
    let csv_in = "Name\nsearch_alpha\nsearch_beta\nother_gamma\n";
    let in_path = write_temp_csv(csv_in, "exp_search");
    let out = temp_out_path("exp_search");
    let state = setup_state();
    let app = mock_app_with(state);

    let _ = crate::commands::csv::import_csv(app.state(), in_path)
        .await
        .unwrap();

    // Export only matching "search"
    let result = crate::commands::csv::export_csv(
        app.state(),
        out.clone(),
        None,
        None,
        Some("search".into()),
    )
    .await;

    let count = result.unwrap();
    assert_eq!(count, 2);

    let content = std::fs::read_to_string(&out).unwrap_or_default();
    cleanup("exp_search");
    assert!(content.contains("search_alpha"));
    assert!(content.contains("search_beta"));
    assert!(!content.contains("other_gamma"));
}

#[tokio::test]
async fn test_export_csv_filter_by_status() {
    let csv_in = "Name,Status\nstatus_fin,finalized_owned\nstatus_not,not_started\n";
    let in_path = write_temp_csv(csv_in, "exp_status_filter");
    let out = temp_out_path("exp_status_filter");
    let state = setup_state();
    let app = mock_app_with(state);

    let _ = crate::commands::csv::import_csv(app.state(), in_path)
        .await
        .unwrap();

    // Export only finalized_owned
    let result = crate::commands::csv::export_csv(
        app.state(),
        out.clone(),
        Some("finalized_owned".into()),
        None,
        None,
    )
    .await;

    let count = result.unwrap();
    assert_eq!(count, 1);

    let content = std::fs::read_to_string(&out).unwrap_or_default();
    cleanup("exp_status_filter");
    assert!(content.contains("status_fin"));
    assert!(!content.contains("status_not"));
}
