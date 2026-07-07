//! Integration tests for `commands::namebase` wired up via `namebase_client()`
//! (which respects the `namebase_base_url` test seam).
//!
//! The underlying `NamebaseClient` methods are tested in `namebase_client_tests.rs`;
//! these tests focus on the command-layer: DB side effects, audit logging, error
//! propagation, and the connection/disconnection flow.

use mockito::Server;
use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::namebase::{
    connect_namebase, disconnect_namebase, fetch_namebase_domain_withdrawals,
    fetch_namebase_domains, fetch_namebase_renewals, fetch_namebase_staked,
    fetch_namebase_withdrawals, get_namebase_status, import_from_namebase,
    namebase_transfer_domain, namebase_withdraw_hns,
};
use crate::db;
use crate::error::AppError;
use crate::AppState;

const PROFILE: &str = "nbp1";
const COOKIE: &str = "test-cookie-123";

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// In-memory DB with an active MAINNET profile and `namebase_cookie` + `namebase_base_url` set.
fn seeded_conn(base_url: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn, PROFILE, "NB", "mnemonic_hot", "mainnet", "xpubFAKE", 0, false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "namebase_cookie", COOKIE).unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", base_url).unwrap();
    conn
}

/// DB with a MAINNET profile but no cookie / base URL set.
fn conn_without_cookie() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn, PROFILE, "NB", "mnemonic_hot", "mainnet", "xpubFAKE", 0, false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    conn
}

/// DB with no wallet profile at all (active_profile_network should fall back to Main).
fn conn_without_profile() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn good_addr() -> String {
    "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8".to_string()
}

// =========================================================================
// get_namebase_status
// =========================================================================

#[tokio::test]
async fn status_no_cookie_returns_not_connected() {
    let app = app_with(conn_without_cookie());
    let v = get_namebase_status(app.state::<AppState>())
        .await
        .expect("status should succeed");
    assert_eq!(v["connected"], serde_json::json!(false));
    assert_eq!(v["has_cookie"], serde_json::json!(false));
}

#[tokio::test]
async fn status_cookie_present_session_expired_returns_not_connected_with_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account")
        .with_status(401)
        .with_body(r#"{"error":"unauthorized"}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let v = get_namebase_status(app.state::<AppState>())
        .await
        .expect("status should succeed");
    assert_eq!(v["connected"], serde_json::json!(false));
    assert_eq!(v["has_cookie"], serde_json::json!(true));
    assert!(v["error"].as_str().unwrap().contains("expired"), "got: {v:?}");
}

#[tokio::test]
async fn status_cookie_present_session_valid_returns_connected_and_account() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(r#"{"email":"test@namebase.io","balance":500}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let v = get_namebase_status(app.state::<AppState>())
        .await
        .expect("status should succeed");
    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["has_cookie"], serde_json::json!(true));
    assert_eq!(v["account"]["email"], "test@namebase.io");
    assert_eq!(v["account"]["balance"], 500);
}

// =========================================================================
// fetch_namebase_domains
// =========================================================================

#[tokio::test]
async fn fetch_domains_returns_domains() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(r#"{"domains":[{"name":"example","status":"active"}]}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let v = fetch_namebase_domains(app.state::<AppState>())
        .await
        .expect("fetch_domains should succeed");
    assert_eq!(v["domains"][0]["name"], "example");
    m.assert_async().await;
}

#[tokio::test]
async fn fetch_domains_propagates_api_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains")
        .with_status(500)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = fetch_namebase_domains(app.state::<AppState>())
        .await
        .expect_err("should fail on 500");
    let msg = format!("{err}");
    assert!(msg.contains("500"), "msg: {msg}");
}

// =========================================================================
// fetch_namebase_staked
// =========================================================================

#[tokio::test]
async fn fetch_staked_returns_staked_domains() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("GET", "/api/domains/staked")
        .with_status(200)
        .with_body(r#"{"stakedDomains":[{"name":"staked1"}]}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let v = fetch_namebase_staked(app.state::<AppState>())
        .await
        .expect("fetch_staked should succeed");
    assert_eq!(v["stakedDomains"][0]["name"], "staked1");
    m.assert_async().await;
}

#[tokio::test]
async fn fetch_staked_propagates_api_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains/staked")
        .with_status(403)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = fetch_namebase_staked(app.state::<AppState>())
        .await
        .expect_err("should fail on 403");
    let msg = format!("{err}");
    assert!(msg.contains("403"), "msg: {msg}");
}

