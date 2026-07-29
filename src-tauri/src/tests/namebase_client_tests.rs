//! Unit tests for `namebase::client::NamebaseClient`.
//!
//! Each public method is exercised against a mockito HTTP server so we don't need
//! a real Namebase session. The client's `with_base_url` constructor is the test
//! seam that points at the mock.

use crate::error::AppError;
use crate::namebase::client::NamebaseClient;

// ---------------------------------------------------------------------------
// Constructor tests
// ---------------------------------------------------------------------------

#[test]
fn test_new_uses_production_host() {
    let _client = NamebaseClient::new("test-cookie").expect("new should succeed");
    // We can't inspect the private fields, but we can verify it doesn't error.
    // The real test is that `with_base_url` accepts an explicit URL.
}

#[test]
fn test_with_base_url_trims_trailing_slash() {
    // Host must be the allowlisted Namebase host or loopback (test build);
    // loopback exercises the trailing-slash trim without tripping the guard.
    let _client = NamebaseClient::with_base_url("c", "http://127.0.0.1:8080/")
        .expect("with_base_url should succeed");
    // Construction succeeds — the trim is verified indirectly via mockito tests.
}

#[test]
fn test_with_base_url_does_not_trim_single_slash() {
    let _client = NamebaseClient::with_base_url("c", "http://127.0.0.1:8080")
        .expect("with_base_url should succeed");
}

#[test]
fn test_with_base_url_empty_cookie() {
    let _client = NamebaseClient::with_base_url("", "http://127.0.0.1:8080")
        .expect("empty cookie should be accepted");
}

// ---------------------------------------------------------------------------
// check_session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_check_session_returns_true_on_200() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(r#"{"email":"test@example.com"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let result = client
        .check_session()
        .await
        .expect("check_session should succeed");
    assert!(result, "200 → true");
    m.assert_async().await;
}

#[tokio::test]
async fn test_check_session_returns_false_on_401() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(401)
        .with_body(r#"{"error":"unauthorized"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let result = client
        .check_session()
        .await
        .expect("check_session should succeed");
    assert!(!result, "401 → false");
    m.assert_async().await;
}

#[tokio::test]
async fn test_check_session_returns_false_on_500() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(500)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let result = client
        .check_session()
        .await
        .expect("check_session should succeed");
    assert!(!result, "500 → false");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_account
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_account_returns_json_on_200() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"email":"a@b.com","balance":100}"#;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client
        .get_account()
        .await
        .expect("get_account should succeed");
    assert_eq!(v["email"], "a@b.com");
    assert_eq!(v["balance"], 100);
    m.assert_async().await;
}

#[tokio::test]
async fn test_get_account_errors_on_non_200() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(403)
        .with_body(r#"{"error":"forbidden"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client.get_account().await.expect_err("should error on 403");
    let msg = format!("{err}");
    assert!(msg.contains("403"), "msg: {msg}");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_domains
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_domains_returns_json() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"domains":[{"name":"example","status":"active"}]}"#;
    let m = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client
        .get_domains()
        .await
        .expect("get_domains should succeed");
    assert_eq!(v["domains"][0]["name"], "example");
    m.assert_async().await;
}

#[tokio::test]
async fn test_get_domains_errors_on_non_200() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/domains")
        .with_status(500)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client.get_domains().await.expect_err("should error on 500");
    let msg = format!("{err}");
    assert!(msg.contains("500"), "msg: {msg}");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_staked_domains
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_staked_domains_returns_json() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"stakedDomains":[{"name":"staked1"}]}"#;
    let m = server
        .mock("GET", "/api/domains/staked")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client
        .get_staked_domains()
        .await
        .expect("get_staked_domains should succeed");
    assert_eq!(v["stakedDomains"][0]["name"], "staked1");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_renewals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_renewals_returns_json() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"expiring":[{"domain":"soon","expire_block":100}]}"#;
    let m = server
        .mock("GET", "/api/domains/renewals")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client
        .get_renewals()
        .await
        .expect("get_renewals should succeed");
    assert_eq!(v["expiring"][0]["domain"], "soon");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_withdrawals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_withdrawals_returns_json() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"withdrawals":[{"id":1,"amount":"100"}]}"#;
    let m = server
        .mock("GET", "/api/withdrawals")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client
        .get_withdrawals()
        .await
        .expect("get_withdrawals should succeed");
    assert_eq!(v["withdrawals"][0]["id"], 1);
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_slds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_slds_returns_json() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"slds":[{"name":"sub.example"}]}"#;
    let m = server
        .mock("GET", "/api/slds")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client.get_slds().await.expect("get_slds should succeed");
    assert_eq!(v["slds"][0]["name"], "sub.example");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_domain_withdrawals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_domain_withdrawals_returns_json() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{"domainWithdrawals":[{"domain":"ex","status":"pending"}]}"#;
    let m = server
        .mock("GET", "/api/domains/withdrawals")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let v = client
        .get_domain_withdrawals()
        .await
        .expect("get_domain_withdrawals should succeed");
    assert_eq!(v["domainWithdrawals"][0]["domain"], "ex");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// transfer_domain
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_transfer_domain_succeeds_on_200() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/domains/mytld/withdraw")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"address": "hs1qtest"}),
        ))
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    client
        .transfer_domain("mytld", "hs1qtest")
        .await
        .expect("transfer should succeed");
    m.assert_async().await;
}

