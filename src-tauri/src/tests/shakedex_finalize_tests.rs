//! Command-level tests for finalizing a Shakedex purchase out of the lock
//! (R14): `shakedex_build_purchase_finalize_draft` over a migrated in-memory
//! DB, with `mockito` standing in for the profile's node. The shared fixtures
//! (funded profile, mock app, node mocks, signed regtest listing) come from
//! `shakedex_cmd_tests`.

use mockito::{Matcher, Mock, ServerGuard};
use rusqlite::params;
use serde_json::{json, Value};
use tauri::Manager;

use super::shakedex_cmd_tests::{
    account_xpub, app_with, err_text, fixture_name, mock_blockchain_info, mock_coin,
    mock_name_info, regtest_listing, rpc_ok, seeded, set, with_db, FUND_TXID, LISTING_FILE,
    PROFILE,
};
use crate::commands::shakedex::shakedex_build_purchase_finalize_draft;
use crate::db;
use crate::db::queries::ShakedexPurchase;
use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::listing_file::ListingFile;
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::sync::{COV_FINALIZE, COV_TRANSFER};

const PURCHASE_ID: &str = "purchase1";
const PURCHASE_TXID: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
/// The height the purchase (and its TRANSFER) was mined at.
const TRANSFER_HEIGHT: i64 = 200;
const TRANSFER_VALUE: u64 = 1_000_000;
const RENEWAL_BLOCK: &str = "abababababababababababababababababababababababababababababababab";

/// Our reserved name destination: receive address 1 of the test account.
fn destination(net: Network) -> String {
    derivation::derive_one(net, &account_xpub(net), derivation::BRANCH_RECEIVE, 1)
        .unwrap()
        .address
}

fn lockup(net: Network) -> i64 {
    i64::from(net.name_params().transfer_lockup)
}

/// The TRANSFER coin our purchase created at `PURCHASE_TXID:0`, sitting at the
/// lock address and committing to `committed_to`, at the name height of the
/// `getnameinfo` result `name_info` (as hsd links them).
fn transfer_coin(net: Network, listing_json: &str, committed_to: &str, name_info: &Value) -> Value {
    let name_height = u32::try_from(name_info["info"]["height"].as_u64().expect("name height"))
        .expect("u32 height");
    let l = ListingFile::parse(listing_json, net).unwrap();
    let (version, hash) = address::decode(net, committed_to).unwrap();
    json!({
        "hash": PURCHASE_TXID,
        "index": 0,
        "value": TRANSFER_VALUE,
        "address": lock_address(net, &l.public_key).unwrap(),
        "height": TRANSFER_HEIGHT,
        "coinbase": false,
        "version": 0,
        "covenant": {
            "type": COV_TRANSFER,
            "action": "TRANSFER",
            "items": [
                hex::encode(crate::noncustodial::names::hash_name(&l.name).unwrap()),
                hex::encode(name_height.to_le_bytes()),
                hex::encode([version]),
                hex::encode(hash),
            ]
        }
    })
}

/// Record an `awaiting_finalize` purchase of `listing_json` for the profile.
fn insert_purchase(conn: &rusqlite::Connection, net: Network, listing_json: &str, state: &str) {
    let l = ListingFile::parse(listing_json, net).unwrap();
    db::queries::insert_shakedex_purchase(
        conn,
        &ShakedexPurchase {
            id: PURCHASE_ID.into(),
            wallet_profile_id: PROFILE.into(),
            name: l.name.clone(),
            listing_json: listing_json.into(),
            lock_txid: hex::encode(l.lock_txid),
            lock_vout: i64::from(l.lock_vout),
            price_doos: l.steps[0].price as i64,
            purchase_draft_id: "purchase-draft".into(),
            purchase_txid: PURCHASE_TXID.into(),
            destination_address: destination(net),
            state: state.parse().unwrap(),
            purchase_height: Some(TRANSFER_HEIGHT),
            blocks_remaining: Some(0),
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: None,
            finalize_draft_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .unwrap();
}

async fn mock_block_hash(s: &mut ServerGuard) -> Mock {
    s.mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({ "method": "getblockhash" })))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!(RENEWAL_BLOCK)))
        .create_async()
        .await
}