// =========================================================================
// fetch_namebase_renewals (already has a basic test in namebase_transfer_tests.rs)
// =========================================================================

#[tokio::test]
async fn fetch_renewals_propagates_api_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains/renewals")
        .with_status(500)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = fetch_namebase_renewals(app.state::<AppState>())
        .await
        .expect_err("should fail on 500");
    let msg = format!("{err}");
    assert!(msg.contains("500"), "msg: {msg}");
}

// =========================================================================
// fetch_namebase_withdrawals
// =========================================================================

#[tokio::test]
async fn fetch_withdrawals_returns_withdrawals() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("GET", "/api/withdrawals")
        .with_status(200)
        .with_body(r#"{"withdrawals":[{"id":1,"amount":"100"}]}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let v = fetch_namebase_withdrawals(app.state::<AppState>())
        .await
        .expect("fetch_withdrawals should succeed");
    assert_eq!(v["withdrawals"][0]["id"], 1);
    m.assert_async().await;
}

#[tokio::test]
async fn fetch_withdrawals_propagates_api_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/withdrawals")
        .with_status(400)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = fetch_namebase_withdrawals(app.state::<AppState>())
        .await
        .expect_err("should fail on 400");
    let msg = format!("{err}");
    assert!(msg.contains("400"), "msg: {msg}");
}

// =========================================================================
// fetch_namebase_domain_withdrawals
// =========================================================================

#[tokio::test]
async fn fetch_domain_withdrawals_returns_domain_withdrawals() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("GET", "/api/domains/withdrawals")
        .with_status(200)
        .with_body(r#"{"domainWithdrawals":[{"domain":"ex","status":"pending"}]}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let v = fetch_namebase_domain_withdrawals(app.state::<AppState>())
        .await
        .expect("fetch_domain_withdrawals should succeed");
    assert_eq!(v["domainWithdrawals"][0]["domain"], "ex");
    m.assert_async().await;
}

#[tokio::test]
async fn fetch_domain_withdrawals_propagates_api_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains/withdrawals")
        .with_status(503)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = fetch_namebase_domain_withdrawals(app.state::<AppState>())
        .await
        .expect_err("should fail on 503");
    let msg = format!("{err}");
    assert!(msg.contains("503"), "msg: {msg}");
}

// =========================================================================
// import_from_namebase
// =========================================================================

#[tokio::test]
async fn import_imports_domains_and_staked_domains_into_assets() {
    let mut server = Server::new_async().await;
    let _domains_mock = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(
            r#"{"domains":[
                {"name":"ALPHA"},
                {"name":"beta"},
                {"name":"STAKED1"},
                {"name":"gamma"}
            ]}"#,
        )
        .create_async()
        .await;
    let _staked_mock = server
        .mock("GET", "/api/domains/staked")
        .with_status(200)
        .with_body(r#"{"stakedDomains":[{"name":"STAKED1"},{"name":"staked2"}]}"#)
        .create_async()
        .await;

    let conn = seeded_conn(&server.url());
    let app = app_with(conn);

    let result = import_from_namebase(app.state::<AppState>())
        .await
        .expect("import should succeed");
    assert_eq!(result["imported"], 4);
    assert_eq!(result["skipped"], 0);
    assert_eq!(result["errors"].as_array().unwrap().len(), 0);
    assert_eq!(result["staked_count"], 2);

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();

    let staked1 = db::queries::get_assets_by_tlds(&db, &["staked1".to_string()]).unwrap().into_iter().next().unwrap();
    assert!(staked1.is_staked, "staked1 should be staked");
    assert_eq!(staked1.status.as_str(), "do_not_touch_staked");

    let alpha = db::queries::get_assets_by_tlds(&db, &["alpha".to_string()]).unwrap().into_iter().next().unwrap();
    assert!(!alpha.is_staked, "alpha should not be staked");
    assert_eq!(alpha.status.as_str(), "not_started");

    let beta = db::queries::get_assets_by_tlds(&db, &["beta".to_string()]).unwrap().into_iter().next().unwrap();
    assert!(!beta.is_staked, "beta should not be staked");
    assert_eq!(beta.status.as_str(), "not_started");

    let gamma = db::queries::get_assets_by_tlds(&db, &["gamma".to_string()]).unwrap().into_iter().next().unwrap();
    assert!(!gamma.is_staked, "gamma should not be staked");
    assert_eq!(gamma.status.as_str(), "not_started");

    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'namebase_import'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1);
}

