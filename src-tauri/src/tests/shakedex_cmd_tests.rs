//! Command-level tests for the Shakedex buying commands: browse, import,
//! preview and build a purchase. They drive the real `#[tauri::command]`
//! functions over a migrated in-memory DB, with `mockito` standing in for both
//! the profile's node (JSON-RPC `POST /` and REST `GET /coin/<hash>/<index>`)
//! and LearnHNS Market (through the debug/test-only `learnhns_base_url` seam).

use mockito::{Matcher, Mock, ServerGuard};
use rusqlite::params;
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::shakedex::{
    shakedex_build_purchase_draft, shakedex_import_listing, shakedex_list_market,
    shakedex_preview_purchase, ImportSource,
};
use crate::db;
use crate::error::AppError;
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

pub(crate) fn with_db<T>(
    app: &tauri::App<tauri::test::MockRuntime>,
    f: impl FnOnce(&rusqlite::Connection) -> T,
) -> T {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    f(&conn)
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

async fn mock_fee_info(s: &mut ServerGuard, rate: u32, addr: &str) -> Mock {
    s.mock("GET", "/api/v2/fee_info")
        .with_header("content-type", "application/json")
        .with_body(json!({ "rate": rate, "address": addr }).to_string())
        .create_async()
        .await
}

fn mainnet_addr(byte: u8) -> String {
    address::encode_p2wpkh(Network::Main, &[byte; 20]).unwrap()
}

/// The fixture listing with a 1% market fee to `fee_addr` (fee and feeAddr
/// are not covered by the seller's signature, so it still verifies).
fn listing_with_fee(fee_addr: &str) -> String {
    let mut j: Value = serde_json::from_str(LISTING_FILE).unwrap();
    let price = j["data"][0]["price"].as_u64().unwrap();
    j["data"][0]["fee"] = (price / 100).into();
    j["feeAddr"] = fee_addr.into();
    j.to_string()
}

fn plan_outputs(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) -> Vec<Value> {
    with_db(app, |c| {
        let row = db::queries::get_tx_draft(c, draft_id).unwrap().unwrap();
        let plan: Value = serde_json::from_str(&row.signing_inputs_json).unwrap();
        plan["outputs"].as_array().unwrap().clone()
    })
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

/// The default regtest listing: one step of 5 HNS, valid well before
/// `REGTEST_MTP`, over a lock coin of 1 HNS.
pub(crate) fn regtest_listing() -> RegtestListing {
    regtest_listing_with(1_000_000, &[(5_000_000, REGTEST_MTP - 100_000)])
}

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

// --- build: gates -----------------------------------------------------------

#[tokio::test]
async fn build_purchase_refused_without_write_capability() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let conn = mainnet(&node.url());
    set(&conn, "chain_source", "explorer");
    let app = app_with(conn);
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains(crate::commands::shakedex::NEEDS_SENDING_NODE),
        "the sentence the UI shows on the disabled Buy"
    );
    with_db(&app, |c| {
        assert!(db::queries::list_open_shakedex_purchases(c, PROFILE)
            .unwrap()
            .is_empty());
    });
}

#[tokio::test]
async fn watch_only_profile_is_refused_with_the_software_wallet_sentence() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let conn = seeded("mainnet", "watch_only_xpub", &node.url());
    conn.execute(
        "UPDATE wallet_profiles SET watch_only = 1 WHERE id = ?1",
        params![PROFILE],
    )
    .unwrap();
    set(&conn, "shakedex_experimental", "true");
    let app = app_with(conn);
    let sentence = crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY;
    let err = shakedex_preview_purchase(app.state(), LISTING_FILE.into(), false, false, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains(sentence), "preview");
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains(sentence), "build");
}

#[tokio::test]
async fn mainnet_purchase_needs_experimental_flag() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let conn = seeded("mainnet", "mnemonic_hot", &node.url());
    let app = app_with(conn);
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains(crate::noncustodial::shakedex::MAINNET_EXPERIMENTAL),
        "the sentence the UI shows on the disabled Buy"
    );

    with_db(&app, |c| set(c, "shakedex_experimental", "true"));
    let draft = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .expect("flag set: purchase builds");
    assert_eq!(draft.action, "shakedex_purchase");
}

#[tokio::test]
async fn regtest_purchase_ignores_flag() {
    let r = regtest_listing();
    let mut node = mockito::Server::new_async().await;
    let _b = mock_blockchain_info(&mut node, REGTEST_TIP, Some(REGTEST_MTP)).await;
    let _n = mock_name_info(&mut node, r.name_info.clone()).await;
    let _c = mock_coin(&mut node, &r.txid, r.coin.clone()).await;
    let app = app_with(seeded("regtest", "mnemonic_hot", &node.url()));
    let draft = shakedex_build_purchase_draft(app.state(), r.json.clone(), None, false, None)
        .await
        .expect("regtest purchase needs no flag");
    assert_eq!(draft.action, "shakedex_purchase");
    assert_eq!(draft.summary["priceDoos"], 5_000_000);
}