/// A node at `tip` that reports `transfer` at `PURCHASE_TXID:0`.
async fn node_with(
    s: &mut ServerGuard,
    tip: i64,
    name_info: Value,
    transfer: Option<Value>,
) -> Vec<Mock> {
    let mut mocks = vec![
        mock_blockchain_info(s, tip, None).await,
        mock_name_info(s, name_info).await,
        mock_block_hash(s).await,
    ];
    match transfer {
        Some(coin) => mocks.push(mock_coin(s, PURCHASE_TXID, coin).await),
        None => mocks.push(
            s.mock("GET", format!("/coin/{PURCHASE_TXID}/0").as_str())
                .with_status(404)
                .create_async()
                .await,
        ),
    }
    mocks
}

/// A regtest app with an `awaiting_finalize` purchase in `state`, and a node
/// at `tip` whose TRANSFER commits to `committed_to` (our destination when
/// `None`).
async fn regtest_app(
    node: &mut ServerGuard,
    tip: i64,
    state: &str,
    committed_to: Option<String>,
) -> (tauri::App<tauri::test::MockRuntime>, Vec<Mock>) {
    let net = Network::Regtest;
    let r = regtest_listing();
    let to = committed_to.unwrap_or_else(|| destination(net));
    let mocks = node_with(
        node,
        tip,
        r.name_info.clone(),
        Some(transfer_coin(net, &r.json, &to, &r.name_info)),
    )
    .await;
    let conn = seeded("regtest", "mnemonic_hot", &node.url());
    insert_purchase(&conn, net, &r.json, state);
    (app_with(conn), mocks)
}

fn purchase(app: &tauri::App<tauri::test::MockRuntime>) -> ShakedexPurchase {
    with_db(app, |c| {
        db::queries::get_shakedex_purchase(c, PURCHASE_ID)
            .unwrap()
            .unwrap()
    })
}

fn draft_count(app: &tauri::App<tauri::test::MockRuntime>) -> i64 {
    with_db(app, |c| {
        c.query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
            .unwrap()
    })
}

/// The first tip at which a FINALIZE is accepted: hsd judges a mempool
/// transaction at `tip + 1`, so the block where the lockup ends is the next one.
fn ready_tip(net: Network) -> i64 {
    TRANSFER_HEIGHT + lockup(net) - 1
}

// --- refusals -----------------------------------------------------------------

#[tokio::test]
async fn finalize_refused_before_lockup() {
    let mut node = mockito::Server::new_async().await;
    // One block short of the lockup: the stored row says 0, the chain says 1.
    let tip = ready_tip(Network::Regtest) - 1;
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", None).await;
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("1 block"));
    assert_eq!(draft_count(&app), 0);
    assert!(purchase(&app).finalize_draft_id.is_none());
}

#[tokio::test]
async fn finalize_built_one_block_before_the_lockup_height() {
    let mut node = mockito::Server::new_async().await;
    // The FINALIZE lands in block `TRANSFER_HEIGHT + lockup`, the first one
    // hsd accepts it in.
    let tip = TRANSFER_HEIGHT + lockup(Network::Regtest) - 1;
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", None).await;
    let draft = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap();
    assert_eq!(draft_count(&app), 1);
    // The stored summary reads back as a typed one in the secure window.
    let row = {
        let state = app.state::<crate::AppState>();
        let conn = state.db.lock().unwrap();
        db::queries::get_tx_draft(&conn, &draft.id)
            .unwrap()
            .unwrap()
    };
    crate::commands::tx::confirm_details_for_draft(&row).unwrap();
}

#[tokio::test]
async fn finalize_refused_unless_awaiting_finalize() {
    let mut node = mockito::Server::new_async().await;
    let tip = ready_tip(Network::Regtest);
    let (app, _m) = regtest_app(&mut node, tip, "unconfirmed", None).await;
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("not awaiting finalize"));
    assert_eq!(draft_count(&app), 0);
}

#[tokio::test]
async fn finalize_refuses_foreign_transfer() {
    let mut node = mockito::Server::new_async().await;
    let tip = ready_tip(Network::Regtest);
    let elsewhere = address::encode_p2wpkh(Network::Regtest, &[0x42; 20]).unwrap();
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", Some(elsewhere)).await;
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("does not commit to your address"));
    assert_eq!(draft_count(&app), 0);
    assert!(purchase(&app).finalize_draft_id.is_none());
}

