//! Command-level tests for the Shakedex market commands: browse and import.
//! They drive the real `#[tauri::command]` functions over a migrated
//! in-memory DB, with `mockito` standing in for both the profile's node
//! (JSON-RPC `POST /` and REST `GET /coin/<hash>/<index>`) and LearnHNS Market
//! (through the debug/test-only `learnhns_base_url` seam).

use mockito::{Matcher, Mock, ServerGuard};
use rusqlite::params;
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::shakedex::{shakedex_import_listing, shakedex_list_market, ImportSource};
use crate::db;
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::shakedex::template::{step_sighash, StepTemplate};
use crate::noncustodial::tx::output_address_from_string;
use crate::AppState;

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
pub(crate) const PROFILE: &str = "shk1";
pub(crate) const FUND_TXID: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";
/// 1000 HNS: enough for the 435 HNS fixture listing plus fees.
pub(crate) const FUNDS: u64 = 1_000_000_000;

pub(crate) const LISTING_FILE: &str =
    include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");
const PROOF_COOLJOBS: &str = include_str!("../../tests/vectors/shakedex/proof_cooljobs.json");
const COIN: &str = include_str!("../../tests/vectors/shakedex/coin_dexreviews.json");
const NAME: &str = include_str!("../../tests/vectors/shakedex/name_dexreviews.json");
const LOCK_TXID: &str = "c44db193e4c815969db27870f49805cf6838a3f0ef700165e4d3c3660650abea";
const COOLJOBS_TXID: &str = "b0fdf437d88f96cc7ff13082e4f7e536f3d6c6be053b76dbb29e0071202a830a";

// --- wallet fixtures --------------------------------------------------------

fn seed() -> [u8; 64] {
    hd::seed_from_mnemonic(MNEMONIC, "").unwrap()
}

pub(crate) fn account_xpub(net: Network) -> ExtendedPubKey {
    let master = ExtendedPrivKey::from_seed(&seed()).unwrap();
    let path = hd::bip44_path(net, 0, 0, 0);
    ExtendedPubKey::from_priv(&master.derive_path(&path[..3]).unwrap())
}

/// In-memory DB with one funded profile of `kind` on `network` (a profile
/// network string such as "mainnet" or "regtest"), pointed at `node_url`.
pub(crate) fn seeded(network: &str, kind: &str, node_url: &str) -> rusqlite::Connection {
    let net = derivation::network_from_profile(network).unwrap();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Shakedex",
        kind,
        network,
        &account_xpub(net).to_base58check(net),
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", node_url).unwrap();
    let (_sk, pk, addr) = hd::derive_address(net, &seed(), 0, 0, 0).unwrap();
    let spk = hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap());
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, ?3, ?4)",
        params![PROFILE, addr, spk, hex::encode(pk)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, 0, 'liquid_hns', NULL)",
        params![FUND_TXID, PROFILE, addr, spk, FUNDS as i64],
    )
    .unwrap();
    conn
}

fn mainnet(node_url: &str) -> rusqlite::Connection {
    let conn = seeded("mainnet", "mnemonic_hot", node_url);
    set(&conn, "shakedex_experimental", "true");
    conn
}

pub(crate) fn set(conn: &rusqlite::Connection, key: &str, value: &str) {
    db::queries::set_setting(conn, key, value).unwrap();
}

pub(crate) fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
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

// --- node mock --------------------------------------------------------------

pub(crate) fn rpc_ok(result: Value) -> String {
    json!({ "result": result, "error": null, "id": 1 }).to_string()
}

pub(crate) async fn mock_blockchain_info(s: &mut ServerGuard, tip: i64, mtp: Option<u64>) -> Mock {
    let mut info = json!({ "chain": "main", "blocks": tip, "headers": tip });
    if let Some(m) = mtp {
        info["mediantime"] = m.into();
    }
    s.mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({ "method": "getblockchaininfo" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(info))
        .create_async()
        .await
}

pub(crate) async fn mock_name_info(s: &mut ServerGuard, name_info: Value) -> Mock {
    s.mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({ "method": "getnameinfo" })))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(name_info))
        .create_async()
        .await
}