#[tokio::test]
async fn ledger_profile_refused() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let conn = seeded("mainnet", "ledger_hardware", &node.url());
    set(&conn, "shakedex_experimental", "true");
    let app = app_with(conn);
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("recovery-phrase wallet"));
}

#[tokio::test]
async fn second_purchase_of_same_lock_refused() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let app = app_with(mainnet(&node.url()));
    shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .expect("first purchase builds");
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("already in progress"));
    with_db(&app, |c| {
        assert_eq!(
            db::queries::list_open_shakedex_purchases(c, PROFILE)
                .unwrap()
                .len(),
            1
        );
        let drafts: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM wallet_tx_drafts WHERE action = 'shakedex_purchase'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(drafts, 1, "the refused attempt leaves no draft behind");
    });
}

#[tokio::test]
async fn purchase_can_be_rebuilt_after_its_unsent_draft_is_discarded() {
    // Cancelling the secure prompt discards the draft: the listing must be
    // buyable again at once, without waiting for a sync.
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let app = app_with(mainnet(&node.url()));
    let first = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .expect("first purchase builds");
    with_db(&app, |c| {
        db::queries::delete_tx_draft(c, &first.id).unwrap()
    });
    let second = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .expect("the listing is buyable again");
    with_db(&app, |c| {
        let open = db::queries::list_open_shakedex_purchases(c, PROFILE).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].purchase_draft_id, second.id);
    });
}

#[tokio::test]
async fn purchase_refused_without_mtp() {
    let mut node = mockito::Server::new_async().await;
    let _b = mock_blockchain_info(&mut node, fixture_tip(), None).await;
    let _n = mock_name_info(&mut node, fixture_name()).await;
    let _c = mock_coin(&mut node, LOCK_TXID, fixture_coin()).await;
    let app = app_with(mainnet(&node.url()));
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("median time"));
}

#[tokio::test]
async fn sold_listing_refused() {
    let mut node = mockito::Server::new_async().await;
    let _b = mock_blockchain_info(&mut node, fixture_tip(), Some(fixture_mtp())).await;
    let _c = node
        .mock("GET", format!("/coin/{LOCK_TXID}/0").as_str())
        .with_status(404)
        .with_body("")
        .create_async()
        .await;
    let app = app_with(mainnet(&node.url()));
    let err = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("sold or cancelled"));
}

// --- build: what is written -------------------------------------------------

#[tokio::test]
async fn draft_reserves_only_own_inputs_and_records_purchase() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let app = app_with(mainnet(&node.url()));
    let draft = shakedex_build_purchase_draft(app.state(), LISTING_FILE.into(), None, false, None)
        .await
        .expect("purchase builds");

    let fresh = derivation::derive_one(
        Network::Main,
        &account_xpub(Network::Main),
        derivation::BRANCH_RECEIVE,
        1,
    )
    .unwrap()
    .address;
    with_db(&app, |c| {
        let mut stmt = c
            .prepare("SELECT txid FROM tracked_utxos WHERE reserved_by_draft_id = ?1")
            .unwrap();
        let reserved: Vec<String> = stmt
            .query_map(params![draft.id], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(reserved, vec![FUND_TXID.to_string()]);

        let open = db::queries::list_open_shakedex_purchases(c, PROFILE).unwrap();
        assert_eq!(open.len(), 1);
        let p = &open[0];
        assert_eq!(p.state, crate::db::queries::PurchaseState::PendingSend);
        assert_eq!(p.name, "dexreviews");
        assert_eq!(p.lock_txid, LOCK_TXID);
        assert_eq!(p.lock_vout, 0);
        assert_eq!(p.price_doos, 435_000_000);
        assert_eq!(p.purchase_draft_id, draft.id);
        assert_eq!(p.destination_address, fresh);
        assert_eq!(
            Some(p.purchase_txid.as_str()),
            draft.summary["txid"].as_str()
        );
    });

    let s = &draft.summary;
    assert_eq!(s["action"], "shakedex_purchase");
    assert_eq!(
        s["finalizeWait"],
        "after a finalize, 288 blocks (about 2 days) after the purchase is mined"
    );
    assert_eq!(s["name"], "dexreviews");
    assert_eq!(s["priceDoos"], 435_000_000);
    assert_eq!(s["marketFeeDoos"], 0);
    assert_eq!(s["sendTotalDoos"], 435_000_000);
    let fee = s["feeDoos"].as_i64().unwrap();
    assert!(fee > 0);
    assert_eq!(s["totalDoos"].as_i64().unwrap(), 435_000_000 + fee);
    // Our own coins only: the lock coin's value is the name's, not money spent.
    assert_eq!(
        s["inputTotalDoos"].as_i64().unwrap(),
        s["sendTotalDoos"].as_i64().unwrap() + fee + s["changeDoos"].as_i64().unwrap()
    );
    assert_eq!(s["inputTotalDoos"].as_i64().unwrap(), FUNDS as i64);
    let payment = "hs1qgjy2cj8st8zaenkejej3urvx0msxm6l8q8erep";
    assert_eq!(s["recipientAddress"], payment);
    let outs = plan_outputs(&app, &draft.id);
    assert_eq!(outs.last().unwrap()["address"], payment);
    assert_eq!(outs.last().unwrap()["value"], 435_000_000);
}