/// A node at a ready tip whose TRANSFER is `transfer`, with an
/// `awaiting_finalize` purchase recorded: the build's error text.
async fn finalize_error_with_transfer(transfer: Value) -> (String, i64) {
    let mut node = mockito::Server::new_async().await;
    let net = Network::Regtest;
    let r = regtest_listing();
    let _m = node_with(
        &mut node,
        ready_tip(net),
        r.name_info.clone(),
        Some(transfer),
    )
    .await;
    let conn = seeded("regtest", "mnemonic_hot", &node.url());
    insert_purchase(&conn, net, &r.json, "awaiting_finalize");
    let app = app_with(conn);
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    (err_text(err), draft_count(&app))
}

/// A TRANSFER committing to us but sitting outside the listing's lock: not
/// the coin our purchase created, so not ours to finalize.
#[tokio::test]
async fn finalize_refuses_a_transfer_outside_the_lock() {
    let net = Network::Regtest;
    let r = regtest_listing();
    let mut transfer = transfer_coin(net, &r.json, &destination(net), &r.name_info);
    transfer["address"] = address::encode_p2wpkh(net, &[0x42; 20]).unwrap().into();
    let (err, drafts) = finalize_error_with_transfer(transfer).await;
    assert!(err.contains("not at the listing's lock address"), "{err}");
    assert_eq!(drafts, 0);
}

/// hsd always sends a coin's address (`Coin.getJSON`); a reply without one is
/// not hsd's answer and is not read as "somewhere else".
#[tokio::test]
async fn finalize_refused_when_the_node_omits_the_transfer_address() {
    let net = Network::Regtest;
    let r = regtest_listing();
    let mut transfer = transfer_coin(net, &r.json, &destination(net), &r.name_info);
    transfer.as_object_mut().unwrap().remove("address");
    let (err, drafts) = finalize_error_with_transfer(transfer).await;
    assert!(err.contains("did not report the address"), "{err}");
    assert_eq!(drafts, 0);
}

/// hsd links a FINALIZE to its TRANSFER only at the same name height. The
/// name now registered at another height expired and was registered again
/// since the purchase: refused before anything is signed.
#[tokio::test]
async fn finalize_refused_when_the_name_was_registered_again() {
    let net = Network::Regtest;
    let r = regtest_listing();
    let mut earlier = r.name_info.clone();
    let height = earlier["info"]["height"].as_u64().unwrap();
    earlier["info"]["height"] = (height - 1).into();
    let transfer = transfer_coin(net, &r.json, &destination(net), &earlier);
    let (err, drafts) = finalize_error_with_transfer(transfer).await;
    assert!(err.contains("registered again since the purchase"), "{err}");
    assert_eq!(drafts, 0);
}

/// A transfer covenant without hsd's 4-byte height is not hsd's reply.
#[tokio::test]
async fn finalize_refused_when_the_transfer_height_is_unreadable() {
    let net = Network::Regtest;
    let r = regtest_listing();
    let mut transfer = transfer_coin(net, &r.json, &destination(net), &r.name_info);
    transfer["covenant"]["items"][1] = "3200".into();
    let (err, drafts) = finalize_error_with_transfer(transfer).await;
    assert!(err.contains("readable height in the transfer"), "{err}");
    assert_eq!(drafts, 0);
}

/// Two builds of one purchase at once, each reading the purchase before the
/// other links its draft: the second must not link its own over the first,
/// which would orphan that draft and the coins it reserved.
#[tokio::test]
async fn concurrent_finalize_builds_leave_one_draft() {
    let mut node = mockito::Server::new_async().await;
    let tip = ready_tip(Network::Regtest);
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", None).await;
    let (a, b) = tokio::join!(
        shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None),
        shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None),
    );
    let (built, refused) = match (a, b) {
        (Ok(d), Err(e)) | (Err(e), Ok(d)) => (d, e),
        (a, b) => panic!("one build must win: {a:?} / {b:?}"),
    };
    assert!(err_text(refused).contains("prepared meanwhile"));
    assert_eq!(draft_count(&app), 1);
    assert_eq!(
        purchase(&app).finalize_draft_id.as_deref(),
        Some(built.id.as_str())
    );
}