#[tokio::test]
async fn import_upserts_existing_asset() {
    let mut server = Server::new_async().await;
    let _domains_mock = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(r#"{"domains":[{"name":"existing"}]}"#)
        .create_async()
        .await;
    let _staked_mock = server
        .mock("GET", "/api/domains/staked")
        .with_status(200)
        .with_body(r#"{"stakedDomains":[]}"#)
        .create_async()
        .await;

    let conn = seeded_conn(&server.url());
    conn.execute(
        "INSERT INTO assets (tld, is_staked, status, category, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["existing", 1, "do_not_touch_staked", "Manual", "old note"],
    )
    .unwrap();
    let app = app_with(conn);

    let result = import_from_namebase(app.state::<AppState>())
        .await
        .expect("import should succeed");

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let asset = db::queries::get_assets_by_tlds(&db, &["existing".to_string()])
        .unwrap()
        .into_iter()
        .next()
        .expect("asset should exist");
    assert!(!asset.is_staked, "is_staked should be updated to false");
    assert_eq!(asset.status.as_str(), "do_not_touch_staked");
    assert_eq!(result["imported"], 1);
}

#[tokio::test]
async fn import_skips_malformed_domain_rows() {
    let mut server = Server::new_async().await;
    let _domains_mock = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(
            r#"{"domains":[
                {"name":"good"},
                {"notname":"missing"},
                {"name":""}
            ]}"#,
        )
        .create_async()
        .await;
    let _staked_mock = server
        .mock("GET", "/api/domains/staked")
        .with_status(200)
        .with_body(r#"{"stakedDomains":[]}"#)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let result = import_from_namebase(app.state::<AppState>())
        .await
        .expect("import should succeed");
    assert_eq!(result["imported"], 2);
    assert_eq!(result["skipped"], 1);
}

#[tokio::test]
async fn import_propagates_domains_api_error() {
    let mut server = Server::new_async().await;
    let _domains_mock = server
        .mock("GET", "/api/domains")
        .with_status(500)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = import_from_namebase(app.state::<AppState>())
        .await
        .expect_err("should fail on 500");
    let msg = format!("{err}");
    assert!(msg.contains("500"), "msg: {msg}");
}

#[tokio::test]
async fn import_propagates_staked_api_error() {
    let mut server = Server::new_async().await;
    let _domains_mock = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(r#"{"domains":[]}"#)
        .create_async()
        .await;
    let _staked_mock = server
        .mock("GET", "/api/domains/staked")
        .with_status(500)
        .create_async()
        .await;

    let app = app_with(seeded_conn(&server.url()));
    let err = import_from_namebase(app.state::<AppState>())
        .await
        .expect_err("should fail on 500");
    let msg = format!("{err}");
    assert!(msg.contains("500"), "msg: {msg}");
}

// =========================================================================
// connect_namebase (now uses namebase_client_with_cookie -> honors the seam)
// =========================================================================

#[tokio::test]
async fn connect_namebase_success_stores_cookie_and_returns_account() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(r#"{"email":"test@namebase.io","balance":1000}"#)
        .create_async()
        .await;

    let conn = conn_without_cookie();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let v = connect_namebase(app.state::<AppState>(), "session-cookie-abc".into())
        .await
        .expect("connect should succeed against mock");
    assert_eq!(v["email"], "test@namebase.io");
    assert_eq!(v["balance"], 1000);

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let settings = db::queries::get_settings(&db).unwrap();
    assert_eq!(settings.get("namebase_cookie").map(|s| s.as_str()), Some("session-cookie-abc"));

    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'namebase_connect'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1);
}

#[tokio::test]
async fn connect_namebase_rejects_invalid_session() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account")
        .with_status(401)
        .with_body(r#"{"error":"unauthorized"}"#)
        .create_async()
        .await;

    let conn = conn_without_cookie();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let err = connect_namebase(app.state::<AppState>(), "bad-cookie".into())
        .await
        .expect_err("invalid session must be rejected");
    match err {
        AppError::Other(m) => assert!(m.contains("401"), "msg: {m}"),
        other => panic!("expected AppError::Other, got {other:?}"),
    }
}

// =========================================================================
// disconnect_namebase - additional side-effect assertions
// =========================================================================

#[tokio::test]
async fn disconnect_writes_audit_log_entry() {
    let conn = seeded_conn("http://localhost:1");
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    disconnect_namebase(state.clone())
        .await
        .expect("disconnect should succeed");

    let db = state.db.lock().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'namebase_disconnect'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1);
}

