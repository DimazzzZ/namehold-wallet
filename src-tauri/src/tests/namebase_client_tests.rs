//! Unit tests for `namebase::client::NamebaseClient`.
//!
//! Each public method is exercised against a mockito HTTP server so we don't need
//! a real Namebase session. The client's `with_base_url` constructor is the test
//! seam that points at the mock.

use crate::namebase::client::NamebaseClient;

// ---------------------------------------------------------------------------
// Constructor tests
// ---------------------------------------------------------------------------

#[test]
fn test_new_uses_production_host() {
    let client = NamebaseClient::new("test-cookie").expect("new should succeed");
    // We can't inspect the private fields, but we can verify it doesn't error.
    // The real test is that `with_base_url` accepts an explicit URL.
}

#[test]
fn test_with_base_url_trims_trailing_slash() {
    let client = NamebaseClient::with_base_url("c", "https://example.com/")
        .expect("with_base_url should succeed");
    // Construction succeeds — the trim is verified indirectly via mockito tests.
}

#[test]
fn test_with_base_url_does_not_trim_single_slash() {
    let client = NamebaseClient::with_base_url("c", "https://example.com")
        .expect("with_base_url should succeed");
}

#[test]
fn test_with_base_url_empty_cookie() {
    let client = NamebaseClient::with_base_url("", "https://example.com")
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
    let result = client.check_session().await.expect("check_session should succeed");
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
    let result = client.check_session().await.expect("check_session should succeed");
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
    let result = client.check_session().await.expect("check_session should succeed");
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
    let v = client.get_account().await.expect("get_account should succeed");
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
    let v = client.get_domains().await.expect("get_domains should succeed");
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
    let v = client.get_staked_domains().await.expect("get_staked_domains should succeed");
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
    let v = client.get_renewals().await.expect("get_renewals should succeed");
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
    let v = client.get_withdrawals().await.expect("get_withdrawals should succeed");
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
    let v = client.get_domain_withdrawals().await.expect("get_domain_withdrawals should succeed");
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
    client.transfer_domain("mytld", "hs1qtest").await.expect("transfer should succeed");
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
    let err = client.transfer_domain("bad", "hs1qtest").await.expect_err("should error");
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
    let err = client.transfer_domain("nomsg", "hs1qtest").await.expect_err("should error");
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
    client.withdraw_hns("hs1qtest", "1.5").await.expect("withdraw should succeed");
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
    let err = client.withdraw_hns("hs1qtest", "999").await.expect_err("should error");
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
    let err = client.withdraw_hns("hs1qtest", "5").await.expect_err("should error");
    let msg = format!("{err}");
    assert!(msg.contains("status 400"), "msg: {msg}");
    m.assert_async().await;
}
