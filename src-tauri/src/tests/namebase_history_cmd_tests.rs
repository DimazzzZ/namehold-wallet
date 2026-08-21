//! Tests for `commands::namebase_history` — file-import, query, summary, and clear.
//! The live-import path (`import_namebase_history_live`) is skipped because it
//! needs a real Namebase session.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::namebase_history::{
    clear_namebase_history, get_namebase_history, get_namebase_history_summary,
    import_namebase_history_from_file,
};
use crate::db;
use crate::AppState;

/// RAII guard that removes a file on drop.
struct TmpFile(String);
impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn migrated_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(migrated_conn()),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// Minimal valid Namebase export CSV with one place-bid event.
fn sample_csv() -> String {
    [
        r#""This export covers your Namebase account history only.""#,
        r#""""#,
        r#""It does NOT include Sunset activity.""#,
        "",
        "id,created_at,type,data",
        r#"100,2024-06-01T10:00:00.000Z,auctions:place-bid:4,"{""domainName"":""testname"",""auctionId"":""aaa-bbb"",""custodian"":""us"",""bidAmountString"":""5000000"",""stakeAmountString"":""10000000"",""prepaidFeeString"":""100000""}""#,
        r#"101,2024-06-02T11:00:00.000Z,auctions:reveal-bid:3,"{""domainName"":""testname"",""auctionId"":""aaa-bbb""}""#,
    ]
    .join("\n")
}

fn write_tmp_csv(label: &str, content: &str) -> String {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "namehold_nbhist_{}_{}_{}.csv",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    path.push(unique);
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn import_from_file_inserts_events() {
    let path = write_tmp_csv("import", &sample_csv());
    let _cleanup = TmpFile(path.clone());

    let app = app();
    let result = import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();
    assert_eq!(result.total, 2);
    assert_eq!(result.inserted, 2);
    assert_eq!(result.updated, 0);
}

#[tokio::test]
async fn import_is_idempotent() {
    let path = write_tmp_csv("idem", &sample_csv());
    let _cleanup = TmpFile(path.clone());

    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();
    let second = import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();
    // Same IDs → updated, not inserted.
    assert_eq!(second.inserted, 0);
    assert_eq!(second.updated, 2);
}

#[tokio::test]
async fn get_history_returns_imported_rows() {
    let path = write_tmp_csv("get", &sample_csv());
    let _cleanup = TmpFile(path.clone());

    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let rows = get_namebase_history(app.state(), None, None, None).unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn get_history_filters_by_name() {
    let path = write_tmp_csv("filter", &sample_csv());
    let _cleanup = TmpFile(path.clone());

    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let rows = get_namebase_history(app.state(), Some("testname".into()), None, None).unwrap();
    assert_eq!(rows.len(), 2);

    let rows = get_namebase_history(app.state(), Some("nonexistent".into()), None, None).unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn summary_reflects_imported_data() {
    let path = write_tmp_csv("summary", &sample_csv());
    let _cleanup = TmpFile(path.clone());

    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let summary = get_namebase_history_summary(app.state()).unwrap();
    assert_eq!(summary.event_count, 2);
    assert_eq!(summary.name_count, 1); // both events reference "testname"
    assert!(summary.earliest.is_some());
    assert!(summary.latest.is_some());
}

#[tokio::test]
async fn clear_removes_all_events() {
    let path = write_tmp_csv("clear", &sample_csv());
    let _cleanup = TmpFile(path.clone());

    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let removed = clear_namebase_history(app.state()).unwrap();
    assert_eq!(removed, 2);

    let rows = get_namebase_history(app.state(), None, None, None).unwrap();
    assert!(rows.is_empty());

    let summary = get_namebase_history_summary(app.state()).unwrap();
    assert_eq!(summary.event_count, 0);
}

#[test]
fn summary_on_empty_db() {
    let app = app();
    let summary = get_namebase_history_summary(app.state()).unwrap();
    assert_eq!(summary.event_count, 0);
    assert_eq!(summary.name_count, 0);
    assert!(summary.earliest.is_none());
    assert!(summary.latest.is_none());
}
