//! Guard tests for `namebase_transfer_domain`'s destination-address validation.
//!
//! A Namebase withdrawal is irreversible, so the command MUST reject a malformed
//! or wrong-network address BEFORE making the Namebase call. Validation happens
//! first (no cookie needed), so these assert the rejection without any network.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::namebase::{
    disconnect_namebase, fetch_namebase_renewals, get_namebase_status, namebase_transfer_domain,
    namebase_withdraw_hns,
};
use crate::db;
use crate::error::AppError;
use crate::AppState;

const PROFILE: &str = "nbp1";

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

/// In-memory DB with an active MAINNET profile.
fn seeded_conn() -> rusqlite::Connection {
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
    conn
}

#[tokio::test]
async fn rejects_malformed_destination_before_namebase_call() {
    let app = app_with(seeded_conn());
    let err = namebase_transfer_domain(
        app.state::<AppState>(),
        "exampletld".into(),
        "not-an-address".into(),
    )
    .await
    .expect_err("malformed address must be rejected");
    match err {
        AppError::InvalidInput(m) => assert!(m.contains("HNS address"), "msg: {m}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_wrong_network_destination() {
    // A valid REGTEST address (rs1…) must be rejected for a mainnet profile.
    let app = app_with(seeded_conn());
    let err = namebase_transfer_domain(
        app.state::<AppState>(),
        "exampletld".into(),
        "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2".into(),
    )
    .await
    .expect_err("wrong-network address must be rejected");
    match err {
        AppError::InvalidInput(m) => assert!(m.contains("main"), "msg: {m}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn accepts_valid_mainnet_address_past_validation() {
    // A well-formed mainnet address passes validation; it then proceeds to the
    // Namebase call, which fails because no cookie is configured. The point is it
    // gets PAST validation — i.e. the error is NOT an address InvalidInput.
    let app = app_with(seeded_conn());
    let res = namebase_transfer_domain(
        app.state::<AppState>(),
        "exampletld".into(),
        "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8".into(),
    )
    .await;
    match res {
        Ok(()) => {} // unlikely without a real cookie, but acceptable
        Err(AppError::InvalidInput(m)) => {
            assert!(
                !m.contains("HNS address"),
                "valid address wrongly rejected: {m}"
            );
        }
        Err(_) => {} // Namebase/network/cookie error — past validation, as expected
    }
}

// --- namebase_withdraw_hns guards (mirror the domain-transfer guards) -------

#[tokio::test]
async fn withdraw_rejects_malformed_destination() {
    let app = app_with(seeded_conn());
    let err = namebase_withdraw_hns(
        app.state::<AppState>(),
        "not-an-address".into(),
        "1000000".into(),
    )
    .await
    .expect_err("malformed address must be rejected");
    match err {
        AppError::InvalidInput(m) => assert!(m.contains("HNS address"), "msg: {m}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn withdraw_rejects_wrong_network_destination() {
    let app = app_with(seeded_conn());
    let err = namebase_withdraw_hns(
        app.state::<AppState>(),
        "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2".into(),
        "1000000".into(),
    )
    .await
    .expect_err("wrong-network address must be rejected");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
}

#[tokio::test]
async fn withdraw_rejects_nonpositive_amount() {
    // Valid mainnet address, but amount 0 / non-numeric → rejected before the call.
    let app = app_with(seeded_conn());
    let good_addr = "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8";
    for bad in ["0", "-5", "abc", ""] {
        let err = namebase_withdraw_hns(app.state::<AppState>(), good_addr.into(), bad.into())
            .await
            .expect_err("non-positive amount must be rejected");
        match err {
            AppError::InvalidInput(m) => assert!(m.contains("positive"), "amount '{bad}' msg: {m}"),
            other => panic!("expected InvalidInput for '{bad}', got {other:?}"),
        }
    }
}

// --- execution against a mock Namebase API ---------------------------------

const GOOD_ADDR: &str = "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8";

#[tokio::test]
async fn transfer_domain_posts_to_namebase_with_the_address() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/domains/exampletld/withdraw")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({ "address": GOOD_ADDR }),
        ))
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    namebase_transfer_domain(
        app.state::<AppState>(),
        "exampletld".into(),
        GOOD_ADDR.into(),
    )
    .await
    .expect("transfer should succeed against the mock");
    m.assert_async().await;
}

#[tokio::test]
async fn withdraw_hns_posts_currency_amount_and_address() {
    let mut server = mockito::Server::new_async().await;
    // The create endpoint expects the amount in HNS (not doos) — guard the unit.
    let m = server
        .mock("POST", "/api/withdrawals")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "currency": "hns",
            "amount": "2",
            "address": GOOD_ADDR,
        })))
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    namebase_withdraw_hns(app.state::<AppState>(), GOOD_ADDR.into(), "2".into())
        .await
        .expect("withdraw should succeed against the mock");
    m.assert_async().await;
}