pub(crate) async fn mock_coin(s: &mut ServerGuard, txid: &str, coin: Value) -> Mock {
    s.mock("GET", format!("/coin/{txid}/0").as_str())
        .with_header("content-type", "application/json")
        .with_body(coin.to_string())
        .create_async()
        .await
}

fn fixture_coin() -> Value {
    serde_json::from_str::<Value>(COIN).unwrap()["coin"].clone()
}

/// `getnameinfo`'s result: `{"info": {...}, "start": ...}`.
pub(crate) fn fixture_name() -> Value {
    serde_json::from_str::<Value>(NAME).unwrap()["nameInfo"].clone()
}

fn fixture_tip() -> i64 {
    fixture_name()["info"]["renewal"].as_i64().unwrap() + 1_000
}

fn fixture_mtp() -> u64 {
    let proof: Value = serde_json::from_str(LISTING_FILE).unwrap();
    proof["data"][0]["lockTime"].as_u64().unwrap() + 10_000
}

/// A node on which the dexreviews fixture listing is live and buyable.
async fn live_node(s: &mut ServerGuard) -> Vec<Mock> {
    vec![
        mock_blockchain_info(s, fixture_tip(), Some(fixture_mtp())).await,
        mock_name_info(s, fixture_name()).await,
        mock_coin(s, LOCK_TXID, fixture_coin()).await,
    ]
}

// --- market mock ------------------------------------------------------------

async fn mock_market_page(s: &mut ServerGuard, listings: &[&str]) -> Mock {
    let auctions: Vec<Value> = listings
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    s.mock(
        "GET",
        "/api/v2/auctions?availability=available&page=1&per_page=100",
    )
    .with_header("content-type", "application/json")
    .with_body(json!({ "auctions": auctions, "total": listings.len() }).to_string())
    .create_async()
    .await
}

pub(crate) fn err_text(e: crate::error::AppError) -> String {
    e.to_string()
}

// --- regtest listing, signed here -------------------------------------------

/// A regtest listing signed with a throwaway lock key, plus the lock coin and
/// `getnameinfo` result that make it buyable at `REGTEST_TIP`/`REGTEST_MTP`.
pub(crate) struct RegtestListing {
    pub(crate) json: String,
    pub(crate) txid: String,
    pub(crate) coin: Value,
    pub(crate) name_info: Value,
}

pub(crate) const REGTEST_TIP: i64 = 150;
pub(crate) const REGTEST_MTP: u64 = 1_700_000_000;

/// A regtest listing over a lock coin of `lock_value` doos, one signed step
/// per `(price, lock_time)`.
pub(crate) fn regtest_listing_with(lock_value: u64, steps: &[(u64, u64)]) -> RegtestListing {
    use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
    let net = Network::Regtest;
    let name = "regname";
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
    let pubkey = PublicKey::from_secret_key(&secp, &sk).serialize();
    let lock_txid = [0xab; 32];
    let payment = address::encode_p2wpkh(net, &[9; 20]).unwrap();
    let data: Vec<Value> = steps
        .iter()
        .map(|&(price, lock_time)| {
            let t = StepTemplate {
                lock_outpoint: (lock_txid, 0),
                lock_value,
                lock_pubkey: &pubkey,
                payment: output_address_from_string(net, &payment).unwrap(),
                price,
                lock_time_secs: lock_time,
            };
            let msg = Message::from_digest(step_sighash(&t).unwrap());
            let mut sig = secp.sign_ecdsa(&msg, &sk).serialize_compact().to_vec();
            sig.push(0x84);
            json!({ "price": price, "lockTime": lock_time, "signature": hex::encode(sig), "fee": 0 })
        })
        .collect();
    let txid = hex::encode(lock_txid);
    let json = json!({
        "name": name,
        "lockingTxHash": txid,
        "lockingOutputIdx": 0,
        "publicKey": hex::encode(pubkey),
        "paymentAddr": payment,
        "feeAddr": null,
        "data": data,
        "version": 2
    })
    .to_string();
    let name_height = 50u32;
    let coin = json!({
        "hash": txid,
        "index": 0,
        "value": lock_value,
        "address": lock_address(net, &pubkey).unwrap(),
        "height": 100,
        "coinbase": false,
        "version": 0,
        "covenant": {
            "type": 10,
            "action": "FINALIZE",
            "items": [
                hex::encode(crate::noncustodial::names::hash_name(name).unwrap()),
                hex::encode(name_height.to_le_bytes()),
                hex::encode(name.as_bytes()),
                "00", "00000000", "03000000",
                "00".repeat(32)
            ]
        }
    });
    let name_info = json!({
        "info": {
            "name": name,
            "height": name_height,
            "renewal": 100,
            "renewals": 0,
            "claimed": 0,
            "weak": false,
            "owner": { "hash": txid, "index": 0 },
            "value": lock_value
        },
        "start": null
    });
    RegtestListing {
        json,
        txid,
        coin,
        name_info,
    }
}

