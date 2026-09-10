//! Tests for `commands::namebase_history` — file-import, query, summary, and clear.
//! The live-import path (`import_namebase_history_live`) is skipped because it
//! needs a real Namebase session.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::namebase_history::{
    clear_namebase_history, get_namebase_history, get_namebase_history_summary,
    import_namebase_history_from_file, import_namebase_history_live,
};
use crate::db;
use crate::error::AppError;
use crate::AppState;

// ---------------------------------------------------------------------------
// Live-import helpers (copied/adapted from namebase_cmd_tests.rs; the guidance
// says COPY rather than refactor into a shared module).
// ---------------------------------------------------------------------------

const PROFILE: &str = "nbhistp1";
const COOKIE: &str = "test-cookie-123";

/// Install a fixed test DEK so the encrypt/decrypt cookie flow does not touch
/// the OS keyring. Idempotent; called from every seeded-conn helper.
fn install_test_dek() {
    crate::noncustodial::cookie_vault::set_test_dek(Some((0..32u8).collect()));
}

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
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

/// In-memory DB with an active MAINNET profile plus `namebase_cookie` and
/// `namebase_base_url` set (the test seam honored by `namebase_client`).
fn seeded_conn_with_namebase(base_url: &str) -> rusqlite::Connection {
    install_test_dek();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "NB",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "namebase_cookie", COOKIE).unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", base_url).unwrap();
    conn
}

/// Same as above but WITHOUT a cookie set → `namebase_client()` should error.
fn seeded_conn_no_cookie(base_url: &str) -> rusqlite::Connection {
    install_test_dek();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "NB",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", base_url).unwrap();
    conn
}

/// Count audit-log rows matching a given action.
fn audit_count(state: &AppState, action: &str) -> i64 {
    let db = state.db.lock().unwrap();
    db.query_row(
        "SELECT COUNT(*) FROM audit_log WHERE action = ?1",
        [action],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count imported history rows.
fn history_count(state: &AppState) -> i64 {
    let db = state.db.lock().unwrap();
    db.query_row("SELECT COUNT(*) FROM namebase_history", [], |r| r.get(0))
        .unwrap()
}

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

// =========================================================================
// Priority 1: import_namebase_history_live via mockito
// =========================================================================

#[tokio::test]
async fn live_import_happy_path() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account/history/export")
        .with_status(200)
        .with_body(sample_csv())
        .create_async()
        .await;

    let app = app_with(seeded_conn_with_namebase(&server.url()));
    let result = import_namebase_history_live(app.state())
        .await
        .expect("live import should succeed");

    assert_eq!(result.total, 2);
    assert_eq!(result.inserted, 2);
    assert_eq!(result.updated, 0);

    let state = app.state::<AppState>();
    assert_eq!(audit_count(&state, "namebase_history_import_live"), 1);
    m.assert_async().await;
}

#[tokio::test]
async fn live_import_is_idempotent() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account/history/export")
        .with_status(200)
        .with_body(sample_csv())
        .expect(2)
        .create_async()
        .await;

    let app = app_with(seeded_conn_with_namebase(&server.url()));
    import_namebase_history_live(app.state())
        .await
        .expect("first live import");
    let second = import_namebase_history_live(app.state())
        .await
        .expect("second live import");

    assert_eq!(second.inserted, 0);
    assert_eq!(second.updated, 2);
}

#[tokio::test]
async fn live_import_rotates_cookie() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account/history/export")
        .with_status(200)
        .with_header("set-cookie", "nb-sunset=NEW")
        .with_body(sample_csv())
        .create_async()
        .await;

    let conn = seeded_conn_with_namebase(&server.url());
    db::queries::set_setting(&conn, "namebase_cookie", "nb-sunset=OLD").unwrap();
    let app = app_with(conn);

    import_namebase_history_live(app.state())
        .await
        .expect("live import should succeed");

    let state = app.state::<AppState>();
    {
        let db = state.db.lock().unwrap();
        let settings = db::queries::get_settings(&db).unwrap();
        let v1 = settings
            .get("namebase_cookie_v1")
            .cloned()
            .unwrap_or_default();
        assert!(!v1.is_empty(), "encrypted v1 blob should be present");
        let decrypted = crate::noncustodial::cookie_vault::decrypt_cookie(&v1).expect("decrypt");
        assert_eq!(String::from_utf8_lossy(&decrypted), "nb-sunset=NEW");
    }
    m.assert_async().await;
}