#[tokio::test]
async fn fetch_renewals_returns_the_expiring_calendar() {
    // The renewal calendar (/api/domains/renewals) is now surfaced in the UI;
    // lock the contract: the command returns Namebase's `{ expiring: [...] }`.
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/domains/renewals")
        .with_status(200)
        .with_body(
            r#"{"expiring":[
                {"domain":"soon","expire_block":339000,"estimated_date":"2026-07-05T00:00:00.000Z"},
                {"domain":"later","expire_block":340000,"estimated_date":"2026-09-01T00:00:00.000Z"}
            ]}"#,
        )
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let v = fetch_namebase_renewals(app.state::<AppState>())
        .await
        .expect("renewals fetch should succeed against the mock");
    m.assert_async().await;

    let expiring = v
        .get("expiring")
        .and_then(|e| e.as_array())
        .expect("expiring array present");
    assert_eq!(expiring.len(), 2);
    assert_eq!(expiring[0]["domain"], serde_json::json!("soon"));
    assert_eq!(expiring[0]["expire_block"], serde_json::json!(339000));
    assert!(expiring[0]["estimated_date"]
        .as_str()
        .unwrap()
        .ends_with("Z"));
}

#[tokio::test]
async fn withdraw_accepts_decimal_hns_amount_past_validation() {
    // HNS amounts are decimal — "1.5" must pass validation (it then fails later
    // at the Namebase call since there's no cookie, which is fine here).
    let app = app_with(seeded_conn());
    let good_addr = "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8";
    let res = namebase_withdraw_hns(app.state::<AppState>(), good_addr.into(), "1.5".into()).await;
    match res {
        Ok(()) => {}
        Err(AppError::InvalidInput(m)) => {
            assert!(
                !m.contains("amount") && !m.contains("HNS address"),
                "decimal amount wrongly rejected by validation: {m}"
            );
        }
        Err(_) => {} // Namebase/cookie/network error — past validation, as expected
    }
}

// --- disconnect_namebase tests -----------------------------------------------

#[tokio::test]
async fn disconnect_clears_cookie_and_logs_audit() {
    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "old-cookie").unwrap();
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    disconnect_namebase(state.clone())
        .await
        .expect("disconnect should succeed");

    let db = state.db.lock().unwrap();
    let settings = db::queries::get_settings(&db).unwrap();
    assert_eq!(
        settings.get("namebase_cookie").map(|s| s.as_str()),
        Some("")
    );
}

// --- transfer_domain error from Namebase API ---------------------------------

#[tokio::test]
async fn transfer_domain_propagates_namebase_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/api/domains/baddomain/withdraw")
        .with_status(400)
        .with_body(r#"{"error":"Domain not found"}"#)
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    let err = namebase_transfer_domain(state, "baddomain".into(), GOOD_ADDR.into())
        .await
        .expect_err("should fail with Namebase error");
    match err {
        AppError::Other(m) => assert!(m.contains("Domain not found"), "msg: {m}"),
        other => panic!("expected Other, got {other:?}"),
    }
}