// --- browse -----------------------------------------------------------------

#[tokio::test]
async fn list_market_survives_one_lookup_failure() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let _bad = node
        .mock("GET", format!("/coin/{COOLJOBS_TXID}/0").as_str())
        .with_status(500)
        .with_body(r#"{"error":{"message":"boom"}}"#)
        .create_async()
        .await;
    let mut market = mockito::Server::new_async().await;
    let _p = mock_market_page(&mut market, &[LISTING_FILE, PROOF_COOLJOBS]).await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let page = shakedex_list_market(app.state(), None).await.expect("page");
    assert!(page.network_has_market);
    assert!(page.verified);
    assert_eq!(page.rows.len(), 1);
    let row = &page.rows[0];
    assert_eq!(row.name, "dexreviews");
    assert_eq!(serde_json::to_value(row).unwrap()["kind"], "buyNow");
    assert_eq!(row.current_price, Some(435_000_000));
    assert_eq!(row.floor_price, 435_000_000);
    assert_eq!(page.hidden.could_not_check, 1);
    assert_eq!(page.hidden_rows.len(), 1);
    assert_eq!(page.hidden_rows[0].name.as_deref(), Some("cooljobs"));
    let h = serde_json::to_value(&page.hidden_rows[0]).unwrap();
    assert_eq!(h["reason"]["kind"], "couldNotCheck");
    assert!(h["reason"]["reason"].as_str().unwrap().contains("boom"));
    assert_eq!(page.hidden.sold_or_cancelled, 0);
    assert_eq!(page.hidden.failed_verification, 0);
    let v = serde_json::to_value(row).unwrap();
    assert_eq!(v["verdict"]["verdict"], "buyable");
    assert_eq!(v["steps"][0]["lockTime"], 1_783_696_480u64);
}

/// The page exactly as the live Market served it (ISO `expiresAt`, `bids`,
/// `createdAt`, `id`, ...): the row the node can check is listed, the two it
/// cannot are "could not check", and none fails verification on its shape.
#[tokio::test]
async fn list_market_reads_the_live_feed_shape() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let _p = market
        .mock(
            "GET",
            "/api/v2/auctions?availability=available&page=1&per_page=100",
        )
        .with_header("content-type", "application/json")
        .with_body(include_str!(
            "../../tests/vectors/shakedex/market_auctions_page.json"
        ))
        .create_async()
        .await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let page = shakedex_list_market(app.state(), None).await.expect("page");
    assert_eq!(page.hidden.failed_verification, 0, "{:?}", page.hidden_rows);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].name, "dexreviews");
    assert_eq!(page.rows[0].expires_at, Some(1_815_232_480));
    assert_eq!(page.hidden.could_not_check, 2);
}

/// The live Market listed 268 listings, a page holds 100: the list says which
/// page it is and how many there are, so the screen can offer the rest
/// (R5, R8) instead of silently showing the first hundred.
#[tokio::test]
async fn list_market_says_which_page_of_how_many() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let _p2 = market
        .mock(
            "GET",
            "/api/v2/auctions?availability=available&page=2&per_page=100",
        )
        .with_header("content-type", "application/json")
        .with_body(include_str!(
            "../../tests/vectors/shakedex/market_auctions_page.json"
        ))
        .expect(1)
        .create_async()
        .await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let page = shakedex_list_market(app.state(), Some(2))
        .await
        .expect("page");

    _p2.assert_async().await;
    let v = serde_json::to_value(&page).unwrap();
    assert_eq!(v["page"], 2);
    assert_eq!(v["pageCount"], 3);
}