// --- market fee -------------------------------------------------------------

#[tokio::test]
async fn foreign_market_fee_is_not_paid_by_default() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let foreign = mainnet_addr(7);
    let _f = mock_fee_info(&mut market, 100, &mainnet_addr(8)).await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let listing = listing_with_fee(&foreign);

    let preview = shakedex_preview_purchase(app.state(), listing.clone(), false, true, None)
        .await
        .expect("preview");
    let fee = preview.market_fee.as_ref().expect("the listing asks a fee");
    assert!(!fee.published);
    assert!(fee.payable);
    assert_eq!(fee.value_doos, 4_350_000);
    assert_eq!(fee.percent_text.as_deref(), Some("1.00%"));
    assert!(fee
        .warning
        .as_deref()
        .unwrap()
        .contains("not signed by the seller"));
    assert_eq!(
        preview.total_doos,
        preview.price_doos + preview.network_fee_doos
    );

    let draft = shakedex_build_purchase_draft(app.state(), listing, None, true, None)
        .await
        .expect("build");
    let outs = plan_outputs(&app, &draft.id);
    assert!(outs.iter().all(|o| o["address"] != foreign.as_str()));
    assert_eq!(draft.summary["marketFeeDoos"], 0);
}

#[tokio::test]
async fn unusable_fee_address_is_never_paid() {
    // A regtest fee address on a mainnet listing: the listing stays
    // buyable, the fee line explains that no fee is paid.
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let app = app_with(mainnet(&node.url()));
    let regtest_fee_addr = address::encode_p2wpkh(Network::Regtest, &[7; 20]).unwrap();
    let listing = listing_with_fee(&regtest_fee_addr);

    let preview = shakedex_preview_purchase(app.state(), listing.clone(), true, false, None)
        .await
        .expect("preview");
    let fee = preview.market_fee.as_ref().expect("the listing asks a fee");
    assert!(!fee.payable);
    assert!(fee
        .warning
        .as_deref()
        .unwrap()
        .contains("no valid fee address"));
    assert_eq!(
        preview.total_doos,
        preview.price_doos + preview.network_fee_doos
    );

    let draft = shakedex_build_purchase_draft(app.state(), listing, None, false, None)
        .await
        .expect("build");
    assert_eq!(draft.summary["marketFeeDoos"], 0);
    assert_eq!(
        plan_outputs(&app, &draft.id).len(),
        3,
        "TRANSFER, change, payment"
    );
}

#[tokio::test]
async fn published_market_fee_is_paid_when_chosen() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let fee_addr = mainnet_addr(7);
    let _f = mock_fee_info(&mut market, 100, &fee_addr).await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let listing = listing_with_fee(&fee_addr);

    let preview = shakedex_preview_purchase(app.state(), listing.clone(), true, true, None)
        .await
        .expect("preview");
    let fee = preview.market_fee.as_ref().unwrap();
    assert!(fee.published);
    assert!(fee.payable);
    assert!(fee.warning.is_none());
    assert_eq!(
        preview.total_doos,
        preview.price_doos + 4_350_000 + preview.network_fee_doos
    );

    let draft = shakedex_build_purchase_draft(app.state(), listing, Some(4_350_000), true, None)
        .await
        .expect("build");
    let outs = plan_outputs(&app, &draft.id);
    let paid: Vec<&Value> = outs
        .iter()
        .filter(|o| o["address"] == fee_addr.as_str())
        .collect();
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0]["value"], 4_350_000);
    assert_eq!(draft.summary["marketFeeDoos"], 4_350_000);
    assert_eq!(draft.summary["sendTotalDoos"], 435_000_000 + 4_350_000);
}