/// Another profile's purchase is not found from this one.
#[tokio::test]
async fn finalize_refuses_another_profiles_purchase() {
    let mut node = mockito::Server::new_async().await;
    let tip = ready_tip(Network::Regtest);
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", None).await;
    with_db(&app, |c| {
        c.execute_batch(&format!(
            "CREATE TEMP TABLE other AS SELECT * FROM wallet_profiles WHERE id = '{PROFILE}';
             UPDATE other SET id = 'other-profile';
             INSERT INTO wallet_profiles SELECT * FROM other;
             UPDATE shakedex_purchases SET wallet_profile_id = 'other-profile';"
        ))
        .unwrap();
    });
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
    assert_eq!(draft_count(&app), 0);
}

#[tokio::test]
async fn finalize_refused_when_transfer_already_spent() {
    let mut node = mockito::Server::new_async().await;
    let net = Network::Regtest;
    let r = regtest_listing();
    let _m = node_with(&mut node, ready_tip(net), r.name_info.clone(), None).await;
    let conn = seeded("regtest", "mnemonic_hot", &node.url());
    insert_purchase(&conn, net, &r.json, "awaiting_finalize");
    let app = app_with(conn);
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("no longer unspent"));
    assert_eq!(draft_count(&app), 0);
}

#[tokio::test]
async fn finalize_refused_for_ledger_profile() {
    let mut node = mockito::Server::new_async().await;
    let net = Network::Regtest;
    let r = regtest_listing();
    let _m = node_with(
        &mut node,
        ready_tip(net),
        r.name_info.clone(),
        Some(transfer_coin(net, &r.json, &destination(net), &r.name_info)),
    )
    .await;
    let conn = seeded("regtest", "ledger_hardware", &node.url());
    insert_purchase(&conn, net, &r.json, "awaiting_finalize");
    let app = app_with(conn);
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("recovery-phrase wallet"));
    assert_eq!(draft_count(&app), 0);
}

/// R6: finalizing is a send too. A profile whose node cannot send (here a
/// read-only explorer source) is refused when the draft is built, with the
/// sentence the UI shows on the disabled Finalize, and no draft is written.
#[tokio::test]
async fn finalize_refused_without_write_capability() {
    let mut node = mockito::Server::new_async().await;
    let (app, _m) = regtest_app(
        &mut node,
        ready_tip(Network::Regtest),
        "awaiting_finalize",
        None,
    )
    .await;
    with_db(&app, |c| set(c, "chain_source", "explorer"));
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(
        err_text(err).contains(crate::commands::shakedex::NEEDS_SENDING_NODE),
        "the sentence the UI shows on the disabled Finalize"
    );
    assert_eq!(draft_count(&app), 0);
    assert!(purchase(&app).finalize_draft_id.is_none());
}

#[tokio::test]
async fn finalize_refused_while_a_finalize_is_already_sent() {
    let mut node = mockito::Server::new_async().await;
    let tip = ready_tip(Network::Regtest);
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", None).await;
    let first = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("first finalize builds");
    with_db(&app, |c| {
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'broadcasted' WHERE id = ?1",
            params![first.id],
        )
        .unwrap();
    });
    let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains("already sent"));
    assert_eq!(
        purchase(&app).finalize_draft_id.as_deref(),
        Some(first.id.as_str())
    );
}

// --- what is built ------------------------------------------------------------

#[tokio::test]
async fn finalize_ignores_experimental_flag() {
    let mut node = mockito::Server::new_async().await;
    let net = Network::Main;
    let _m = node_with(
        &mut node,
        ready_tip(net),
        fixture_name(),
        Some(transfer_coin(
            net,
            LISTING_FILE,
            &destination(net),
            &fixture_name(),
        )),
    )
    .await;
    // Mainnet, and `shakedex_experimental` never set: purchases are refused,
    // but finalizing one already made is not (R15).
    let conn = seeded("mainnet", "mnemonic_hot", &node.url());
    insert_purchase(&conn, net, LISTING_FILE, "awaiting_finalize");
    let app = app_with(conn);
    let draft = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("finalize needs no experimental flag");
    assert_eq!(draft.action, "shakedex_purchase_finalize");

    // And with the flag explicitly off.
    with_db(&app, |c| set(c, "shakedex_experimental", "false"));
    let again = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("finalize still builds with the flag off");
    assert_eq!(again.action, "shakedex_purchase_finalize");
}

