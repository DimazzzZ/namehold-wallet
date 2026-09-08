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
        hsd_child: std::sync::Mutex::new(None),
        node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
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

// --- coverage: CSV deserialize error path (lines 97-99 in csv.rs) ---

/// Invalid UTF-8 bytes inside a CSV field cause a serde-level deserialize
/// error → exercises the `Err(e) => errors.push(...)` branch.
#[tokio::test]
async fn test_import_csv_deserialize_error_invalid_utf8() {
    // Header + one valid row + one row with invalid UTF-8 (0xFF byte) in the
    // name field. The csv crate's UTF-8 validation fails for the second row.
    let mut content: Vec<u8> = b"Name,Staked\ngood,false\n".to_vec();
    content.extend_from_slice(b"bad");
    content.push(0xFF);
    content.extend_from_slice(b"name,false\n");

    let dir = std::env::temp_dir().join("namehold_csv_cmd_test_imp_utf8");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.csv");
    std::fs::write(&path, &content).unwrap();
    let path_str = path.to_str().unwrap().to_string();

    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path_str).await;
    cleanup("imp_utf8");

    let res = result.unwrap();
    assert_eq!(res.imported, 1, "the good row should import");
    assert!(
        !res.errors.is_empty(),
        "the invalid-utf8 row should produce an error"
    );
    assert!(
        res.errors[0].contains("Row"),
        "error message should reference the row: {:?}",
        res.errors
    );
}

// --- coverage: empty-TLD-after-normalize skip path (lines 113-114) ---

/// A TLD that is just dots (e.g. ".") passes the `!t.trim().is_empty()` guard
/// but becomes empty after `normalize_tld` strips leading dots → triggers the
/// `tld.is_empty()` skip path.
#[tokio::test]
async fn test_import_csv_skips_dot_only_tld() {
    let csv_content = "Name,Staked\ngood,false\n.,false\n...,false\n";
    let path = write_temp_csv(csv_content, "imp_dot_tld");
    let state = setup_state();
    let app = mock_app_with(state);

    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_dot_tld");

    let res = result.unwrap();
    assert_eq!(res.imported, 1, "only 'good' should import");
    assert_eq!(
        res.skipped, 2,
        "'.' and '...' should be skipped after normalize"
    );
    assert!(res.errors.is_empty());
}

// --- coverage: INSERT error path (line 157/158) via BEFORE INSERT trigger ---

/// Install a SQLite trigger that raises an error for a specific TLD, so the
/// INSERT during import fails and hits the `Err(e) => errors.push(...)` path.
#[tokio::test]
async fn test_import_csv_reports_insert_errors() {
    let csv_content = "Name,Staked\ngood,false\nblocked_tld,false\n";
    let path = write_temp_csv(csv_content, "imp_insert_err");
    let state = setup_state();

    // Install a trigger that aborts INSERTs for the specific tld.
    {
        let db = state.db.lock().unwrap();
        db.execute_batch(
            "CREATE TRIGGER block_insert BEFORE INSERT ON assets
             FOR EACH ROW WHEN NEW.tld = 'blocked_tld'
             BEGIN
                 SELECT RAISE(ABORT, 'blocked by test trigger');
             END;",
        )
        .unwrap();
    }

    let app = mock_app_with(state);
    let result = crate::commands::csv::import_csv(app.state(), path).await;
    cleanup("imp_insert_err");

    let res = result.unwrap();
    assert_eq!(res.imported, 1, "the good row should still import");
    assert_eq!(
        res.errors.len(),
        1,
        "the blocked row should produce an error"
    );
    assert!(
        res.errors[0].contains("Row") && res.errors[0].contains("blocked"),
        "error message should reference the row and trigger message: {:?}",
        res.errors
    );
}

// --- coverage: export list_assets error propagation (line 199) ---------------

/// If the underlying `list_assets` query fails (e.g. because the table is gone),
/// `export_csv` propagates the error via `?` at line 199.
#[tokio::test]
async fn test_export_csv_propagates_query_error() {
    let out = temp_out_path("exp_query_err");
    let state = setup_state();
    // Drop the assets table so list_assets fails.
    {
        let db = state.db.lock().unwrap();
        db.execute_batch("DROP TABLE assets;").unwrap();
    }
    let app = mock_app_with(state);
    let result = crate::commands::csv::export_csv(app.state(), out, None, None, None).await;
    cleanup("exp_query_err");
    assert!(
        result.is_err(),
        "export should fail when assets table is missing"
    );
}