#[tokio::test]
async fn fee_from_a_file_is_never_published() {
    let mut node = mockito::Server::new_async().await;
    let _m = live_node(&mut node).await;
    let mut market = mockito::Server::new_async().await;
    let fee_addr = mainnet_addr(7);
    let f = market
        .mock("GET", "/api/v2/fee_info")
        .with_body(json!({ "rate": 100, "address": fee_addr }).to_string())
        .expect(0)
        .create_async()
        .await;
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let preview =
        shakedex_preview_purchase(app.state(), listing_with_fee(&fee_addr), false, false, None)
            .await
            .expect("preview");
    assert!(!preview.market_fee.unwrap().published);
    f.assert_async().await;
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

/// There is no LearnHNS Market off mainnet (R15): a link import is refused
/// with the sentence the UI shows, before anything is fetched.
#[tokio::test]
async fn import_link_off_mainnet_is_refused_before_fetching() {
    let node = mockito::Server::new_async().await;
    let mut market = mockito::Server::new_async().await;
    let fetch = market
        .mock("GET", "/listing/dexreviews/proof.json")
        .with_body(LISTING_FILE)
        .expect(0)
        .create_async()
        .await;
    let conn = seeded("regtest", "mnemonic_hot", &node.url());
    set(&conn, "learnhns_base_url", &market.url());
    let app = app_with(conn);
    let err = shakedex_import_listing(
        app.state(),
        ImportSource::Link {
            url: "https://market.learnhns.com/listing/dexreviews".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(err_text(err).contains(crate::commands::shakedex::MARKET_MAINNET_ONLY));
    fetch.assert_async().await;
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

// --- secure confirmation ----------------------------------------------------

/// A stored purchase draft carrying `summary`, for the secure window's rows.
fn purchase_draft_row(summary: &Value) -> db::queries::TxDraftRow {
    db::queries::TxDraftRow {
        id: "d1".into(),
        wallet_profile_id: PROFILE.into(),
        action: "shakedex_purchase".into(),
        unsigned_tx_hex: String::new(),
        signed_tx_hex: None,
        signing_inputs_json: "{}".into(),
        summary_json: summary.to_string(),
        status: "draft".into(),
        error_message: None,
        txid: None,
        confirmation_height: None,
        created_at: String::new(),
    }
}

/// A complete purchase summary, as `shakedex_build_purchase_draft` stores it.
fn purchase_summary() -> Value {
    json!({
        "action": "shakedex_purchase",
        "name": "dexreviews",
        "priceDoos": 435_000_000,
        "marketFeeDoos": 4_350_000,
        "marketFeeAddress": "hs1qgjy2cj8st8zaenkejej3urvx0msxm6l8q8erep",
        "feeDoos": 2_500,
        "totalDoos": 439_352_500,
        "sendTotalDoos": 439_350_000,
        "changeDoos": 0,
        "inputTotalDoos": 439_352_500,
        "numInputs": 2,
        "recipientAddress": "hs1qgjy2cj8st8zaenkejej3urvx0msxm6l8q8erep",
        "paymentAddress": "hs1qgjy2cj8st8zaenkejej3urvx0msxm6l8q8erep",
        "destinationAddress": "hs1qgjy2cj8st8zaenkejej3urvx0msxm6l8q8erep",
        "finalizeWait": "after a finalize, 288 blocks (about 2 days) after the purchase is mined",
        "txid": "ab".repeat(32),
        "warnings": ["finalize it in time"]
    })
}

fn confirm_rows(summary: &Value) -> Vec<Value> {
    crate::commands::tx::confirm_details_for_draft(&purchase_draft_row(summary)).unwrap()["rows"]
        .as_array()
        .unwrap()
        .clone()
}

fn row_value(rows: &[Value], label: &str) -> Option<String> {
    rows.iter()
        .find(|r| r["label"] == label)
        .and_then(|r| r["value"].as_str())
        .map(str::to_owned)
}

#[test]
fn confirm_rows_for_purchase() {
    let rows = confirm_rows(&purchase_summary());
    let value = |label: &str| row_value(&rows, label);
    assert_eq!(value("Name").as_deref(), Some("dexreviews"));
    assert_eq!(
        value("Price (to seller)").as_deref(),
        Some("435.000000 HNS")
    );
    assert_eq!(
        value("Market fee").as_deref(),
        Some("4.350000 HNS (1.00% of the price)")
    );
    assert_eq!(value("Network fee").as_deref(), Some("0.002500 HNS"));
    assert_eq!(value("Total").as_deref(), Some("439.352500 HNS"));
    assert_eq!(
        value("Name becomes yours").as_deref(),
        Some("after a finalize, 288 blocks (about 2 days) after the purchase is mined")
    );
    assert_eq!(value("Warning").as_deref(), Some("finalize it in time"));
    // The generic covenant rows would show the name's own value as spent.
    assert!(value("Fee").is_none());

    let mut no_fee = purchase_summary();
    no_fee["marketFeeDoos"] = 0.into();
    no_fee["marketFeeAddress"] = Value::Null;
    assert!(confirm_rows(&no_fee)
        .iter()
        .all(|r| r["label"] != "Market fee"));
}

/// The summary's amounts are u64: the window renders every one of them
/// exactly, never through a cast that turns a large one negative.
#[test]
fn confirm_rows_render_amounts_above_i64() {
    let mut s = purchase_summary();
    let big = u64::MAX;
    s["priceDoos"] = big.into();
    s["totalDoos"] = big.into();
    s["feeDoos"] = (big - 1).into();
    let rows = confirm_rows(&s);
    let value = |label: &str| row_value(&rows, label);
    assert_eq!(
        value("Price (to seller)").as_deref(),
        Some("18446744073709.551615 HNS")
    );
    assert_eq!(value("Total").as_deref(), Some("18446744073709.551615 HNS"));
    assert_eq!(
        value("Network fee").as_deref(),
        Some("18446744073709.551614 HNS")
    );
}

/// The window shows the wait the draft was built with, for its network
/// (`purchase::finalize_wait_text`), and refuses a summary without it rather
/// than show an empty row.
#[test]
fn confirm_rows_show_the_stored_finalize_wait() {
    let mut s = purchase_summary();
    s["finalizeWait"] = "after a finalize, 10 blocks after the purchase is mined".into();
    assert_eq!(
        row_value(&confirm_rows(&s), "Name becomes yours").as_deref(),
        Some("after a finalize, 10 blocks after the purchase is mined")
    );
    s.as_object_mut().unwrap().remove("finalizeWait");
    assert!(crate::commands::tx::confirm_details_for_draft(&purchase_draft_row(&s)).is_err());
}

#[test]
fn confirm_refuses_a_purchase_summary_it_cannot_read() {
    // The secure window is what the user trusts: it must never show a
    // purchase as costing 0 HNS because its summary is missing a field.
    let mut missing_total = purchase_summary();
    missing_total.as_object_mut().unwrap().remove("totalDoos");
    for summary in [json!({}), missing_total, json!("not a summary")] {
        let err = crate::commands::tx::confirm_details_for_draft(&purchase_draft_row(&summary))
            .unwrap_err();
        assert!(
            matches!(&err, AppError::Other(m) if m.contains("corrupted draft")),
            "{err:?}"
        );
    }
}

// --- market fee consent -----------------------------------------------------

/// The secure window's rows for a draft as the build stored it — which also
/// proves the stored summary reads back as a typed one.
fn stored_confirm_rows(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) -> Vec<Value> {
    with_db(app, |c| {
        let row = db::queries::get_tx_draft(c, draft_id).unwrap().unwrap();
        crate::commands::tx::confirm_details_for_draft(&row).unwrap()["rows"]
            .as_array()
            .unwrap()
            .clone()
    })
}

/// A mainnet app whose market publishes `published_addr` as its fee address.
async fn fee_app(
    node: &mut ServerGuard,
    market: &mut ServerGuard,
    published_addr: &str,
) -> (tauri::App<tauri::test::MockRuntime>, Vec<Mock>) {
    let mut mocks = live_node(node).await;
    mocks.push(mock_fee_info(market, 100, published_addr).await);
    let conn = mainnet(&node.url());
    set(&conn, "learnhns_base_url", &market.url());
    (app_with(conn), mocks)
}

#[tokio::test]
async fn unpublished_fee_paid_is_flagged_in_the_secure_window() {
    let mut node = mockito::Server::new_async().await;
    let mut market = mockito::Server::new_async().await;
    let foreign = mainnet_addr(7);
    let (app, _m) = fee_app(&mut node, &mut market, &mainnet_addr(8)).await;
    let draft = shakedex_build_purchase_draft(
        app.state(),
        listing_with_fee(&foreign),
        Some(4_350_000),
        true,
        None,
    )
    .await
    .expect("the user may still choose to pay it");
    assert_eq!(draft.summary["marketFeeDoos"], 4_350_000);
    assert_eq!(draft.summary["marketFeeAddress"], foreign.as_str());
    assert!(draft.summary["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("not signed by the seller")));

    let rows = stored_confirm_rows(&app, &draft.id);
    let value = |label: &str| {
        rows.iter()
            .filter(|r| r["label"] == label)
            .map(|r| r["value"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(value("Market fee to"), vec![foreign.clone()]);
    assert!(value("Warning")
        .iter()
        .any(|w| w.contains("not signed by the seller")));
}

#[tokio::test]
async fn published_fee_paid_shows_its_address_without_warning() {
    let mut node = mockito::Server::new_async().await;
    let mut market = mockito::Server::new_async().await;
    let fee_addr = mainnet_addr(7);
    let (app, _m) = fee_app(&mut node, &mut market, &fee_addr).await;
    let draft = shakedex_build_purchase_draft(
        app.state(),
        listing_with_fee(&fee_addr),
        Some(4_350_000),
        true,
        None,
    )
    .await
    .expect("matching accepted fee builds");
    assert!(draft.summary["warnings"].as_array().unwrap().is_empty());
    let rows = stored_confirm_rows(&app, &draft.id);
    assert!(rows
        .iter()
        .any(|r| r["label"] == "Market fee to" && r["value"] == fee_addr.as_str()));
    assert!(rows.iter().all(|r| r["label"] != "Warning"));
}

#[tokio::test]
async fn accepted_market_fee_mismatch_refused() {
    let mut node = mockito::Server::new_async().await;
    let mut market = mockito::Server::new_async().await;
    let fee_addr = mainnet_addr(7);
    let (app, _m) = fee_app(&mut node, &mut market, &fee_addr).await;
    let err = shakedex_build_purchase_draft(
        app.state(),
        listing_with_fee(&fee_addr),
        Some(4_000_000),
        true,
        None,
    )
    .await
    .unwrap_err();
    assert!(err_text(err).contains("the market fee changed since you reviewed it"));

    // A fee accepted for a listing that no longer asks one is a change too.
    let err = shakedex_build_purchase_draft(
        app.state(),
        LISTING_FILE.into(),
        Some(4_350_000),
        true,
        None,
    )
    .await
    .unwrap_err();
    assert!(err_text(err).contains("the market fee changed since you reviewed it"));
    with_db(&app, |c| {
        assert!(db::queries::list_open_shakedex_purchases(c, PROFILE)
            .unwrap()
            .is_empty());
    });
}

#[test]
fn draft_insert_in_caller_transaction_rolls_back_with_it() {
    let conn = mainnet("http://127.0.0.1:1");
    {
        let tx = conn.unchecked_transaction().unwrap();
        db::queries::insert_tx_draft_reserving_coins_in_tx(
            &tx,
            "d-rollback",
            PROFILE,
            "shakedex_purchase",
            "",
            "{}",
            "{}",
            &[(FUND_TXID.to_string(), 0)],
        )
        .unwrap();
        // Dropped without commit, as when the purchase row insert fails.
    }
    assert!(db::queries::get_tx_draft(&conn, "d-rollback")
        .unwrap()
        .is_none());
    let reserved: Option<String> = conn
        .query_row(
            "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = ?1",
            params![FUND_TXID],
            |r| r.get(0),
        )
        .unwrap();
    assert!(reserved.is_none());
}

// --- pre-broadcast re-check of the price step (R10) --------------------------

/// A regtest node mock answering `getblockchaininfo` on chain "regtest".
async fn mock_regtest_info(s: &mut ServerGuard, mtp: Option<u64>) -> Mock {
    let mut info = json!({ "chain": "regtest", "blocks": REGTEST_TIP, "headers": REGTEST_TIP });
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

async fn mock_send(s: &mut ServerGuard, hits: usize) -> Mock {
    s.mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({ "method": "sendrawtransaction" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!("ab".repeat(32))))
        .expect(hits)
        .create_async()
        .await
}

/// A reverse auction on regtest: 9 HNS valid now, 5 HNS valid from
/// `REGTEST_MTP + 1024`. Built at `REGTEST_MTP` (paying 9 HNS) and marked
/// signed, ready for `broadcast_tx_draft`.
async fn signed_reverse_auction_purchase(
    node: &mut ServerGuard,
) -> (tauri::App<tauri::test::MockRuntime>, String, Mock) {
    let r = regtest_listing_with(
        1_000_000,
        &[
            (9_000_000, REGTEST_MTP - 100_000),
            (5_000_000, REGTEST_MTP + 1024),
        ],
    );
    let info = mock_regtest_info(node, Some(REGTEST_MTP)).await;
    let _n = mock_name_info(node, r.name_info.clone()).await;
    let _c = mock_coin(node, &r.txid, r.coin.clone()).await;
    let app = app_with(seeded("regtest", "mnemonic_hot", &node.url()));
    let draft = shakedex_build_purchase_draft(app.state(), r.json.clone(), None, false, None)
        .await
        .expect("purchase builds");
    assert_eq!(draft.summary["priceDoos"], 9_000_000);
    with_db(&app, |c| {
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'signed', signed_tx_hex = '00' WHERE id = ?1",
            params![draft.id],
        )
        .unwrap();
    });
    (app, draft.id, info)
}

fn draft_status(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> String {
    with_db(app, |c| {
        db::queries::get_tx_draft(c, id).unwrap().unwrap().status
    })
}

/// A purchase refused before sending leaves nothing behind: no draft, no
/// purchase record, no reserved coin. The listing can be reviewed again.
fn assert_unsent_purchase_discarded(app: &tauri::App<tauri::test::MockRuntime>, id: &str) {
    with_db(app, |c| {
        assert!(
            db::queries::get_tx_draft(c, id).unwrap().is_none(),
            "the draft is discarded"
        );
        assert!(
            db::queries::get_shakedex_purchase_by_draft(c, id)
                .unwrap()
                .is_none(),
            "the purchase record goes with it"
        );
        let reserved: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reserved, 0, "its coins are released");
    });
}

#[tokio::test]
async fn broadcast_refused_when_a_cheaper_step_became_valid() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    info.remove_async().await;
    let _later = mock_regtest_info(&mut node, Some(REGTEST_MTP + 2_000)).await;
    let send = mock_send(&mut node, 0).await;

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("the price changed — review the purchase again"),
        "refused before sending"
    );
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

/// R10: "if it changed ... the draft is discarded". The median time went back
/// below the paid step's lock time (a reorg, a node behind): hsd would refuse
/// the purchase as non-final, so it is not sent at all.
#[tokio::test]
async fn broadcast_refused_when_the_paid_step_is_no_longer_valid() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    info.remove_async().await;
    let _earlier = mock_regtest_info(&mut node, Some(REGTEST_MTP - 200_000)).await;
    let send = mock_send(&mut node, 0).await;

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("the price changed — review the purchase again"),
        "refused before sending"
    );
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

/// The re-check tells the paid step by its lock time, not its price. The
/// shakedex CLI writes a listing whose steps all share one price when a
/// reverse auction starts and ends at the same price. Here our purchase pays
/// the later of two such steps; the median time then goes back between them,
/// so only the earlier step is valid. Its price is ours, but hsd would take
/// our purchase only as non-final: nothing is sent.
#[tokio::test]
async fn broadcast_refused_when_only_another_step_at_the_same_price_is_valid() {
    let mut node = mockito::Server::new_async().await;
    let r = regtest_listing_with(
        1_000_000,
        &[
            (5_000_000, REGTEST_MTP - 50_000),
            (5_000_000, REGTEST_MTP - 100_000),
        ],
    );
    let info = mock_regtest_info(&mut node, Some(REGTEST_MTP)).await;
    let _n = mock_name_info(&mut node, r.name_info.clone()).await;
    let _c = mock_coin(&mut node, &r.txid, r.coin.clone()).await;
    let app = app_with(seeded("regtest", "mnemonic_hot", &node.url()));
    let draft = shakedex_build_purchase_draft(app.state(), r.json.clone(), None, false, None)
        .await
        .expect("purchase builds");
    let plan_lock_time = with_db(&app, |c| {
        let row = db::queries::get_tx_draft(c, &draft.id).unwrap().unwrap();
        crate::noncustodial::shakedex::purchase::plan_lock_time(&row.signing_inputs_json).unwrap()
    });
    assert_eq!(
        plan_lock_time,
        crate::noncustodial::shakedex::template::encode_lock_time(REGTEST_MTP - 50_000).unwrap(),
        "the purchase pays the later step"
    );
    with_db(&app, |c| {
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'signed', signed_tx_hex = '00' WHERE id = ?1",
            params![draft.id],
        )
        .unwrap();
    });
    info.remove_async().await;
    let _between = mock_regtest_info(&mut node, Some(REGTEST_MTP - 75_000)).await;
    let send = mock_send(&mut node, 0).await;

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("the price changed — review the purchase again"),
        "refused before sending"
    );
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft.id);
}

#[tokio::test]
async fn retry_of_a_maybe_sent_purchase_keeps_it_and_says_why() {
    // The first attempt hit a transport error, so the node may hold the
    // purchase. A retry refused by the price re-check must neither discard
    // that draft nor hide the refusal behind "cannot delete".
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    with_db(&app, |c| {
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'broadcast_pending' WHERE id = ?1",
            params![draft_id],
        )
        .unwrap();
    });
    info.remove_async().await;
    let _later = mock_regtest_info(&mut node, Some(REGTEST_MTP + 2_000)).await;
    let send = mock_send(&mut node, 0).await;

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("the price changed — review the purchase again"),
        "the re-check's own reason reaches the user"
    );
    send.assert_async().await;
    assert_eq!(draft_status(&app, &draft_id), "broadcast_pending");
    with_db(&app, |c| {
        assert!(
            db::queries::get_shakedex_purchase_by_draft(c, &draft_id)
                .unwrap()
                .is_some(),
            "a purchase the node may hold stays tracked"
        );
    });
}

#[tokio::test]
async fn broadcast_refused_when_the_recorded_price_is_unreadable() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    info.remove_async().await;
    // A cheaper step is valid: a check that read the paid price as 0 would
    // wave the purchase through.
    let _later = mock_regtest_info(&mut node, Some(REGTEST_MTP + 2_000)).await;
    let send = mock_send(&mut node, 0).await;
    with_db(&app, |c| {
        c.execute(
            "UPDATE shakedex_purchases SET price_doos = -1 WHERE purchase_draft_id = ?1",
            params![draft_id],
        )
        .unwrap();
    });

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("the purchase was not sent"),
        "fails closed"
    );
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

#[tokio::test]
async fn broadcast_refused_without_median_time() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    info.remove_async().await;
    let _later = mock_regtest_info(&mut node, None).await;
    let send = mock_send(&mut node, 0).await;

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(err_text(err).contains("node did not report median time"));
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

/// A proxy's error page where the node's answer should be, just before the
/// send: the price cannot be re-checked, so nothing is sent and the purchase
/// is discarded for the user to review again (R10, "or the node cannot say").
#[tokio::test]
async fn broadcast_refused_when_the_node_answers_with_a_proxy_error() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    info.remove_async().await;
    let _proxy = node
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({ "method": "getblockchaininfo" }),
        ))
        .with_status(502)
        .with_header("content-type", "text/html")
        .with_body("<html><body>502 Bad Gateway</body></html>")
        .create_async()
        .await;
    let send = mock_send(&mut node, 0).await;

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("502"),
        "the proxy's status is reported"
    );
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