#[tokio::test]
async fn finalize_draft_reserves_only_own_inputs() {
    let mut node = mockito::Server::new_async().await;
    let net = Network::Regtest;
    let (app, _m) = regtest_app(&mut node, ready_tip(net), "awaiting_finalize", None).await;
    let draft = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("finalize builds");

    assert_eq!(draft.action, "shakedex_purchase_finalize");
    let r = regtest_listing();
    let l = ListingFile::parse(&r.json, net).unwrap();
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

        let row = db::queries::get_tx_draft(c, &draft.id).unwrap().unwrap();
        let plan: Value = serde_json::from_str(&row.signing_inputs_json).unwrap();
        let inputs = plan["inputs"].as_array().unwrap();
        // The TRANSFER out of the lock, spent with the lock script alone.
        assert_eq!(inputs[0]["txid"], PURCHASE_TXID);
        assert_eq!(inputs[0]["vout"], 0);
        assert_eq!(inputs[0]["value"], TRANSFER_VALUE);
        let lock_script = crate::noncustodial::shakedex::script::lock_script(&l.public_key);
        assert_eq!(
            inputs[0]["foreign_witness_hex"],
            json!([hex::encode(lock_script)])
        );
        assert!(inputs[1..]
            .iter()
            .all(|i| i["foreign_witness_hex"].is_null() && i["txid"] == FUND_TXID));

        // The FINALIZE pays our destination, carrying the name's value.
        let fin = &plan["outputs"][0];
        assert_eq!(fin["covenant_type"], COV_FINALIZE);
        assert_eq!(fin["address"], destination(net));
        assert_eq!(fin["value"], TRANSFER_VALUE);
        let items = fin["covenant_items_hex"].as_array().unwrap();
        assert_eq!(items[2], hex::encode(b"regname"));
        assert_eq!(items[6], RENEWAL_BLOCK);
    });

    assert_eq!(
        purchase(&app).finalize_draft_id.as_deref(),
        Some(draft.id.as_str())
    );
    let s = &draft.summary;
    assert_eq!(s["action"], "shakedex_purchase_finalize");
    assert_eq!(s["name"], "regname");
    assert_eq!(s["purchaseId"], PURCHASE_ID);
    assert_eq!(s["destinationAddress"], destination(net));
    let fee = s["feeDoos"].as_i64().unwrap();
    assert!(fee > 0);
    // Our own coins only: the TRANSFER's value is the name's, not money spent.
    assert_eq!(
        s["inputTotalDoos"].as_i64().unwrap(),
        fee + s["changeDoos"].as_i64().unwrap()
    );
}

#[tokio::test]
async fn rebuilding_an_unsent_finalize_replaces_it() {
    let mut node = mockito::Server::new_async().await;
    let tip = ready_tip(Network::Regtest);
    let (app, _m) = regtest_app(&mut node, tip, "awaiting_finalize", None).await;
    let first = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("first finalize builds");
    let second = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("an unsent finalize is replaced");
    assert_ne!(first.id, second.id);
    assert_eq!(draft_count(&app), 1);
    assert_eq!(
        purchase(&app).finalize_draft_id.as_deref(),
        Some(second.id.as_str())
    );
    // Signed but not sent is still unsent: replaced too.
    with_db(&app, |c| {
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'signed' WHERE id = ?1",
            params![second.id],
        )
        .unwrap();
    });
    let third = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
        .await
        .expect("a signed, unsent finalize is replaced");
    assert_ne!(second.id, third.id);
    assert_eq!(draft_count(&app), 1);
    assert_eq!(
        purchase(&app).finalize_draft_id.as_deref(),
        Some(third.id.as_str())
    );
}

// --- secure confirmation --------------------------------------------------------