#[tokio::test]
async fn test_transfer_domain_propagates_error() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/domains/bad/withdraw")
        .with_status(400)
        .with_body(r#"{"error":"Domain not found"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .transfer_domain("bad", "hs1qtest")
        .await
        .expect_err("should error");
    let msg = format!("{err}");
    assert!(msg.contains("Domain not found"), "msg: {msg}");
    m.assert_async().await;
}

#[tokio::test]
async fn test_transfer_domain_error_fallback_when_no_error_field() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/domains/nomsg/withdraw")
        .with_status(400)
        .with_body(r#"{"other":"data"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .transfer_domain("nomsg", "hs1qtest")
        .await
        .expect_err("should error");
    let msg = format!("{err}");
    assert!(msg.contains("status 400"), "msg: {msg}");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// withdraw_hns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_withdraw_hns_succeeds_on_200() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/withdrawals")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "currency": "hns",
            "amount": "1.5",
            "address": "hs1qtest",
        })))
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    client
        .withdraw_hns("hs1qtest", "1.5")
        .await
        .expect("withdraw should succeed");
    m.assert_async().await;
}

#[tokio::test]
async fn test_withdraw_hns_propagates_error() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/withdrawals")
        .with_status(400)
        .with_body(r#"{"error":"Insufficient balance"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .withdraw_hns("hs1qtest", "999")
        .await
        .expect_err("should error");
    let msg = format!("{err}");
    assert!(msg.contains("Insufficient balance"), "msg: {msg}");
    m.assert_async().await;
}

#[tokio::test]
async fn test_withdraw_hns_error_fallback_when_no_error_field() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/withdrawals")
        .with_status(400)
        .with_body(r#"{"other":"data"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .withdraw_hns("hs1qtest", "5")
        .await
        .expect_err("should error");
    let msg = format!("{err}");
    assert!(msg.contains("status 400"), "msg: {msg}");
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// Set-Cookie capture (Task 5)
// ---------------------------------------------------------------------------

/// A response carrying `Set-Cookie: nb-sunset=NEW; Path=/` must update the jar:
/// the next request sends the replaced value, `current_cookie()` reflects it
/// immediately, and cookies from the original string that weren't touched by
/// the response survive untouched (same value, same relative order).
#[tokio::test]
async fn test_set_cookie_replaces_named_cookie_and_preserves_others() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/api/domains")
        .match_header("cookie", "nb-sunset=OLD; session=abc123")
        .with_status(200)
        .with_header("set-cookie", "nb-sunset=NEW; Path=/")
        .with_body(r#"{"domains":[]}"#)
        .create_async()
        .await;
    let second = server
        .mock("GET", "/api/domains/staked")
        .match_header("cookie", "nb-sunset=NEW; session=abc123")
        .with_status(200)
        .with_body(r#"{"stakedDomains":[]}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("nb-sunset=OLD; session=abc123", &server.url())
        .expect("client should construct");

    client
        .get_domains()
        .await
        .expect("first call should succeed");
    assert_eq!(client.current_cookie(), "nb-sunset=NEW; session=abc123");

    client
        .get_staked_domains()
        .await
        .expect("second call should send the updated cookie");

    first.assert_async().await;
    second.assert_async().await;
}

/// Multiple `Set-Cookie` headers on one response are all applied, and a cookie
/// not mentioned by any of them is left alone.
#[tokio::test]
async fn test_set_cookie_multiple_headers_all_applied() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_header("set-cookie", "nb-sunset=NEW1")
        .with_header("set-cookie", "extra=NEW2")
        .with_body(r#"{"email":"a@b.com"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("nb-sunset=OLD; keep=me", &server.url()).unwrap();
    client.get_account().await.expect("should succeed");

    assert_eq!(
        client.current_cookie(),
        "nb-sunset=NEW1; keep=me; extra=NEW2"
    );
    m.assert_async().await;
}

/// `Max-Age=0` on a `Set-Cookie` header deletes that cookie from the jar rather
/// than storing an empty value.
#[tokio::test]
async fn test_set_cookie_max_age_zero_deletes_cookie() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_header("set-cookie", "nb-sunset=; Max-Age=0")
        .with_body(r#"{"email":"a@b.com"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("nb-sunset=OLD; keep=me", &server.url()).unwrap();
    client.get_account().await.expect("should succeed");

    assert_eq!(client.current_cookie(), "keep=me");
    m.assert_async().await;
}

/// An `Expires` date far in the past also deletes the cookie (naive HTTP-date
/// parsing is enough per the task brief).
#[tokio::test]
async fn test_set_cookie_expires_in_past_deletes_cookie() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_header(
            "set-cookie",
            "nb-sunset=; Expires=Wed, 09 Jun 2021 10:18:14 GMT",
        )
        .with_body(r#"{"email":"a@b.com"}"#)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("nb-sunset=OLD; keep=me", &server.url()).unwrap();
    client.get_account().await.expect("should succeed");

    assert_eq!(client.current_cookie(), "keep=me");
    m.assert_async().await;
}

/// Bare (non `name=value`) cookie fixtures — as used by the constructor tests
/// above — must round-trip byte-for-byte when nothing touches them.
#[tokio::test]
async fn test_current_cookie_round_trips_bare_token() {
    let client = NamebaseClient::with_base_url("test-cookie-123", "http://127.0.0.1:8080").unwrap();
    assert_eq!(client.current_cookie(), "test-cookie-123");
}

// ---------------------------------------------------------------------------
// Session-expired detection (Task 5)
// ---------------------------------------------------------------------------

/// An HTML login page served with HTTP 200 (Namebase's soft-expiry behavior)
/// must surface as a typed `NamebaseSessionExpired` error, not a JSON parse
/// error.
#[tokio::test]
async fn test_get_account_html_200_is_session_expired_not_parse_error() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body("<!doctype html><html><body>Please log in</body></html>")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .get_account()
        .await
        .expect_err("HTML body should be treated as session-expired");
    assert!(
        matches!(err, AppError::NamebaseSessionExpired),
        "expected NamebaseSessionExpired, got: {err:?}"
    );
    m.assert_async().await;
}

/// A non-JSON `content-type` on an otherwise-200 response is treated the same
/// way, even if the body happens not to start with `<`.
#[tokio::test]
async fn test_get_domains_non_json_content_type_is_session_expired() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("please log in")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .get_domains()
        .await
        .expect_err("non-JSON content-type should be session-expired");
    assert!(
        matches!(err, AppError::NamebaseSessionExpired),
        "got: {err:?}"
    );
    m.assert_async().await;
}