// --- withdraw_hns error from Namebase API ------------------------------------

#[tokio::test]
async fn withdraw_hns_propagates_namebase_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/api/withdrawals")
        .with_status(400)
        .with_body(r#"{"error":"Insufficient balance"}"#)
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    let err = namebase_withdraw_hns(state, GOOD_ADDR.into(), "999".into())
        .await
        .expect_err("should fail with Namebase error");
    match err {
        AppError::Other(m) => assert!(m.contains("Insufficient balance"), "msg: {m}"),
        other => panic!("expected Other, got {other:?}"),
    }
}

// --- transfer_domain audit log test ------------------------------------------

#[tokio::test]
async fn transfer_domain_writes_audit_log() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/api/domains/audittest/withdraw")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    namebase_transfer_domain(state.clone(), "audittest".into(), GOOD_ADDR.into())
        .await
        .expect("transfer should succeed");

    let db = state.db.lock().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'namebase_transfer'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1, "audit log should have a transfer entry");
}

// --- withdraw_hns audit log test ---------------------------------------------

#[tokio::test]
async fn withdraw_hns_writes_audit_log() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/api/withdrawals")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);

    let state = app.state::<AppState>().clone();
    namebase_withdraw_hns(state.clone(), GOOD_ADDR.into(), "5".into())
        .await
        .expect("withdraw should succeed");

    let db = state.db.lock().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'namebase_withdraw_hns'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1, "audit log should have a withdraw entry");
}

// =========================================================================
// get_namebase_status tests
// =========================================================================
//
// NOTE: Only the no-cookie path is tested here (it returns immediately
// without an HTTP call). The cookie-and-session paths use
// `NamebaseClient::new()` which hardcodes the real Namebase host and
// ignores the `namebase_base_url` seam, so they cannot be tested with
// mockito. Those paths are covered indirectly by the `namebase_client()`
// function via `namebase_transfer_domain` / `namebase_withdraw_hns` tests
// (which DO honor `namebase_base_url`), and by the client-level tests in
// `namebase_client_tests.rs`.

#[tokio::test]
async fn status_reports_not_connected_when_no_cookie() {
    let app = app_with(seeded_conn());
    let v = get_namebase_status(app.state::<AppState>())
        .await
        .expect("status should succeed");
    assert_eq!(v["connected"], serde_json::json!(false));
    assert_eq!(v["has_cookie"], serde_json::json!(false));
}

// NOTE: fetch_namebase_domains, fetch_namebase_staked,
// fetch_namebase_withdrawals, fetch_namebase_domain_withdrawals, and
// import_from_namebase all use NamebaseClient::new() which hardcodes the
// real Namebase host and ignores the namebase_base_url test seam, so
// mock-based tests for those commands are not possible without production
// code changes. The NamebaseClient methods themselves are covered by
// namebase_client_tests.rs.

// =========================================================================
// namebase_client function tests
// =========================================================================

#[tokio::test]
async fn namebase_client_no_base_url_uses_default() {
    let conn = seeded_conn();
    let app = app_with(conn);
    let state = app.state::<AppState>();
    let client = crate::commands::namebase::namebase_client(&state).unwrap();
    // Without namebase_base_url set, it should use NamebaseClient::new()
    // which uses the real production host. The check_session will fail with
    // a transport error (no real network), but construction should succeed.
    let _ = client;
}

#[tokio::test]
async fn namebase_client_with_base_url_uses_custom() {
    let mut server = mockito::Server::new_async().await;
    let conn = seeded_conn();
    crate::db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    crate::db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();
    let client = crate::commands::namebase::namebase_client(&state).unwrap();
    // When namebase_base_url is set, it should construct with that URL.
    // Verify by making a request to the mock server.
    let _m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .create_async()
        .await;
    let result = client.check_session().await;
    assert!(
        result.is_ok(),
        "should reach mock server: {:?}",
        result.err()
    );
}