#[test]
fn confirm_rows_for_finalize() {
    let summary = json!({
        "action": "shakedex_purchase_finalize",
        "name": "regname",
        "purchaseId": PURCHASE_ID,
        "sendTotalDoos": 0,
        "feeDoos": 1_500,
        "totalDoos": 1_500,
        "changeDoos": 0,
        "inputTotalDoos": 1_500,
        "numInputs": 2,
        "recipientAddress": "rs1qdest",
        "destinationAddress": "rs1qdest",
        "txid": "cd".repeat(32),
        "warnings": ["The name keeps the seller's DNS records until you change them."]
    });
    let draft = db::queries::TxDraftRow {
        id: "d1".into(),
        wallet_profile_id: PROFILE.into(),
        action: "shakedex_purchase_finalize".into(),
        unsigned_tx_hex: String::new(),
        signed_tx_hex: None,
        signing_inputs_json: "{}".into(),
        summary_json: summary.to_string(),
        status: "draft".into(),
        error_message: None,
        txid: None,
        confirmation_height: None,
        created_at: String::new(),
    };
    let details = crate::commands::tx::confirm_details_for_draft(&draft).unwrap();
    let rows = details["rows"].as_array().unwrap();
    let value = |label: &str| {
        rows.iter()
            .find(|r| r["label"] == label)
            .and_then(|r| r["value"].as_str())
            .map(str::to_owned)
    };
    assert_eq!(value("Finalize purchased name").as_deref(), Some("regname"));
    assert_eq!(value("Network fee").as_deref(), Some("0.001500 HNS"));
    assert!(value("Warning").unwrap().contains("DNS records"));
    // The generic rows would read "Name action: shakedex_purchase_finalize".
    assert!(value("Action").is_none());

    // Without its fee the window cannot say what is spent: refused.
    let mut broken = summary.clone();
    broken.as_object_mut().unwrap().remove("feeDoos");
    let err = crate::commands::tx::confirm_details_for_draft(&db::queries::TxDraftRow {
        summary_json: broken.to_string(),
        ..draft
    })
    .unwrap_err();
    assert!(
        matches!(&err, AppError::Other(m) if m.contains("corrupted draft")),
        "{err:?}"
    );
}

/// The FINALIZE covenant carries the name's height, renewals, claimed count
/// and weak flag from `getnameinfo`. A reply without one is refused before a
/// draft exists, instead of a guessed 0 the node would reject after the user
/// signed.
#[tokio::test]
async fn finalize_refused_when_the_node_omits_a_covenant_field() {
    for field in ["height", "renewals", "claimed", "weak"] {
        let mut node = mockito::Server::new_async().await;
        let net = Network::Regtest;
        let r = regtest_listing();
        let mut name_info = r.name_info.clone();
        name_info["info"].as_object_mut().unwrap().remove(field);
        let _m = node_with(
            &mut node,
            ready_tip(net),
            name_info,
            Some(transfer_coin(net, &r.json, &destination(net), &r.name_info)),
        )
        .await;
        let conn = seeded("regtest", "mnemonic_hot", &node.url());
        insert_purchase(&conn, net, &r.json, "awaiting_finalize");
        let app = app_with(conn);
        let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
            .await
            .unwrap_err();
        assert!(err_text(err).contains(field), "{field}");
        assert_eq!(draft_count(&app), 0, "{field}: no draft");
    }
}

/// hsd always sends a coin's height, -1 in the mempool (`Coin.getJSON`). A
/// TRANSFER at -1 is not mined yet; one whose reply leaves the height out is
/// not hsd's answer, and the refusal says so rather than "not mined".
#[tokio::test]
async fn finalize_tells_a_mempool_transfer_from_one_without_its_height() {
    for (height, says) in [
        (Some(json!(-1)), "not mined yet"),
        (None, "did not report the height"),
    ] {
        let mut node = mockito::Server::new_async().await;
        let net = Network::Regtest;
        let r = regtest_listing();
        let mut transfer = transfer_coin(net, &r.json, &destination(net), &r.name_info);
        match &height {
            Some(h) => transfer["height"] = h.clone(),
            None => {
                transfer.as_object_mut().unwrap().remove("height");
            }
        }
        let _m = node_with(
            &mut node,
            ready_tip(net),
            r.name_info.clone(),
            Some(transfer),
        )
        .await;
        let conn = seeded("regtest", "mnemonic_hot", &node.url());
        insert_purchase(&conn, net, &r.json, "awaiting_finalize");
        let app = app_with(conn);
        let err = shakedex_build_purchase_finalize_draft(app.state(), PURCHASE_ID.into(), None)
            .await
            .unwrap_err();
        assert!(err_text(err).contains(says), "{height:?}");
        assert_eq!(draft_count(&app), 0, "{height:?}: no draft");
    }
}