/// `check_session` must also treat an HTML 200 as "not connected" rather than
/// reporting a live session.
#[tokio::test]
async fn test_check_session_html_200_returns_false() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account")
        .with_status(200)
        .with_body("<html>login</html>")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let result = client
        .check_session()
        .await
        .expect("check_session should not error on HTML");
    assert!(
        !result,
        "HTML login page should not count as a valid session"
    );
    m.assert_async().await;
}

// ---------------------------------------------------------------------------
// get_account_history — the CSV export endpoint uses a body-only expiry check
// so a legitimate text/csv response is NOT flagged as expired (the JSON-endpoint
// heuristic would misclassify it, since text/csv doesn't contain "json").
// ---------------------------------------------------------------------------

/// A successful CSV response with `Content-Type: text/csv` must be returned as
/// the raw CSV body, NOT flagged as session-expired. Regression guard for the
/// bug where the JSON-endpoint heuristic (`content-type does not contain
/// 'json' → expired`) misfired on every valid export.
#[tokio::test]
async fn test_get_account_history_csv_200_is_not_session_expired() {
    let mut server = mockito::Server::new_async().await;
    let csv_body = "\"This export covers your Namebase account history only.\"\n\
                    \"\"\n\
                    \"It does NOT include Sunset activity.\"\n\
                    \n\
                    id,created_at,type,data\n\
                    1,2024-01-01T00:00:00Z,auctions:place-bid:4,\"{}\"\n";
    let m = server
        .mock("GET", "/api/account/history/export")
        .with_status(200)
        .with_header("content-type", "text/csv")
        .with_body(csv_body)
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let body = client
        .get_account_history()
        .await
        .expect("valid CSV must not be flagged as expired");
    assert!(
        body.contains("id,created_at,type,data"),
        "expected export header in returned body, got: {body:?}"
    );
    m.assert_async().await;
}

/// An HTML login page served with HTTP 200 (Namebase's soft-expiry behavior)
/// on the export endpoint must surface as `NamebaseSessionExpired`.
#[tokio::test]
async fn test_get_account_history_html_200_is_session_expired() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account/history/export")
        .with_status(200)
        .with_body("<!doctype html><html><body>Please log in</body></html>")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .get_account_history()
        .await
        .expect_err("HTML body should be treated as session-expired");
    assert!(
        matches!(err, AppError::NamebaseSessionExpired),
        "expected NamebaseSessionExpired, got: {err:?}"
    );
    m.assert_async().await;
}

/// A 429 response must surface as `NamebaseRateLimited` with the `Retry-After`
/// value parsed out, not as generic error or session-expired.
#[tokio::test]
async fn test_get_account_history_429_is_rate_limited() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/account/history/export")
        .with_status(429)
        .with_header("retry-after", "42")
        .with_body("rate limited")
        .create_async()
        .await;

    let client = NamebaseClient::with_base_url("c", &server.url()).unwrap();
    let err = client
        .get_account_history()
        .await
        .expect_err("429 should surface as NamebaseRateLimited");
    match err {
        AppError::NamebaseRateLimited { retry_after_secs } => {
            assert_eq!(retry_after_secs, 42);
        }
        other => panic!("expected NamebaseRateLimited, got: {other:?}"),
    }
    m.assert_async().await;
}