#[tokio::test]
async fn broadcast_refused_when_the_purchase_is_no_longer_recorded() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, _info) = signed_reverse_auction_purchase(&mut node).await;
    let send = mock_send(&mut node, 0).await;
    with_db(&app, |c| {
        c.execute(
            "DELETE FROM shakedex_purchases WHERE purchase_draft_id = ?1",
            params![draft_id],
        )
        .unwrap();
    });

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(err_text(err).contains("this purchase is no longer recorded"));
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

/// A purchase draft older than its coin reservation is refused before
/// sending, and discarded: past the TTL its coins may already fund another
/// draft, and the sync deletes an unsent draft that old. Refusing at the TTL
/// leaves every send that started a margin before the sync's deletion.
#[tokio::test]
async fn broadcast_refused_once_the_purchase_draft_outlived_its_reservation() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, _info) = signed_reverse_auction_purchase(&mut node).await;
    let send = mock_send(&mut node, 0).await;
    with_db(&app, |c| {
        c.execute(
            "UPDATE wallet_tx_drafts SET created_at = datetime('now', ?2) WHERE id = ?1",
            params![
                draft_id,
                format!(
                    "-{} seconds",
                    crate::noncustodial::send::RESERVATION_TTL_SECS + 60
                )
            ],
        )
        .unwrap();
    });

    let err = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains("prepared more than an hour ago"),
        "refused before sending"
    );
    send.assert_async().await;
    assert_unsent_purchase_discarded(&app, &draft_id);
}

