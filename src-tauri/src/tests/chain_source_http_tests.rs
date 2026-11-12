//! What the node and explorer clients do with a server that does not behave:
//! a redirect is not followed, an oversized reply is refused, and a loopback
//! URL goes past any proxy, to `127.0.0.1` when it says `localhost`.

use crate::noncustodial::rpc::ChainSource;
use crate::noncustodial::rpc::{NodeRpcClient, NODE_REPLY_MAX_BYTES};
use crate::providers::hnsfans::{HnsFansClient, EXPLORER_REPLY_MAX_BYTES};
use mockito::{Matcher, Server};
use serial_test::serial;

const KEY: &str = "secret-node-key";

fn node(url: &str) -> NodeRpcClient {
    NodeRpcClient::try_new(url, KEY, ChainSource::LocalNode).unwrap()
}

/// A 3xx from the node is an answer, not a place to go: nothing reaches the
/// target, so neither the api-key nor the signed transaction in the body
/// does, and the call fails.
#[tokio::test]
async fn the_node_client_does_not_follow_a_redirect() {
    let mut target = Server::new_async().await;
    let not_called = target
        .mock("POST", Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    let mut node_server = Server::new_async().await;
    node_server
        .mock("POST", "/")
        .with_status(307)
        .with_header("location", &format!("{}/", target.url()))
        .create_async()
        .await;

    node(&node_server.url())
        .get_blockchain_info()
        .await
        .expect_err("a redirect is not an answer");
    not_called.assert_async().await;
}

/// The same for a REST read: a GET redirect would carry the address to the
/// target, and on a same-host target reqwest would keep the api-key.
#[tokio::test]
async fn the_node_client_does_not_follow_a_redirect_on_a_rest_read() {
    let mut node_server = Server::new_async().await;
    let not_called = node_server
        .mock("GET", "/elsewhere")
        .expect(0)
        .create_async()
        .await;
    node_server
        .mock("GET", "/coin/address/hs1qa")
        .with_status(302)
        .with_header("location", "/elsewhere")
        .create_async()
        .await;

    node(&node_server.url())
        .get_coins_by_address("hs1qa")
        .await
        .expect_err("a redirect is not an answer");
    not_called.assert_async().await;
}

#[tokio::test]
async fn the_explorer_client_does_not_follow_a_redirect() {
    let mut server = Server::new_async().await;
    let not_called = server
        .mock("GET", "/elsewhere")
        .expect(0)
        .create_async()
        .await;
    server
        .mock("GET", "/api/names/alice")
        .with_status(302)
        .with_header("location", "/elsewhere")
        .create_async()
        .await;

    HnsFansClient::new(&server.url())
        .get_name_info("alice")
        .await
        .expect_err("a redirect is not an answer");
    not_called.assert_async().await;
}

/// Over the cap the reply is refused for every kind of node call, whether or
/// not it announces its length.
#[tokio::test]
async fn the_node_client_refuses_an_oversized_reply() {
    let huge = " ".repeat(NODE_REPLY_MAX_BYTES + 1);
    let mut server = Server::new_async().await;
    server
        .mock("POST", "/")
        .with_body(&huge)
        .create_async()
        .await;
    server
        .mock("GET", "/coin/address/hs1qa")
        .with_body(&huge)
        .create_async()
        .await;
    // No Content-Length: only counting the bytes can refuse this one.
    server
        .mock("GET", "/tx/address/hs1qa")
        .with_chunked_body(move |w| {
            for _ in 0..=(NODE_REPLY_MAX_BYTES / (1024 * 1024)) {
                w.write_all(&vec![b' '; 1024 * 1024])?;
            }
            Ok(())
        })
        .create_async()
        .await;
    // A 404 with an endless body cannot read as hsd's empty one either.
    server
        .mock("GET", "/coin/aa/0")
        .with_status(404)
        .with_body(&huge)
        .create_async()
        .await;

    let client = node(&server.url());
    let rpc = client.get_blockchain_info().await.unwrap_err().to_string();
    assert!(rpc.contains("larger than"), "{rpc}");
    let coins = client
        .get_coins_by_address("hs1qa")
        .await
        .unwrap_err()
        .to_string();
    assert!(coins.contains("larger than"), "{coins}");
    let txs = client
        .get_txs_by_address("hs1qa")
        .await
        .unwrap_err()
        .to_string();
    assert!(txs.contains("larger than"), "{txs}");
    let coin = client.get_coin("aa", 0).await.unwrap_err().to_string();
    assert!(coin.contains("larger than"), "{coin}");
}

#[tokio::test]
async fn the_explorer_client_refuses_an_oversized_reply() {
    let mut server = Server::new_async().await;
    server
        .mock("GET", "/api/names/alice")
        .with_body(" ".repeat(EXPLORER_REPLY_MAX_BYTES + 1))
        .create_async()
        .await;
    let err = HnsFansClient::new(&server.url())
        .get_name_info("alice")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("larger than"), "{err}");
}