#[tokio::test]
async fn live_import_http_error_propagates() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account/history/export")
        .with_status(500)
        .with_body("boom")
        .create_async()
        .await;

    let app = app_with(seeded_conn_with_namebase(&server.url()));
    let err = import_namebase_history_live(app.state()).await;
    assert!(err.is_err(), "500 should propagate as Err");

    let state = app.state::<AppState>();
    assert_eq!(history_count(&state), 0, "no rows on error");
    assert_eq!(
        audit_count(&state, "namebase_history_import_live"),
        0,
        "no audit row on error"
    );
}

#[tokio::test]
async fn live_import_csv_parse_error_propagates() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account/history/export")
        .with_status(200)
        .with_body("garbage,not,valid\n")
        .create_async()
        .await;

    let app = app_with(seeded_conn_with_namebase(&server.url()));
    let err = import_namebase_history_live(app.state()).await;
    assert!(err.is_err(), "malformed CSV should propagate as Err");

    let state = app.state::<AppState>();
    assert_eq!(history_count(&state), 0);
    assert_eq!(audit_count(&state, "namebase_history_import_live"), 0);
}

#[tokio::test]
async fn live_import_without_cookie_errors() {
    let server = mockito::Server::new_async().await;
    // No mock registered — client build should fail before any HTTP call.
    let app = app_with(seeded_conn_no_cookie(&server.url()));
    let err = import_namebase_history_live(app.state()).await;
    assert!(
        err.is_err(),
        "missing cookie should error from namebase_client"
    );
}

// =========================================================================
// Priority 2: get_namebase_history family / search filter branches
// =========================================================================

#[tokio::test]
async fn get_history_family_filter_matches() {
    let path = write_tmp_csv("fam_match", &sample_csv());
    let _cleanup = TmpFile(path.clone());
    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    // Both sample rows are `auctions:*` events → family = "auctions".
    let rows = get_namebase_history(app.state(), None, Some("auctions".into()), None).unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn get_history_family_filter_excludes() {
    let path = write_tmp_csv("fam_excl", &sample_csv());
    let _cleanup = TmpFile(path.clone());
    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let rows = get_namebase_history(app.state(), None, Some("nonexistent".into()), None).unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn get_history_search_filter_matches() {
    let path = write_tmp_csv("search_match", &sample_csv());
    let _cleanup = TmpFile(path.clone());
    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let rows = get_namebase_history(app.state(), None, None, Some("testname".into())).unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn get_history_search_filter_excludes() {
    let path = write_tmp_csv("search_excl", &sample_csv());
    let _cleanup = TmpFile(path.clone());
    let app = app();
    import_namebase_history_from_file(app.state(), path.clone())
        .await
        .unwrap();

    let rows = get_namebase_history(app.state(), None, None, Some("zzz".into())).unwrap();
    assert!(rows.is_empty());
}

// =========================================================================
// Priority 3: import_namebase_history_from_file error branches
// =========================================================================

#[tokio::test]
async fn import_from_file_missing_path_errors() {
    let app = app();
    let err =
        import_namebase_history_from_file(app.state(), "/nonexistent/does-not-exist.csv".into())
            .await;
    assert!(
        matches!(err, Err(AppError::Io(_))),
        "expected Io error, got {err:?}"
    );
}

#[tokio::test]
async fn import_from_file_malformed_csv_errors() {
    let path = write_tmp_csv("bad", "garbage,not,valid\n");
    let _cleanup = TmpFile(path.clone());
    let app = app();
    let err = import_namebase_history_from_file(app.state(), path.clone()).await;
    assert!(err.is_err(), "malformed CSV should error");

    let state = app.state::<AppState>();
    assert_eq!(audit_count(&state, "namebase_history_import_file"), 0);
}