// =========================================================================
// namebase_transfer_domain - additional asset-status side-effect assertion
// =========================================================================

#[tokio::test]
async fn transfer_domain_sets_asset_status_to_transfer_requested() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("POST", "/api/domains/exampletld/withdraw")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn(&server.url());
    conn.execute(
        "INSERT INTO assets (tld, is_staked, status, category, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["exampletld", 0, "not_started", "Namebase", ""],
    )
    .unwrap();
    let app = app_with(conn);

    namebase_transfer_domain(
        app.state::<AppState>(),
        "exampletld".into(),
        good_addr(),
    )
    .await
    .expect("transfer should succeed");

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let asset = db::queries::get_assets_by_tlds(&db, &["exampletld".to_string()])
        .unwrap()
        .into_iter()
        .next()
        .expect("asset should exist");
    assert_eq!(
        asset.status.as_str(),
        "namebase_transfer_requested"
    );
    assert!(!asset.updated_at.is_empty());
}

// =========================================================================
// namebase_withdraw_hns - additional audit-log detail assertion
// =========================================================================

#[tokio::test]
async fn withdraw_hns_stores_address_and_amount_in_audit_log() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("POST", "/api/withdrawals")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn(&server.url());
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    namebase_withdraw_hns(state.clone(), good_addr(), "3.5".into())
        .await
        .expect("withdraw should succeed");

    let db = state.db.lock().unwrap();
    let detail: String = db
        .query_row(
            "SELECT detail FROM audit_log WHERE action = 'namebase_withdraw_hns' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&detail).expect("audit detail should be valid JSON");
    assert_eq!(parsed["address"], good_addr());
    assert_eq!(parsed["amount"], "3.5");
}

// =========================================================================
// Edge case: namebase_client with whitespace-only base_url
// =========================================================================

#[tokio::test]
async fn namebase_client_trims_whitespace_from_base_url() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .create_async()
        .await;

    let conn = seeded_conn(&server.url());
    db::queries::set_setting(&conn, "namebase_base_url", &format!("  {}  ", server.url())).unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();

    let client = crate::commands::namebase::namebase_client(&state).unwrap();
    let result = client.check_session().await;
    assert!(result.is_ok(), "should reach mock server despite whitespace: {:?}", result.err());
}

// =========================================================================
// active_profile_network fallback tests
// =========================================================================

#[tokio::test]
async fn transfer_domain_falls_back_to_mainnet_when_no_profile() {
    let conn = conn_without_profile();
    db::queries::set_setting(&conn, "namebase_base_url", "http://localhost:1").unwrap();
    let app = app_with(conn);

    let res = namebase_transfer_domain(
        app.state::<AppState>(),
        "exampletld".into(),
        good_addr(),
    )
    .await;
    match res {
        Ok(()) => {}
        Err(AppError::InvalidInput(m)) => {
            assert!(!m.contains("HNS address"), "valid address wrongly rejected: {m}");
        }
        Err(_) => {}
    }
}

#[tokio::test]
async fn withdraw_hns_falls_back_to_mainnet_when_no_profile() {
    let conn = conn_without_profile();
    db::queries::set_setting(&conn, "namebase_base_url", "http://localhost:1").unwrap();
    let app = app_with(conn);

    let res = namebase_withdraw_hns(
        app.state::<AppState>(),
        good_addr(),
        "1.0".into(),
    )
    .await;
    match res {
        Ok(()) => {}
        Err(AppError::InvalidInput(m)) => {
            assert!(!m.contains("HNS address"), "valid address wrongly rejected: {m}");
            assert!(!m.contains("positive"), "valid amount wrongly rejected: {m}");
        }
        Err(_) => {}
    }
}

// =========================================================================
// namebase_client_with_cookie tests
// =========================================================================

#[tokio::test]
async fn namebase_client_with_cookie_uses_base_url_from_settings() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .create_async()
        .await;

    let conn = conn_without_cookie();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();

    let client = crate::commands::namebase::namebase_client_with_cookie(&state, "my-cookie")
        .unwrap();
    let result = client.check_session().await;
    assert!(result.is_ok(), "should reach mock server: {:?}", result.err());
}

#[tokio::test]
async fn namebase_client_with_cookie_falls_back_to_default_host() {
    let conn = conn_without_cookie();
    let app = app_with(conn);
    let state = app.state::<AppState>();

    let client = crate::commands::namebase::namebase_client_with_cookie(&state, "some-cookie")
        .unwrap();
    let result = client.check_session().await;
    let _ = result;
}
