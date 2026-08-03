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

// --- explorer_client_from_settings: the single factory (Task 11 / S1) ---

#[tokio::test]
async fn test_explorer_client_from_settings_uses_configured_url() {
    // A custom `explorer_api_url` in settings must be the URL the factory's
    // client actually talks to — not the hard-coded default.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/customname")
        .with_status(200)
        .with_body(r#"{"name":"customname","state":"CLOSED"}"#)
        .create_async()
        .await;

    let mut settings = std::collections::HashMap::new();
    settings.insert("explorer_api_url".to_string(), server.url());
    let client = crate::providers::explorer_client_from_settings(&settings);

    let name = client
        .get_name_info_optional("customname")
        .await
        .expect("lookup should succeed")
        .expect("name should be found");
    assert_eq!(name.name, "customname");
    mock.assert_async().await;
}

// See `providers::hnsfans::tests::explorer_client_from_settings_defaults_when_blank_or_missing`
// for the base_url-level assertion of the missing/blank-key fallback (needs
// field access to `HnsFansClient::base_url`, private outside the module).

#[tokio::test]
async fn test_health_succeeds_on_probe_endpoint() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/txs")
        .match_query(mockito::Matcher::UrlEncoded("limit".into(), "1".into()))
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

// --- get_name_info_optional: AVAILABLE normalization tests ---

#[tokio::test]
async fn test_get_name_info_optional_returns_none_on_404() {
    // When the explorer returns 404 for a name, get_name_info_optional should
    // return Ok(None) so the caller can synthesize an AVAILABLE response.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/neveropened")
        .with_status(404)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client
        .get_name_info_optional("neveropened")
        .await
        .expect("404 should not be an error");
    assert!(
        result.is_none(),
        "404 should yield None for AVAILABLE synthesis"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_name_info_optional_returns_none_on_empty_body() {
    // An empty or unparseable body from a 200 response should also yield None
    // so the caller treats the name as AVAILABLE.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/emptyname")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client
        .get_name_info_optional("emptyname")
        .await
        .expect("empty body should not be an error");
    assert!(
        result.is_none(),
        "empty body should yield None for AVAILABLE synthesis"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_name_info_optional_propagates_5xx_errors() {
    // Real server errors (5xx) must propagate as Err, NOT be silently treated
    // as AVAILABLE. This is critical to avoid false "name is available" when
    // the explorer is simply down.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/servererror")
        .with_status(500)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.get_name_info_optional("servererror").await;
    assert!(
        result.is_err(),
        "5xx should be an error, not None/AVAILABLE"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_name_info_optional_errors_on_unrecognized_200_shape() {
    // Task 11 / S1: a 200 whose body has CONTENT but not the expected `name`
    // field (the explorer's contract drifted — e.g. renamed the field) must
    // be a loud, typed error, NOT `Ok(None)` — the latter is indistinguishable
    // from "name genuinely not found" and silently degrades ownership checks
    // to "you own nothing".
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/driftedname")
        .with_status(200)
        .with_body(r#"{"domain":"driftedname","status":"ok"}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.get_name_info_optional("driftedname").await;
    match result {
        Err(crate::error::AppError::ExplorerFormat(_)) => {}
        other => panic!("expected ExplorerFormat error, got {other:?}"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_name_info_optional_returns_some_for_known_name() {
    // A normal name with valid data should return Some(ChainName), not None.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/knownname")
        .with_status(200)
        .with_body(r#"{"name":"knownname","hash":"cafebabe","state":"CLOSED","height":5040,"value":400000,"renewal":329999,"transfer":0,"revoked":0}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client
        .get_name_info_optional("knownname")
        .await
        .expect("valid name should succeed");
    let name = result.expect("known name should be Some");
    assert_eq!(name.name, "knownname");
    assert_eq!(name.state.as_deref(), Some("CLOSED"));
    mock.assert_async().await;
}

// --- get_name_current_owner (history): loud degradation (Task 11 / S1) ---

#[tokio::test]
async fn test_get_name_current_owner_returns_none_on_recognized_empty_history() {
    // The documented, recognized "no history yet" shape: a `result` array
    // that's simply empty. This is a legitimate "not owned", not an error.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/neverowned/history")
        .with_status(200)
        .with_body(r#"{"result":[]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client
        .get_name_current_owner("neverowned")
        .await
        .expect("empty recognized history should not be an error");
    assert!(result.is_none());
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_name_current_owner_errors_on_unrecognized_200_shape() {
    // The explorer answers 200 but the body isn't the `{ "result": [...] }`
    // (or bare-array) shape at all — the contract drifted. Must be a loud
    // typed error, not a silent "no owner" that makes repair/discover think
    // every owned name was transferred away.
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/driftedhistory/history")
        .with_status(200)
        .with_body(r#"{"entries":[{"txid":"aa","index":0}]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.get_name_current_owner("driftedhistory").await;
    match result {
        Err(crate::error::AppError::ExplorerFormat(_)) => {}
        other => panic!("expected ExplorerFormat error, got {other:?}"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_name_current_owner_returns_owner_for_recognized_history() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/names/owned/history")
        .with_status(200)
        .with_body(r#"{"result":[{"action":"Finalize","txid":"aa","index":32}]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client
        .get_name_current_owner("owned")
        .await
        .expect("recognized history should succeed");
    assert_eq!(result, Some(("aa".to_string(), 32)));
    mock.assert_async().await;
}

// --- get_address_txids / get_tx_named_outputs: loud degradation ---

#[tokio::test]
async fn test_get_address_txids_errors_on_unrecognized_200_shape() {
    let mut server = Server::new_async().await;
    let addr = "hs1qexampleaddress";
    let mock = server
        .mock("GET", "/api/txs")
        .match_query(mockito::Matcher::UrlEncoded("address".into(), addr.into()))
        .with_status(200)
        .with_body(r#"{"unexpected":"payload"}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.get_address_txids(addr, 25, 0).await;
    match result {
        Err(crate::error::AppError::ExplorerFormat(_)) => {}
        other => panic!("expected ExplorerFormat error, got {other:?}"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_address_txids_returns_empty_on_recognized_empty_result() {
    let mut server = Server::new_async().await;
    let addr = "hs1qexampleaddress";
    let mock = server
        .mock("GET", "/api/txs")
        .match_query(mockito::Matcher::UrlEncoded("address".into(), addr.into()))
        .with_status(200)
        .with_body(r#"{"limit":25,"offset":0,"total":0,"result":[]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let (hashes, total) = client
        .get_address_txids(addr, 25, 0)
        .await
        .expect("recognized empty result should not be an error");
    assert!(hashes.is_empty());
    assert_eq!(total, 0);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_tx_named_outputs_errors_on_unrecognized_200_shape() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/txs/drifted")
        .with_status(200)
        .with_body(r#"{"vout":[]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let result = client.get_tx_named_outputs("drifted").await;
    match result {
        Err(crate::error::AppError::ExplorerFormat(_)) => {}
        other => panic!("expected ExplorerFormat error, got {other:?}"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_tx_named_outputs_returns_empty_on_recognized_empty_outputs() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/api/txs/plain")
        .with_status(200)
        .with_body(r#"{"outputs":[]}"#)
        .create_async()
        .await;

    let client = HnsFansClient::new(&server.url());
    let outs = client
        .get_tx_named_outputs("plain")
        .await
        .expect("recognized empty outputs should not be an error");
    assert!(outs.is_empty());
    mock.assert_async().await;
}