/// No listings is still one (empty) page, and a market off mainnet has one.
#[tokio::test]
async fn list_market_has_at_least_one_page() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let _p = mock_market_page(&mut market, &[]).await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let v = serde_json::to_value(shakedex_list_market(app.state(), None).await.unwrap()).unwrap();
    assert_eq!(
        (v["page"].clone(), v["pageCount"].clone()),
        (json!(1), json!(1))
    );
}

#[tokio::test]
async fn list_market_reads_chain_info_once_per_page() {
    let mut node = mockito::Server::new_async().await;
    let info = mock_blockchain_info(&mut node, fixture_tip(), Some(fixture_mtp()))
        .await
        .expect(1);
    let _n = mock_name_info(&mut node, fixture_name()).await;
    let _c = mock_coin(&mut node, LOCK_TXID, fixture_coin()).await;
    let mut market = mockito::Server::new_async().await;
    let _p = mock_market_page(&mut market, &[LISTING_FILE, LISTING_FILE, LISTING_FILE]).await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let page = shakedex_list_market(app.state(), None).await.expect("page");
    assert_eq!(page.rows.len(), 3);
    info.assert_async().await;
}

#[tokio::test]
async fn list_market_without_mtp_hides_every_row_as_could_not_check() {
    let mut node = mockito::Server::new_async().await;
    let _b = mock_blockchain_info(&mut node, fixture_tip(), None).await;
    let _n = mock_name_info(&mut node, fixture_name()).await;
    let _c = mock_coin(&mut node, LOCK_TXID, fixture_coin()).await;
    let mut market = mockito::Server::new_async().await;
    let _p = mock_market_page(&mut market, &[LISTING_FILE, PROOF_COOLJOBS]).await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let page = shakedex_list_market(app.state(), None).await.expect("page");
    assert!(page.rows.is_empty());
    assert_eq!(page.hidden.could_not_check, 2);
    let names: Vec<&str> = page
        .hidden_rows
        .iter()
        .filter_map(|h| h.name.as_deref())
        .collect();
    assert_eq!(names, ["dexreviews", "cooljobs"], "market order is kept");
}

#[tokio::test]
async fn list_market_counts_unparsable_rows_as_failed() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let _p = mock_market_page(
        &mut market,
        &[
            LISTING_FILE,
            r#"{"version":1}"#,
            r#"{"name":"oldname","version":1}"#,
        ],
    )
    .await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let page = shakedex_list_market(app.state(), Some(1)).await.unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.hidden.failed_verification, 2);
    let names: Vec<Option<&str>> = page.hidden_rows.iter().map(|r| r.name.as_deref()).collect();
    assert_eq!(names, vec![None, Some("oldname")]);
    let first = serde_json::to_value(&page.hidden_rows[0]).unwrap();
    assert!(first["name"].is_null(), "a row without a name says so");
    for r in &page.hidden_rows {
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(v["reason"]["kind"], "failedVerification");
        assert!(v["reason"]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Invalid input: listing file:"));
    }
}

#[tokio::test]
async fn list_market_unverified_in_spv() {
    let mut node = mockito::Server::new_async().await;
    let rpc = node.mock("POST", "/").expect(0).create_async().await;
    let mut market = mockito::Server::new_async().await;
    let _p = mock_market_page(&mut market, &[LISTING_FILE, PROOF_COOLJOBS]).await;
    let conn = mainnet(&node.url());
    set(&conn, "node_mode", "spv");
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);

    let page = shakedex_list_market(app.state(), None).await.unwrap();
    assert!(!page.verified);
    assert_eq!(page.rows.len(), 2);
    for row in &page.rows {
        let v = serde_json::to_value(row).unwrap();
        assert_eq!(
            v["verdict"],
            json!({ "verdict": "hidden", "kind": "unverified" })
        );
        assert_eq!(row.current_price, None);
    }
    rpc.assert_async().await;
}

#[tokio::test]
async fn list_market_off_mainnet_has_no_market() {
    let node = mockito::Server::new_async().await;
    let app = app_with(seeded("regtest", "mnemonic_hot", &node.url()));
    let page = shakedex_list_market(app.state(), None).await.unwrap();
    assert!(!page.network_has_market);
    assert!(page.rows.is_empty());
}

// --- import -----------------------------------------------------------------

