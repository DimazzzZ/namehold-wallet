//! Tests for the HNSFans external read-only provider client.
//!
//! These exercise URL normalization, the health probe's lenient reachability
//! behavior, and the defensive JSON parsing in balance/name/transaction
//! mappers using a mock HTTP server. The provider targets the
//! `e.hnsfans.com` explorer API contract:
//!   * balance:      GET /api/addresses/:address
//!   * name detail:  GET /api/names/:name
//!   * txs:          GET /api/txs/:hash and GET /api/txs?...
//!   * health probe: GET /api/txs?limit=1

use crate::providers::hnsfans::HnsFansClient;
use mockito::Server;

#[tokio::test]
async fn test_health_succeeds_on_probe_endpoint() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/txs")
        .match_query(mockito::Matcher::UrlEncoded(
            "limit".into(),
            "1".into(),
        ))
        .with_status(200)
        .with_body(r#"{"limit":1,"offset":0,"total":0,"result":[]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.health().await;
    assert!(result.is_ok());
    mock.assert_async().await;
}

#[tokio::test]
async fn test_health_succeeds_when_probe_endpoint_responds_with_error() {
    // The probe is intentionally lenient: any HTTP response from the probe
    // route (including a 4xx/5xx) means the host is reachable, so health()
    // returns Ok without needing to fall back to the base URL.
    let mut server = Server::new_async().await;
    let probe_mock = server
        .mock("GET", "/api/txs")
        .match_query(mockito::Matcher::Any)
        .with_status(404)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.health().await;
    assert!(result.is_ok());
    probe_mock.assert_async().await;
}

#[tokio::test]
async fn test_health_errors_when_unreachable() {
    // No server is listening on this port, so the request fails at the
    // transport layer, which is the only condition treated as unhealthy.
    let client = HnsFansClient::new("http://127.0.0.1:1");
    let result = client.health().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_balance_reads_explorer_address_payload() {
    // Mirrors GET /api/addresses/:address -> { confirmed, unconfirmed, ... }.
    let mut server = Server::new_async().await;
    let addr = "hs1qexampleaddress";
    let mock = server
        .mock("GET", format!("/api/addresses/{}", addr).as_str())
        .with_status(200)
        .with_body(r#"{"hash":"hs1qexampleaddress","received":1000000,"spent":0,"confirmed":1000000,"unconfirmed":0}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let balance = client
        .get_balance(&[addr.to_string()])
        .await
        .expect("balance should succeed");
    assert_eq!(balance.confirmed, 1_000_000);
    assert_eq!(balance.unconfirmed, 0);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_balance_errors_when_all_addresses_fail() {
    // If every watched address request fails, the provider must return an
    // error rather than a misleading zero balance.
    let mut server = Server::new_async().await;
    let addr = "hs1qfailingaddress";
    let mock = server
        .mock("GET", format!("/api/addresses/{}", addr).as_str())
        .with_status(500)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.get_balance(&[addr.to_string()]).await;
    assert!(
        result.is_err(),
        "all-addresses-failed should be an error, not zero"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_balance_zero_with_no_addresses() {
    // With no watch addresses there is nothing to fail; a genuine zero is fine.
    let server = Server::new_async().await;
    let client = HnsFansClient::new(&server.url());
    let balance = client
        .get_balance(&[])
        .await
        .expect("empty address set should yield zero balance");
    assert_eq!(balance.confirmed, 0);
    assert_eq!(balance.unconfirmed, 0);
}

#[tokio::test]
async fn test_get_name_info_uses_names_endpoint() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/examplename")
        .with_status(200)
        .with_body(r#"{"name":"examplename","hash":"deadbeef","state":"CLOSED","height":5040,"value":400000,"renewal":329999,"transfer":335606,"revoked":0}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let name = client
        .get_name_info("examplename")
        .await
        .expect("name lookup should succeed");
    assert_eq!(name.name, "examplename");
    assert_eq!(name.name_hash.as_deref(), Some("deadbeef"));
    assert_eq!(name.state.as_deref(), Some("CLOSED"));
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_transactions_returns_empty_in_external_mode() {
    // The explorer has no per-address tx route, so external mode returns an
    // empty list rather than hitting a non-existent endpoint.
    let server = Server::new_async().await;
    let client = HnsFansClient::new(&server.url());
    let txs = client
        .get_transactions(&["hs1qexampleaddress".to_string()])
        .await
        .expect("transactions call should succeed");
    assert_eq!(txs, serde_json::Value::Array(Vec::new()));
}
