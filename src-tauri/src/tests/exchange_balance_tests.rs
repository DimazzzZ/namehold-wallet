//! The BTC account card (`exchange_btc_balance`) against Esplora sources served
//! by `mockito`: what the card shows when one source refuses, and how many
//! requests one read costs.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use super::exchange_fixture as fx;
use crate::commands::exchange::exchange_btc_balance;
use crate::db;
use crate::exchange_io::{env, store};
use crate::noncustodial::exchange::btc_account;
use crate::noncustodial::hd::{self, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::AppState;

const PROFILE: &str = "btcbal";

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_children: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_starting: std::sync::Mutex::new(std::collections::HashSet::new()),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// A regtest recovery-phrase profile with the exchange enabled, reading
/// Bitcoin from `sources` (one URL per line).
fn seeded(sources: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    let m = fx::buyer_master();
    let path = hd::bip44_path(Network::Regtest, 0, 0, 0);
    let account = m.derive_path(&path[..3]).unwrap();
    let xpub = ExtendedPubKey::from_priv(&account).to_base58check(Network::Regtest);
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Bal",
        "mnemonic_hot",
        "regtest",
        &xpub,
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(
        &conn,
        &env::key(env::SOURCES_KEY, Network::Regtest),
        sources,
    )
    .unwrap();
    store::insert_account(
        &conn,
        &store::Account {
            profile_id: PROFILE.into(),
            network: "regtest".into(),
            btc_xpub: btc_account::account_xpub(&m, fx::NET, 0).unwrap(),
            swap_xpub: crate::noncustodial::exchange::keys::branch_xpub(&m, fx::NET, 0).unwrap(),
            account: 0,
            next_trade_index: 0,
            next_btc_receive: 0,
            next_btc_change: 0,
        },
        fx::START,
    )
    .unwrap();
    conn
}

fn first_receive() -> String {
    let xpub = btc_account::account_xpub(&fx::buyer_master(), fx::NET, 0).unwrap();
    btc_account::address_at(&xpub, fx::NET, btc_account::BRANCH_RECEIVE, 0)
        .unwrap()
        .to_string()
}

/// The user's report: the first source (mempool.space) answered 429 Too
/// Many Requests, and the card showed nothing ("Not read") although the
/// second source was there to ask.
#[tokio::test]
async fn a_rate_limited_first_source_does_not_blank_the_btc_account() {
    let mut limited = mockito::Server::new_async().await;
    let busy = limited
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_status(429)
        .with_body("Too Many Requests")
        .expect_at_least(1)
        .create_async()
        .await;
    let mut healthy = mockito::Server::new_async().await;
    let coin = healthy
        .mock("GET", format!("/address/{}/utxo", first_receive()).as_str())
        .with_body(
            r#"[{"txid":"6bb38c22ead17f91a5bc7a217a0d4bd4b0f5f807f8de47179539938585b21929","vout":0,"status":{"confirmed":true,"block_height":120},"value":123789}]"#,
        )
        .create_async()
        .await;
    let empty = healthy
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_body("[]")
        .create_async()
        .await;
    let app = app_with(seeded(&format!("{}\n{}", limited.url(), healthy.url())));
    let b = exchange_btc_balance(app.state()).await.unwrap();
    busy.assert_async().await;
    assert_eq!(b.error, None, "the card: {:?}", b.error);
    assert_eq!(b.confirmed_sats, 123_789);
    coin.assert_async().await;
    let _ = empty;
}

/// What one read costs a source: one request per address the account scans
/// (each branch up to its next index plus the gap limit).
#[tokio::test]
async fn one_read_asks_each_scanned_address_once() {
    let mut src = mockito::Server::new_async().await;
    let all = src
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_body("[]")
        .expect(2 * btc_account::GAP_LIMIT as usize)
        .create_async()
        .await;
    let app = app_with(seeded(&src.url()));
    let b = exchange_btc_balance(app.state()).await.unwrap();
    assert_eq!(b.error, None, "{:?}", b.error);
    all.assert_async().await;
}

/// With every source refusing, the card says so and names each of them.
#[tokio::test]
async fn every_source_refusing_names_each_on_the_card() {
    let mut a = mockito::Server::new_async().await;
    let _a = a
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_status(429)
        .create_async()
        .await;
    let mut b = mockito::Server::new_async().await;
    let _b = b
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_status(503)
        .create_async()
        .await;
    let app = app_with(seeded(&format!("{}\n{}", a.url(), b.url())));
    let got = exchange_btc_balance(app.state()).await.unwrap();
    let e = got.error.expect("an error on the card");
    assert!(e.starts_with("no Bitcoin source could be read: "), "{e}");
    assert!(e.contains(&a.url()) && e.contains("429"), "{e}");
    assert!(e.contains(&b.url()) && e.contains("503"), "{e}");
    assert_eq!(got.confirmed_sats, 0);
}

/// Two reads at once (the page re-opened, Refresh pressed while a read runs)
/// make one scan: the second waits for the first and takes its answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_reads_at_once_scan_the_account_once() {
    let mut src = mockito::Server::new_async().await;
    let all = src
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_body_from_request(|_| {
            // A slow public server: the two reads overlap.
            std::thread::sleep(std::time::Duration::from_millis(5));
            b"[]".to_vec()
        })
        .expect(2 * btc_account::GAP_LIMIT as usize)
        .create_async()
        .await;
    let app = app_with(seeded(&src.url()));
    let (a, b) = tokio::join!(
        exchange_btc_balance(app.state()),
        exchange_btc_balance(app.state())
    );
    assert_eq!(a.unwrap().error, None);
    assert_eq!(b.unwrap().error, None);
    all.assert_async().await;
}

/// A source that answers 429 once, with Retry-After, is asked again after the
/// pause it names instead of failing the whole read.
#[tokio::test]
async fn a_429_with_retry_after_is_asked_again_once() {
    let mut src = mockito::Server::new_async().await;
    let first = src
        .mock("GET", format!("/address/{}/utxo", first_receive()).as_str())
        .with_status(429)
        .with_header("retry-after", "0")
        .expect(1)
        .create_async()
        .await;
    let rest = src
        .mock("GET", mockito::Matcher::Regex(r"^/address/.*/utxo$".into()))
        .with_body("[]")
        .create_async()
        .await;
    let app = app_with(seeded(&src.url()));
    let b = exchange_btc_balance(app.state()).await.unwrap();
    assert_eq!(b.error, None, "the card: {:?}", b.error);
    first.assert_async().await;
    let _ = rest;
}