#[tokio::test]
async fn broadcast_sends_while_the_paid_step_is_still_the_cheapest() {
    let mut node = mockito::Server::new_async().await;
    let (app, draft_id, info) = signed_reverse_auction_purchase(&mut node).await;
    info.remove_async().await;
    // Still before the 5 HNS step: its lock time MTP + 1024 rounds down to
    // MTP + 768, valid only once the median time is above that.
    let _later = mock_regtest_info(&mut node, Some(REGTEST_MTP + 768)).await;
    let send = mock_send(&mut node, 1).await;

    let res = crate::commands::tx::broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .expect("broadcast");
    assert_eq!(res.status, "broadcasted");
    send.assert_async().await;
    assert_eq!(draft_status(&app, &draft_id), "broadcasted");
}

/// The UI disables Buy and Finalize with the backend's own sentence (§Frontend:
/// "the backend error uses the same words"). The two copies live in two
/// languages, so this reads the UI's file and requires each backend sentence
/// there verbatim, as a string literal.
#[test]
fn refusal_sentences_match_the_uis_word_for_word() {
    const UI: &str = include_str!("../../../src/components/market/marketText.ts");
    // Prettier may break a long constant after `=`: compare without that.
    let ui = UI.replace("=\n  \"", "= \"");
    for (name, sentence) in [
        (
            "RECOVERY_PHRASE_ONLY",
            crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY,
        ),
        (
            "NEEDS_SENDING_NODE",
            crate::commands::shakedex::NEEDS_SENDING_NODE,
        ),
        (
            "MAINNET_EXPERIMENTAL",
            crate::noncustodial::shakedex::MAINNET_EXPERIMENTAL,
        ),
        (
            "MARKET_MAINNET_ONLY",
            crate::commands::shakedex::MARKET_MAINNET_ONLY,
        ),
    ] {
        let decl = format!("export const {name} = {sentence:?};");
        assert!(ui.contains(&decl), "marketText.ts lacks `{decl}`");
    }
}
