use mockito::{Server, Matcher};
use crate::hsd::client::HandshakeClient;

fn create_test_client(server: &Server) -> HandshakeClient {
    HandshakeClient::new(
        &format!("http://{}", server.host_with_port()),
        &format!("http://{}", server.host_with_port()),
        "test-api-key",
        "primary",
    )
}

#[tokio::test]
async fn test_check_connection_success() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", "/wallet/primary")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"wid": 0, "id": "primary", "network": "main", "watchOnly": false}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.check_connection().await;
    assert!(result.is_ok());
    let info = result.unwrap();
    assert_eq!(info.id.as_deref(), Some("primary"));

    mock.assert_async().await;
}

#[tokio::test]
async fn test_check_connection_failure() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", "/wallet/primary")
        .with_status(500)
        .with_body(r#"{"error": "Internal server error"}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.check_connection().await;
    assert!(result.is_err());

    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_balance_success() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", "/wallet/primary/balance")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"confirmed": 1000000, "unconfirmed": 500000, "lockedUnconfirmed": 0, "lockedConfirmed": 0}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.get_balance().await;
    assert!(result.is_ok());
    let balance = result.unwrap();
    assert_eq!(balance.confirmed, 1000000);
    assert_eq!(balance.unconfirmed, 500000);

    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_names_success() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", Matcher::Regex(r"/wallet/primary/name\?own=true".to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"name": "crypto", "state": "CLOSED", "height": 1000, "renewal": 5000, "owner": {"hash": "abc123", "index": 0}}]"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.get_names().await;
    assert!(result.is_ok());
    let names = result.unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name, "crypto");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_names_filters_invalid() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", Matcher::Regex(r"/wallet/primary/name\?own=true".to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[
            {"name": "valid", "owner": {"hash": "abc123", "index": 0}},
            {"name": "invalid1", "owner": {"hash": "0000000000000000000000000000000000000000000000000000000000000000", "index": 0}},
            {"name": "invalid2", "owner": {"hash": "abc123", "index": 4294967295}}
        ]"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.get_names().await;
    assert!(result.is_ok());
    let names = result.unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name, "valid");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_receive_address_success() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", "/wallet/primary/account/default")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name": "default", "receiveAddress": "hs1qtest123", "changeAddress": "hs1qchange"}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.get_receive_address().await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "hs1qtest123");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_receive_address_empty() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", "/wallet/primary/account/default")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name": "default", "receiveAddress": ""}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.get_receive_address().await;
    assert!(result.is_err());

    mock.assert_async().await;
}

#[tokio::test]
async fn test_list_wallets() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", "/wallet")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"["primary", "test", "mine"]"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.list_wallets().await;
    assert!(result.is_ok());
    let wallets = result.unwrap();
    assert_eq!(wallets, vec!["primary", "test", "mine"]);

    mock.assert_async().await;
}

#[tokio::test]
async fn test_create_wallet() {
    let mut server = Server::new_async().await;
    let mock = server.mock("PUT", "/wallet/newwallet")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"id": "newwallet", "network": "main"}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.create_wallet("newwallet", "passphrase", None).await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

#[tokio::test]
async fn test_create_wallet_with_mnemonic() {
    let mut server = Server::new_async().await;
    let mock = server.mock("PUT", "/wallet/imported")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"id": "imported", "network": "main"}"#)
        .match_body(Matcher::JsonString(r#"{"passphrase":"test","watchOnly":false,"mnemonic":"abandon abandon about"}"#.to_string()))
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.create_wallet("imported", "test", Some("abandon abandon about")).await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

#[tokio::test]
async fn test_stop_node() {
    let mut server = Server::new_async().await;
    let mock = server.mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"result": "Stopping.", "error": null}"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.stop_node().await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

#[tokio::test]
async fn test_get_transactions() {
    let mut server = Server::new_async().await;
    let mock = server.mock("GET", Matcher::Regex(r"/wallet/primary/tx/history.*".to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"hash": "abc123", "height": 100, "confirmations": 5}]"#)
        .create_async()
        .await;

    let client = create_test_client(&server);
    let result = client.get_transactions().await;
    assert!(result.is_ok());

    mock.assert_async().await;
}