#[tokio::test]
async fn import_link_only_learnhns() {
    let node = mockito::Server::new_async().await;
    let app = app_with(mainnet(&node.url()));
    let err = shakedex_import_listing(
        app.state(),
        ImportSource::Link {
            url: "https://evil.example/listing/x".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(err_text(err).contains("market.learnhns.com"));
}

#[tokio::test]
async fn import_link_fetches_listing_file() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let _p = market
        .mock("GET", "/listing/dexreviews/proof.json")
        .with_body(LISTING_FILE)
        .create_async()
        .await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let row = shakedex_import_listing(
        app.state(),
        ImportSource::Link {
            url: "https://market.learnhns.com/listing/dexreviews".into(),
        },
    )
    .await
    .expect("import");
    assert_eq!(row.name, "dexreviews");
    assert_eq!(row.current_price, Some(435_000_000));
}

/// The link names the listing the user asked for: a file for another name
/// is refused, not shown under the link's name or in its place.
#[tokio::test]
async fn import_link_refuses_a_file_for_another_name() {
    let node = mockito::Server::new_async().await;
    let mut market = mockito::Server::new_async().await;
    let _p = market
        .mock("GET", "/listing/othername/proof.json")
        .with_body(LISTING_FILE)
        .create_async()
        .await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let err = shakedex_import_listing(
        app.state(),
        ImportSource::Link {
            url: "https://market.learnhns.com/listing/othername".into(),
        },
    )
    .await
    .unwrap_err();
    let msg = err_text(err);
    assert!(
        msg.contains("othername") && msg.contains("dexreviews"),
        "{msg}"
    );
}

#[tokio::test]
async fn import_text_and_file() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let app = app_with(mainnet(&node.url()));
    let row = shakedex_import_listing(
        app.state(),
        ImportSource::Text {
            json: LISTING_FILE.into(),
        },
    )
    .await
    .expect("text import");
    assert_eq!(row.name, "dexreviews");

    let dir = std::env::temp_dir().join(format!("shakedex-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("listing.json");
    std::fs::write(&ok, LISTING_FILE).unwrap();
    let row = shakedex_import_listing(
        app.state(),
        ImportSource::File {
            path: ok.to_string_lossy().into_owned(),
        },
    )
    .await
    .expect("file import");
    assert_eq!(row.name, "dexreviews");

    let big = dir.join("big.json");
    std::fs::write(
        &big,
        " ".repeat(crate::noncustodial::shakedex::listing_file::MAX_LISTING_FILE_BYTES + 1),
    )
    .unwrap();
    let err = shakedex_import_listing(
        app.state(),
        ImportSource::File {
            path: big.to_string_lossy().into_owned(),
        },
    )
    .await
    .unwrap_err();
    assert!(err_text(err).contains("larger than"));

    let err = shakedex_import_listing(
        app.state(),
        ImportSource::File {
            path: dir.to_string_lossy().into_owned(),
        },
    )
    .await
    .unwrap_err();
    assert!(err_text(err).contains("not a regular file"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_source_is_tagged_by_kind() {
    let s: ImportSource = serde_json::from_value(json!({ "kind": "link", "url": "u" })).unwrap();
    assert!(matches!(s, ImportSource::Link { url } if url == "u"));
    let s: ImportSource = serde_json::from_value(json!({ "kind": "file", "path": "p" })).unwrap();
    assert!(matches!(s, ImportSource::File { path } if path == "p"));
    let s: ImportSource = serde_json::from_value(json!({ "kind": "text", "json": "{}" })).unwrap();
    assert!(matches!(s, ImportSource::Text { json } if json == "{}"));
}

/// The UI disables link import with the backend's own sentence (§Frontend:
/// "the backend error uses the same words"). The two copies live in two
/// languages, so this reads the UI's file and requires the backend sentence
/// there verbatim, as a string literal.
#[test]
fn refusal_sentences_match_the_uis_word_for_word() {
    const UI: &str = include_str!("../../../src/components/market/marketText.ts");
    // Prettier may break a long constant after `=`: compare without that.
    let ui = UI.replace("=\n  \"", "= \"");
    let (name, sentence) = (
        "MARKET_MAINNET_ONLY",
        crate::commands::shakedex::MARKET_MAINNET_ONLY,
    );
    let decl = format!("export const {name} = {sentence:?};");
    assert!(ui.contains(&decl), "marketText.ts lacks `{decl}`");
}