/// Every explorer read holds to the cap: one read left unbounded is all a
/// hostile explorer needs.
#[tokio::test]
async fn every_explorer_read_refuses_an_oversized_reply() {
    let mut server = Server::new_async().await;
    server
        .mock("GET", Matcher::Any)
        .with_body(" ".repeat(EXPLORER_REPLY_MAX_BYTES + 1))
        .create_async()
        .await;
    let client = HnsFansClient::new(&server.url());
    let errors = [
        (
            "get_balance",
            client.get_balance(&["hs1qa".to_string()]).await.err(),
        ),
        ("get_name_info", client.get_name_info("a").await.err()),
        (
            "get_name_info_optional",
            client.get_name_info_optional("a").await.err(),
        ),
        (
            "get_address_txids",
            client.get_address_txids("hs1qa", 10, 0).await.err(),
        ),
        (
            "get_tx_named_outputs",
            client.get_tx_named_outputs("aa").await.err(),
        ),
        (
            "get_name_current_owner",
            client.get_name_current_owner("a").await.err(),
        ),
    ];
    for (read, err) in errors {
        let err = err.unwrap_or_else(|| panic!("{read} took it")).to_string();
        assert!(err.contains("larger than"), "{read}: {err}");
    }
}

/// A reply within the cap reads as before.
#[tokio::test]
async fn a_reply_within_the_cap_is_read() {
    let mut server = Server::new_async().await;
    server
        .mock("POST", "/")
        .with_body(r#"{"result":{"blocks":7},"error":null,"id":null}"#)
        .create_async()
        .await;
    let info = node(&server.url()).get_blockchain_info().await.unwrap();
    assert_eq!(info.blocks, 7);
}

/// The environment proxy does not see a loopback node: reqwest has no
/// loopback exemption of its own, so without `no_proxy` this request, with
/// the api-key in it, goes to the proxy.
#[tokio::test]
#[serial(http_proxy_env)]
async fn a_loopback_node_url_goes_past_the_environment_proxy() {
    let mut proxy = Server::new_async().await;
    let proxy_not_called = proxy
        .mock("POST", Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    let mut node_server = Server::new_async().await;
    node_server
        .mock("POST", "/")
        .with_body(r#"{"result":{"blocks":7},"error":null,"id":null}"#)
        .create_async()
        .await;

    let saved = std::env::var_os("HTTP_PROXY");
    let saved_no = std::env::var_os("NO_PROXY");
    std::env::set_var("HTTP_PROXY", proxy.url());
    std::env::remove_var("NO_PROXY");
    let client = node(&node_server.url());
    let reply = client.get_blockchain_info().await;
    match saved {
        Some(v) => std::env::set_var("HTTP_PROXY", v),
        None => std::env::remove_var("HTTP_PROXY"),
    }
    if let Some(v) = saved_no {
        std::env::set_var("NO_PROXY", v);
    }

    assert_eq!(reply.unwrap().blocks, 7);
    proxy_not_called.assert_async().await;
}

/// `localhost` connects to `127.0.0.1` whatever the resolver says. A listener
/// on `::1` alone is what an unpinned `localhost` reaches on a system that
/// resolves it there, and this client must not.
#[tokio::test]
async fn localhost_connects_to_the_ipv4_loopback_and_nowhere_else() {
    let Ok(v6) = std::net::TcpListener::bind("[::1]:0") else {
        return; // No IPv6 loopback here: nothing to tell apart.
    };
    v6.set_nonblocking(true).unwrap();
    let port = v6.local_addr().unwrap().port();

    let err = node(&format!("http://localhost:{port}"))
        .get_blockchain_info()
        .await;
    assert!(err.is_err(), "nothing listens on 127.0.0.1:{port}");
    assert!(
        v6.accept().is_err(),
        "the request went to ::1, where the resolver pointed"
    );

    // And it does reach a node on 127.0.0.1 by that name.
    let mut server = Server::new_async().await;
    server
        .mock("POST", "/")
        .with_body(r#"{"result":{"blocks":7},"error":null,"id":null}"#)
        .create_async()
        .await;
    let url = server.url().replace("127.0.0.1", "localhost");
    assert_eq!(node(&url).get_blockchain_info().await.unwrap().blocks, 7);
}

/// A remote URL keeps the policy for redirects, and is not a loopback host by
/// a name that merely starts like one.
#[test]
fn only_this_machine_counts_as_loopback() {
    for (url, want) in [
        ("http://localhost:1", true),
        ("http://LOCALHOST:1", true),
        ("http://127.0.0.1:1", true),
        ("http://127.9.9.9:1", true),
        ("http://[::1]:1", true),
        ("http://localhost.evil.example:1", false),
        ("http://127.0.0.1.evil.example:1", false),
        ("http://192.168.1.5:1", false),
    ] {
        let parsed = url::Url::parse(url).unwrap();
        assert_eq!(crate::http::is_loopback_url(&parsed), want, "{url}");
    }
}

// ------------------------------------------------- the exchange's servers
//
// The relay, the watchtower and the Bitcoin sources are not trusted to send
// the wallet elsewhere either: the settings, and the secure window before
// them, name the hosts it talks to.

mod exchange {
    use crate::exchange_io::btc_source::{BitcoinCore, BtcSource, Esplora};
    use crate::exchange_io::relay_client::RelayClient;
    use mockito::{Matcher, Server};

    /// A relay's 3xx to a read is not followed, even to a reply in the
    /// relay's own shape.
    #[tokio::test]
    async fn the_relay_client_does_not_follow_a_redirect_on_a_read() {
        let mut target = Server::new_async().await;
        let not_called = target
            .mock("GET", Matcher::Any)
            .with_body(r#"{"offers":[],"messages":[]}"#)
            .expect(0)
            .create_async()
            .await;
        let mut relay = Server::new_async().await;
        relay
            .mock("GET", Matcher::Any)
            .with_status(302)
            .with_header("location", &format!("{}/v1/offers", target.url()))
            .create_async()
            .await;

        let client = RelayClient::new(&relay.url()).unwrap();
        client
            .offers("regtest", 0)
            .await
            .expect_err("a redirect is not an answer");
        client
            .inbox("02aa", "regtest", None)
            .await
            .expect_err("a redirect is not an answer");
        not_called.assert_async().await;
    }

    /// Nor to a send: a 307 would repeat the body at the target.
    #[tokio::test]
    async fn the_relay_client_does_not_follow_a_redirect_on_a_send() {
        let mut target = Server::new_async().await;
        let not_called = target
            .mock("POST", Matcher::Any)
            .with_body("{}")
            .expect(0)
            .create_async()
            .await;
        let mut relay = Server::new_async().await;
        relay
            .mock("POST", Matcher::Any)
            .with_status(307)
            .with_header("location", &format!("{}/", target.url()))
            .create_async()
            .await;

        RelayClient::new(&relay.url())
            .unwrap()
            .withdraw_offer("00", "00")
            .await
            .expect_err("a redirect is not an answer");
        not_called.assert_async().await;
    }

    #[tokio::test]
    async fn the_esplora_client_does_not_follow_a_redirect() {
        let mut target = Server::new_async().await;
        let not_called = target
            .mock("GET", Matcher::Any)
            .with_body("00")
            .expect(0)
            .create_async()
            .await;
        let mut source = Server::new_async().await;
        source
            .mock("GET", Matcher::Any)
            .with_status(302)
            .with_header("location", &format!("{}/blocks/tip/hash", target.url()))
            .create_async()
            .await;

        Esplora::new(&source.url())
            .unwrap()
            .tip()
            .await
            .expect_err("a redirect is not an answer");
        not_called.assert_async().await;
    }

    /// A 307 on the same host would carry the RPC login along with the call.
    #[tokio::test]
    async fn the_bitcoind_client_does_not_follow_a_redirect() {
        let mut source = Server::new_async().await;
        let not_called = source
            .mock("POST", "/elsewhere")
            .with_body(r#"{"result":{"txindex":{"synced":true}},"error":null,"id":"namehold"}"#)
            .expect(0)
            .create_async()
            .await;
        source
            .mock("POST", "/")
            .with_status(307)
            .with_header("location", "/elsewhere")
            .create_async()
            .await;

        BitcoinCore::with_login(&format!("bitcoind+{}", source.url()), "user", "pass")
            .unwrap()
            .ready()
            .await
            .expect_err("a redirect is not an answer");
        not_called.assert_async().await;
    }
}
