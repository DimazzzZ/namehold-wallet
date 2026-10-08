//! Purchase state from the chain (R13): every sync derives each open
//! purchase's state from what the node knows about the purchase, the lock coin
//! and the TRANSFER it creates — not from the draft's status alone.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::db::{self, queries};
use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{BlockchainInfo, ChainSource, NodeCoin};
use crate::shakedex_jobs::{refresh_purchases_with_client, Rebroadcast};
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
const PROFILE: &str = "buyer1";
const PURCHASE_ID: &str = "purchase1";
const DRAFT_ID: &str = "draft1";
const NAME: &str = "nameholdtest";
const FUNDING_TXID: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const PURCHASE_TXID: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const LOCK_TXID: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const OTHER_TXID: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const SIGNED_HEX: &str = "deadbeef";
/// Mainnet transfer lockup (`Network::Main.name_params().transfer_lockup`).
const LOCKUP: i64 = 288;
/// A live mainnet listing (one step, its price below): what a purchase row
/// records, and what a rebroadcast re-checks the price against.
const LISTING_FILE: &str = include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");
const PRICE: i64 = 435_000_000;
/// A median time well past every step's lock time in [`LISTING_FILE`] and
/// [`dropped_price_listing`].
const LATE_MTP: u64 = 1_900_000_000;

/// [`LISTING_FILE`] with a second, cheaper step that is valid by
/// [`LATE_MTP`]. The parser does not check a step's signature against its
/// template, so the first step's stands in.
fn dropped_price_listing() -> String {
    let mut l: Value = serde_json::from_str(LISTING_FILE).unwrap();
    let mut step = l["data"][0].clone();
    step["price"] = json!(PRICE / 2);
    step["lockTime"] = json!(step["lockTime"].as_u64().unwrap() + 86_400);
    l["data"].as_array_mut().unwrap().push(step);
    l.to_string()
}

/// The lock time of [`LISTING_FILE`]'s one step, the step a purchase pays.
const PAID_LOCK_TIME: u64 = 1_783_696_480;

/// [`LISTING_FILE`] with a dearer step a day before the paid one: between
/// the two lock times only the dearer step is valid.
fn dearer_step_first_listing() -> String {
    let mut l: Value = serde_json::from_str(LISTING_FILE).unwrap();
    assert_eq!(l["data"][0]["lockTime"].as_u64(), Some(PAID_LOCK_TIME));
    let mut step = l["data"][0].clone();
    step["price"] = json!(PRICE * 2);
    step["lockTime"] = json!(PAID_LOCK_TIME - 86_400);
    l["data"].as_array_mut().unwrap().insert(0, step);
    l.to_string()
}

fn seed() -> [u8; 64] {
    hd::seed_from_mnemonic(MNEMONIC, "").unwrap()
}

fn account_xpub() -> ExtendedPubKey {
    let path = hd::bip44_path(Network::Main, 0, 0, 0);
    let master = ExtendedPrivKey::from_seed(&seed()).unwrap();
    let account = master.derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account)
}

/// Receive address at leaf 0/`index`, with its script and pubkey hex.
fn leaf(index: u32) -> (String, String, String) {
    let (_sk, pk, addr) = hd::derive_address(Network::Main, &seed(), 0, 0, index).unwrap();
    let spk = hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

/// Our name destination (leaf 0/1).
fn destination() -> String {
    leaf(1).0
}

/// An address that is not ours to receive the name.
fn elsewhere() -> String {
    let (_sk, _pk, addr) = hd::derive_address(Network::Main, &seed(), 7, 0, 0).unwrap();
    addr
}

/// Migrated in-memory DB with a profile, one funding coin reserved by a
/// purchase draft in `draft_status`, and the purchase row in `state`.
fn seeded(draft_status: &str, state: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    seed_db(&conn, draft_status, state);
    conn
}

/// The plan a purchase draft stores, as `build_purchase` writes it: its lock
/// time is that of the step it pays (`PRICE`), which the rebroadcast re-checks.
fn paid_plan_json() -> String {
    let steps = crate::noncustodial::shakedex::listing_file::ListingFile::parse(
        LISTING_FILE,
        Network::Main,
    )
    .unwrap()
    .encoded_steps()
    .unwrap();
    let (_, locktime) = steps
        .into_iter()
        .find(|(price, _)| *price == PRICE as u64)
        .expect("a step at PRICE");
    serde_json::json!({
        "version": 0,
        "locktime": locktime,
        "account": 0,
        "network": "main",
        "inputs": [],
        "outputs": [],
    })
    .to_string()
}

/// [`seeded`] into an existing connection (a file-backed DB a sync opens by
/// path).
fn seed_db(conn: &Connection, draft_status: &str, state: &str) {
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(conn).unwrap();

    queries::insert_wallet_profile(
        conn,
        PROFILE,
        "Buyer",
        "mnemonic_hot",
        "mainnet",
        &account_xpub().to_base58check(Network::Main),
        0,
        false,
    )
    .unwrap();
    for i in 0..2 {
        let (addr, spk, pubkey) = leaf(i);
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, ?2, ?3, ?4, ?5)",
            params![PROFILE, i, addr, spk, pubkey],
        )
        .unwrap();
    }
    let (addr, spk, _) = leaf(0);
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, 5000000, 0, 'liquid_hns', NULL)",
        params![FUNDING_TXID, PROFILE, addr, spk],
    )
    .unwrap();
    queries::insert_tx_draft_reserving_coins(
        conn,
        DRAFT_ID,
        PROFILE,
        "shakedex_purchase",
        "00",
        &paid_plan_json(),
        "{}",
        &[(FUNDING_TXID.to_string(), 0)],
    )
    .unwrap();
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = ?2, signed_tx_hex = ?3 WHERE id = ?1",
        params![DRAFT_ID, draft_status, SIGNED_HEX],
    )
    .unwrap();
    queries::insert_shakedex_purchase(
        conn,
        &queries::ShakedexPurchase {
            id: PURCHASE_ID.to_string(),
            wallet_profile_id: PROFILE.to_string(),
            name: NAME.to_string(),
            listing_json: LISTING_FILE.to_string(),
            lock_txid: LOCK_TXID.to_string(),
            lock_vout: 0,
            price_doos: PRICE,
            purchase_draft_id: DRAFT_ID.to_string(),
            purchase_txid: PURCHASE_TXID.to_string(),
            destination_address: destination(),
            state: state.parse().unwrap(),
            purchase_height: None,
            blocks_remaining: None,
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

fn row(conn: &Connection) -> queries::ShakedexPurchase {
    queries::get_shakedex_purchase(conn, PURCHASE_ID)
        .unwrap()
        .expect("purchase row exists")
}

fn reservation(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = ?1",
        params![FUNDING_TXID],
        |r| r.get(0),
    )
    .unwrap()
}

/// The purchase draft's status and message, as Activity shows them.
fn draft_status(conn: &Connection) -> (String, Option<String>) {
    let d = queries::get_tx_draft(conn, DRAFT_ID).unwrap().unwrap();
    (d.status, d.error_message)
}

fn tip(blocks: i64) -> BlockchainInfo {
    BlockchainInfo {
        blocks,
        mediantime: Some(LATE_MTP),
        ..Default::default()
    }
}

fn coin(txid: &str, vout: u32, addr: &str, covenant: Value) -> NodeCoin {
    serde_json::from_value(coin_json(txid, vout, addr, covenant)).unwrap()
}

/// [`coin`] as hsd's `GET /coin` sends it: a mempool coin (hsd's height -1)
/// unless the test sets a height.
fn coin_json(txid: &str, vout: u32, addr: &str, covenant: Value) -> Value {
    json!({
        "hash": txid,
        "index": vout,
        "height": -1,
        "value": 0,
        "address": addr,
        "covenant": covenant,
    })
}

/// The coin the seller's lock still holds (unspent).
fn lock_coin() -> NodeCoin {
    coin(LOCK_TXID, 0, "hs1qlock", json!({"type": 10, "items": []}))
}

/// A TRANSFER covenant committing to `addr` (items 2–3: version, hash).
fn transfer_to(addr: &str) -> Value {
    let (version, hash) = address::decode(Network::Main, addr).unwrap();
    json!({
        "type": 9,
        "action": "TRANSFER",
        "items": ["00".repeat(32), "01000000", hex::encode([version]), hex::encode(hash)],
    })
}

/// `getnameinfo` naming an owner coin that is not ours.
fn foreign_owner() -> Value {
    json!({"info": {"name": NAME, "owner": {"hash": OTHER_TXID, "index": 0}}})
}

/// Coin lookups for a purchase nothing on chain traces to: the TRANSFER coin
/// does not exist and the name's owner coin pays someone else. `lock` is the
/// seller's lock coin as the node reports it.
fn untraced(txid: &str, vout: u32, lock: Option<NodeCoin>) -> Result<Option<NodeCoin>, AppError> {
    match (txid, vout) {
        (LOCK_TXID, 0) => Ok(lock),
        (PURCHASE_TXID, 0) => Ok(None),
        (OTHER_TXID, 0) => Ok(Some(coin(
            OTHER_TXID,
            0,
            &elsewhere(),
            json!({"type": 10, "items": []}),
        ))),
        other => panic!("unexpected coin lookup {other:?}"),
    }
}

/// A node that has never seen the purchase while the lock coin is unspent.
fn missing_at(height: i64) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_blockchain_info(tip(height))
        .with_tx_by_hash(Value::Null)
        .with_name_info(foreign_owner())
        .with_get_coin(|txid, vout| untraced(txid, vout, Some(lock_coin())))
}

fn sends(rpc: &MockNodeRpc) -> usize {
    rpc.count_matching(|c| matches!(c, RpcCall::SendRawTransaction(h) if h == SIGNED_HEX))
}

async fn run(conn: &Connection, rpc: &MockNodeRpc) {
    refresh_purchases_with_client(conn, rpc, PROFILE, Network::Main, Rebroadcast::Allowed)
        .await
        .unwrap();
}

/// The sync daemon never broadcasts (SECURITY.md): it tracks a purchase that
/// went missing but leaves its one rebroadcast to the app's next sync.
#[tokio::test]
async fn daemon_never_rebroadcasts_and_leaves_it_to_the_app() {
    let conn = seeded("broadcasted", "unconfirmed");
    let at = |height: i64| missing_at(height).with_send_raw_transaction(PURCHASE_TXID.to_string());

    for height in [1000, 1006, 1020] {
        let rpc = at(height);
        refresh_purchases_with_client(&conn, &rpc, PROFILE, Network::Main, Rebroadcast::Never)
            .await
            .unwrap();
        assert_eq!(sends(&rpc), 0, "the daemon sent at {height}");
        let p = row(&conn);
        assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
        assert_eq!(p.rebroadcast_count, 0);
        assert_eq!(p.missing_since_height, Some(1000));
    }

    let rpc = at(1021);
    run(&conn, &rpc).await;
    assert_eq!(sends(&rpc), 1, "the app rebroadcasts once");
    assert_eq!(row(&conn).rebroadcast_count, 1);
}

/// A purchase that went missing where it may not be resent (the daemon, or a
/// remote node without the opt-in) waits for a sync that may — but not for
/// ever: once hsd's mempool expiry (72 hours, 432 blocks) has passed, no node
/// still holds it, and it is lost while paying nothing, its coins released.
/// Lost is not final for it: a late mining still revives it for 7 days.
#[tokio::test]
async fn purchase_nobody_may_resend_is_lost_after_the_mempool_expiry() {
    for (height, lost) in [(1000 + 431, false), (1000 + 432, true)] {
        let conn = seeded("broadcasted", "unconfirmed");
        conn.execute(
            "UPDATE shakedex_purchases SET missing_since_height = 1000 WHERE id = ?1",
            params![PURCHASE_ID],
        )
        .unwrap();
        let rpc = missing_at(height).with_send_raw_transaction(PURCHASE_TXID.to_string());

        refresh_purchases_with_client(&conn, &rpc, PROFILE, Network::Main, Rebroadcast::Never)
            .await
            .unwrap();

        assert_eq!(sends(&rpc), 0, "at {height}");
        let p = row(&conn);
        if lost {
            assert_eq!(
                p.state,
                crate::db::queries::PurchaseState::Lost,
                "at {height}"
            );
            assert!(p.lost_reason.unwrap().contains("nothing was paid"));
            assert_eq!(reservation(&conn), None, "coins released at {height}");
        } else {
            assert_eq!(
                p.state,
                crate::db::queries::PurchaseState::Unconfirmed,
                "at {height}"
            );
            assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
        }
    }
}

/// A rebroadcast passes the same gates as any broadcast: a node that reports
/// another chain than the purchase's profile is never sent the purchase.
#[tokio::test]
async fn rebroadcast_never_goes_to_a_node_on_another_chain() {
    let conn = seeded("broadcasted", "unconfirmed");
    conn.execute(
        "UPDATE shakedex_purchases SET missing_since_height = 1000 WHERE id = ?1",
        params![PURCHASE_ID],
    )
    .unwrap();
    let rpc = missing_at(1021)
        .with_blockchain_info(BlockchainInfo {
            blocks: 1021,
            mediantime: Some(LATE_MTP),
            chain: Some("regtest".into()),
            ..Default::default()
        })
        .with_send_raw_transaction(PURCHASE_TXID.to_string());

    run(&conn, &rpc).await;

    assert_eq!(sends(&rpc), 0);
    assert_eq!(
        row(&conn).state,
        crate::db::queries::PurchaseState::Unconfirmed
    );
}

#[tokio::test]
async fn mined_purchase_becomes_awaiting_finalize_with_blocks_remaining() {
    let conn = seeded("broadcasted", "unconfirmed");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(owned_by_purchase())
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_get_coin(move |txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0), "reads the TRANSFER coin");
            Ok(Some(coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest))))
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.purchase_height, Some(1000));
    assert_eq!(p.blocks_remaining, Some(1000 + LOCKUP - (1010 + 1))); // judged at the next block
    assert_eq!(p.lost_reason, None);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

/// hsd always sends `height` (-1 in the mempool): a reply without it, or
/// with something else there, says neither mined nor in the mempool, so the
/// purchase is left as it is.
#[tokio::test]
async fn tx_reply_without_a_height_leaves_the_purchase_as_it_is() {
    for tx in [
        json!({"hash": PURCHASE_TXID, "height": null}),
        json!({"hash": PURCHASE_TXID}),
        json!({"hash": PURCHASE_TXID, "height": "1000"}),
        json!({"hash": PURCHASE_TXID, "height": -2}),
    ] {
        let conn = seeded("broadcasted", "pending_send");
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1010))
            .with_tx_by_hash(tx.clone());

        run(&conn, &rpc).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::PendingSend,
            "tx {tx}"
        );
        assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID), "tx {tx}");
    }
}

/// hsd's `GET /tx/:hash` gives a mempool transaction `"height": -1`
/// (`TXMeta.getJSON`).
#[tokio::test]
async fn mempool_purchase_is_unconfirmed() {
    for tx in [json!({"hash": PURCHASE_TXID, "height": -1})] {
        let conn = seeded("broadcasted", "pending_send");
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1010))
            .with_tx_by_hash(tx.clone());

        run(&conn, &rpc).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::Unconfirmed,
            "tx {tx}"
        );
        assert_eq!(p.purchase_height, None);
        assert_eq!(p.blocks_remaining, None);
        assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
        assert_eq!(
            rpc.count_matching(|c| matches!(c, RpcCall::GetCoin(..))),
            0,
            "a mempool purchase needs no coin lookup"
        );
    }
}

#[tokio::test]
async fn lost_to_another_buyer_releases_coins() {
    let conn = seeded("broadcasted", "unconfirmed");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_tx_by_hash(Value::Null)
        .with_name_info(foreign_owner())
        .with_get_coin(|txid, vout| untraced(txid, vout, None));

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    assert!(p.lost_reason.is_some_and(|r| r.contains("someone else")));
    assert_eq!(reservation(&conn), None, "funding coin released");
    assert_eq!(
        rpc.count_matching(|c| matches!(c, RpcCall::SendRawTransaction(_))),
        0
    );
    // R13: Activity says nothing was paid, through the purchase's draft.
    assert_eq!(
        draft_status(&conn),
        (
            "dropped".to_string(),
            Some("someone else bought the name first, or the seller cancelled the listing — nothing was paid".to_string())
        )
    );
}

#[tokio::test]
async fn rejected_at_broadcast_is_lost() {
    // Nothing on chain traces to the purchase.
    let conn = seeded("failed", "pending_send");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(foreign_owner())
        .with_get_coin(|txid, vout| untraced(txid, vout, None));

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    let refused = "the node refused to take the purchase, so nothing was paid";
    assert_eq!(p.lost_reason.as_deref(), Some(refused));
    assert_eq!(reservation(&conn), None, "funding coin released");
    assert_eq!(
        rpc.count_matching(|c| matches!(c, RpcCall::TxByHash(_))),
        0,
        "a refused purchase needs no chain lookup"
    );
    assert_eq!(
        draft_status(&conn),
        ("failed".to_string(), Some(refused.to_string())),
        "a refused draft stays failed and says why"
    );
}

#[tokio::test]
async fn never_mined_rebroadcasts_once_then_lost() {
    let conn = seeded("broadcasted", "unconfirmed");
    let at = |height: i64| missing_at(height).with_send_raw_transaction(PURCHASE_TXID.to_string());

    // First seen missing: the height is recorded, nothing else happens.
    let rpc = at(1000);
    run(&conn, &rpc).await;
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.missing_since_height, Some(1000));
    assert_eq!(p.rebroadcast_count, 0);
    assert_eq!(sends(&rpc), 0);

    // Five blocks later: still waiting.
    let rpc = at(1005);
    run(&conn, &rpc).await;
    assert_eq!(row(&conn).missing_since_height, Some(1000));
    assert_eq!(sends(&rpc), 0);

    // Six blocks missing: rebroadcast once and restart the count.
    let rpc = at(1006);
    run(&conn, &rpc).await;
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.rebroadcast_count, 1);
    assert_eq!(p.missing_since_height, None);
    assert_eq!(sends(&rpc), 1);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));

    // Missing again after the rebroadcast: recorded, then six blocks → lost.
    let rpc = at(1007);
    run(&conn, &rpc).await;
    assert_eq!(row(&conn).missing_since_height, Some(1007));
    let rpc = at(1013);
    run(&conn, &rpc).await;
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    assert!(p.lost_reason.is_some_and(|r| r.contains("never confirmed")));
    assert_eq!(p.rebroadcast_count, 1);
    assert_eq!(sends(&rpc), 0, "rebroadcast only once");
    assert_eq!(reservation(&conn), None, "funding coin released");
}

/// The rebroadcast is a broadcast like any other (R10): if a cheaper step
/// of the listing has become valid while the purchase was missing, the old
/// price is not sent again. Nothing was paid, so the purchase is lost unpaid
/// (and looked at again for a week, in case another node mines it).
#[tokio::test]
async fn rebroadcast_refused_when_a_cheaper_step_became_valid() {
    let conn = seeded("broadcasted", "unconfirmed");
    conn.execute(
        "UPDATE shakedex_purchases SET listing_json = ?1, missing_since_height = 1000",
        params![dropped_price_listing()],
    )
    .unwrap();
    let rpc = missing_at(1006).with_send_raw_transaction(PURCHASE_TXID.to_string());

    run(&conn, &rpc).await;

    assert_eq!(sends(&rpc), 0, "the dearer price is not sent again");
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    let reason = p.lost_reason.unwrap();
    assert!(reason.contains("price"), "{reason}");
    assert!(reason.contains("nothing was paid"), "{reason}");
    assert_eq!(reservation(&conn), None, "funding coin released");
    assert_eq!(draft_status(&conn).0, "dropped");
}

/// Without the node's median time the price cannot be re-checked: nothing is
/// sent, and nothing is decided — the next sync tries again.
#[tokio::test]
async fn rebroadcast_waits_when_the_price_cannot_be_rechecked() {
    let conn = seeded("broadcasted", "unconfirmed");
    conn.execute(
        "UPDATE shakedex_purchases SET missing_since_height = 1000",
        [],
    )
    .unwrap();
    let rpc = missing_at(1006)
        .with_blockchain_info(BlockchainInfo {
            blocks: 1006,
            ..Default::default()
        })
        .with_send_raw_transaction(PURCHASE_TXID.to_string());

    run(&conn, &rpc).await;

    assert_eq!(sends(&rpc), 0);
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.lost_reason, None);
    assert_eq!(p.rebroadcast_count, 0);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

/// A median time gone back below the paid step's lock time (a reorg) leaves
/// the paid step not valid — no step at all, or only a dearer, earlier one:
/// hsd would take the purchase as non-final and still answer with its txid,
/// so a resend would spend the one rebroadcast on nothing. Nothing is sent
/// and nothing decided while the step may become valid again — until hsd's
/// mempool expiry, as for a purchase nobody may resend.
#[tokio::test]
async fn rebroadcast_waits_while_the_paid_step_is_not_valid_yet() {
    let cases = [
        // No step valid at all.
        (LISTING_FILE.to_string(), 1),
        // Only a dearer step, a day earlier, is valid.
        (dearer_step_first_listing(), PAID_LOCK_TIME - 1_000),
    ];
    for (listing, mtp) in cases {
        for (height, lost) in [(1000 + 431, false), (1000 + 432, true)] {
            let conn = seeded("broadcasted", "unconfirmed");
            conn.execute(
                "UPDATE shakedex_purchases SET listing_json = ?1, missing_since_height = 1000",
                params![listing],
            )
            .unwrap();
            let rpc = missing_at(height)
                .with_blockchain_info(BlockchainInfo {
                    blocks: height,
                    mediantime: Some(mtp),
                    ..Default::default()
                })
                .with_send_raw_transaction(PURCHASE_TXID.to_string());

            run(&conn, &rpc).await;

            assert_eq!(sends(&rpc), 0, "at {height}");
            let p = row(&conn);
            assert_eq!(p.rebroadcast_count, 0, "at {height}");
            if lost {
                assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
                assert!(p.lost_reason.unwrap().contains("never confirmed"));
                assert_eq!(reservation(&conn), None, "coins released");
            } else {
                assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
                assert_eq!(p.lost_reason, None);
                assert_eq!(p.missing_since_height, Some(1000));
                assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
            }
        }
    }
}

#[tokio::test]
async fn reorg_moves_awaiting_finalize_back_to_unconfirmed() {
    let conn = seeded("confirmed", "unconfirmed");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(owned_by_purchase())
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_get_coin(move |_, _| {
            Ok(Some(coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest))))
        });
    run(&conn, &rpc).await;
    assert_eq!(
        row(&conn).state,
        crate::db::queries::PurchaseState::AwaitingFinalize
    );

    // The block holding the purchase is reorganised out; it is back in the
    // mempool.
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1009))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": -1}));
    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.purchase_height, None);
    assert_eq!(p.blocks_remaining, None);
    assert_eq!(p.lost_reason, None);
}

#[tokio::test]
async fn transfer_committing_elsewhere_is_lost() {
    let conn = seeded("confirmed", "unconfirmed");
    let other = elsewhere();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_get_coin(move |_, _| {
            Ok(Some(coin(
                PURCHASE_TXID,
                0,
                "hs1qlock",
                transfer_to(&other),
            )))
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    assert_eq!(
        p.lost_reason.as_deref(),
        Some("the purchase transferred the name elsewhere")
    );
    assert_eq!(reservation(&conn), None, "funding coin released");
    assert_eq!(
        draft_status(&conn).0,
        "confirmed",
        "a purchase that was mined did pay: its draft is not marked dropped"
    );
}

#[tokio::test]
async fn arrives_at_destination_is_owned() {
    let conn = seeded("confirmed", "awaiting_finalize");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1400))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_name_info(json!({"info": {"name": NAME, "owner": {"hash": OTHER_TXID, "index": 0}}}))
        .with_get_coin(move |txid, vout| match (txid, vout) {
            // The TRANSFER coin was spent by the FINALIZE.
            (PURCHASE_TXID, 0) => Ok(None),
            (OTHER_TXID, 0) => Ok(Some(coin(
                OTHER_TXID,
                0,
                &dest,
                json!({"type": 10, "items": []}),
            ))),
            other => panic!("unexpected coin lookup {other:?}"),
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Owned);
    assert_eq!(p.blocks_remaining, Some(0));
    assert_eq!(p.lost_reason, None);
    assert_eq!(
        rpc.calls_matching(|c| matches!(c, RpcCall::NameInfo(_))),
        vec![RpcCall::NameInfo(NAME.to_string())]
    );
}

/// A broadcast refuses a purchase draft past the reservation TTL, but one
/// that started just before it is still talking to the node. The sync leaves
/// such a draft and its purchase alone until the send grace is over too, so
/// it never deletes a purchase that is being sent.
#[tokio::test]
async fn signed_draft_within_the_send_grace_is_kept() {
    let conn = seeded("signed", "pending_send");
    conn.execute(
        "UPDATE wallet_tx_drafts SET created_at = datetime('now', ?2) WHERE id = ?1",
        params![
            DRAFT_ID,
            format!(
                "-{} seconds",
                crate::noncustodial::send::RESERVATION_TTL_SECS + 60
            )
        ],
    )
    .unwrap();

    run(&conn, &MockNodeRpc::new().with_blockchain_info(tip(1010))).await;

    assert_eq!(
        row(&conn).state,
        crate::db::queries::PurchaseState::PendingSend
    );
    assert!(queries::get_tx_draft(&conn, DRAFT_ID).unwrap().is_some());
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

#[tokio::test]
async fn abandoned_unsent_draft_row_is_deleted() {
    // A signed draft past the reservation TTL and the send grace that was
    // never sent.
    let conn = seeded("signed", "pending_send");
    conn.execute(
        "UPDATE wallet_tx_drafts SET created_at = datetime('now', ?2) WHERE id = ?1",
        params![
            DRAFT_ID,
            format!(
                "-{} seconds",
                crate::noncustodial::send::RESERVATION_TTL_SECS
                    + crate::shakedex_jobs::SEND_GRACE_SECS
                    + 60
            )
        ],
    )
    .unwrap();
    // A fresh signed draft is kept: the user may still send it.
    let fresh = seeded("signed", "pending_send");
    // A purchase whose draft is gone entirely.
    let orphan = seeded("draft", "pending_send");
    orphan
        .execute("UPDATE tracked_utxos SET reserved_by_draft_id = NULL", [])
        .unwrap();
    orphan
        .execute(
            "DELETE FROM wallet_tx_drafts WHERE id = ?1",
            params![DRAFT_ID],
        )
        .unwrap();
    // An old draft that reached broadcast is never deleted, however old.
    let sent = seeded("broadcast_pending", "pending_send");
    sent.execute(
        "UPDATE wallet_tx_drafts SET created_at = datetime('now', '-30 days') WHERE id = ?1",
        params![DRAFT_ID],
    )
    .unwrap();
    let rpc = MockNodeRpc::new().with_blockchain_info(tip(1010));

    run(&conn, &rpc).await;
    run(&fresh, &rpc).await;
    run(&orphan, &rpc).await;
    let sent_rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": -1}));
    run(&sent, &sent_rpc).await;

    assert!(queries::get_shakedex_purchase(&conn, PURCHASE_ID)
        .unwrap()
        .is_none());
    assert!(
        queries::get_tx_draft(&conn, DRAFT_ID).unwrap().is_none(),
        "the stale signed draft goes with its row, so it cannot be sent untracked"
    );
    assert_eq!(reservation(&conn), None, "funding coin released");
    assert_eq!(
        row(&fresh).state,
        crate::db::queries::PurchaseState::PendingSend
    );
    assert!(queries::get_tx_draft(&fresh, DRAFT_ID).unwrap().is_some());
    assert_eq!(reservation(&fresh).as_deref(), Some(DRAFT_ID));
    assert!(queries::get_shakedex_purchase(&orphan, PURCHASE_ID)
        .unwrap()
        .is_none());
    assert_eq!(
        row(&sent).state,
        crate::db::queries::PurchaseState::Unconfirmed
    );
    assert_eq!(reservation(&sent).as_deref(), Some(DRAFT_ID));
    assert_eq!(
        rpc.count_matching(|c| matches!(c, RpcCall::TxByHash(_))),
        0,
        "unsent purchases need no chain lookup"
    );
}

#[tokio::test]
async fn unknown_tx_with_spent_lock_but_our_transfer_on_chain_is_awaiting_finalize() {
    // A node without a transaction index: the mined purchase reads as
    // unknown and the lock coin as spent — but the TRANSFER is ours.
    let conn = seeded("broadcasted", "unconfirmed");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(owned_by_purchase())
        .with_tx_by_hash(Value::Null)
        .with_get_coin(move |txid, vout| match (txid, vout) {
            (LOCK_TXID, 0) => Ok(None),
            (PURCHASE_TXID, 0) => {
                let mut c = coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest));
                c.height = Some(1000);
                Ok(Some(c))
            }
            other => panic!("unexpected coin lookup {other:?}"),
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.purchase_height, Some(1000));
    assert_eq!(p.blocks_remaining, Some(1000 + LOCKUP - (1010 + 1))); // judged at the next block
    assert_eq!(p.lost_reason, None);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

#[tokio::test]
async fn failed_draft_with_our_transfer_on_chain_is_awaiting_finalize() {
    let conn = seeded("failed", "pending_send");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(owned_by_purchase())
        .with_get_coin(move |txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0));
            let mut c = coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest));
            c.height = Some(1005);
            Ok(Some(c))
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.purchase_height, Some(1005));
    assert_eq!(p.blocks_remaining, Some(1005 + LOCKUP - (1010 + 1))); // judged at the next block
    assert_eq!(p.lost_reason, None);
}

#[tokio::test]
async fn transfer_only_in_the_mempool_is_unconfirmed() {
    // A node without a transaction index sees the TRANSFER coin while the
    // purchase is still in the mempool (hsd's height -1): not mined, so no
    // lockup has begun.
    let conn = seeded("failed", "pending_send");
    let rpc = transfer_coin_at(Some(-1));

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.purchase_height, None);
    assert_eq!(p.blocks_remaining, None);
    assert_eq!(p.lost_reason, None);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

/// hsd always sends a coin's height, -1 in the mempool (`Coin.getJSON`). A
/// TRANSFER reply without it, or with a height hsd never sends, is not
/// "in the mempool": the purchase keeps its state and its coins.
#[tokio::test]
async fn transfer_reply_without_its_height_moves_nothing() {
    for height in [None, Some(-2)] {
        let conn = seeded("failed", "pending_send");
        let rpc = transfer_coin_at(height);

        run(&conn, &rpc).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::PendingSend,
            "height {height:?}"
        );
        assert_eq!(p.lost_reason, None, "height {height:?}");
        assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
    }
}

/// A node that holds the purchase's TRANSFER coin, committed to us, at
/// `height` as its reply sends it.
fn transfer_coin_at(height: Option<i64>) -> MockNodeRpc {
    let dest = destination();
    MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_get_coin(move |txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0));
            let mut c = coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest));
            c.height = height;
            Ok(Some(c))
        })
}

#[tokio::test]
async fn failed_draft_whose_transfer_still_owns_the_name_is_not_lost() {
    // The TRANSFER coin is gone from the coin view (spent by a FINALIZE in
    // the mempool) but the name's owner is still our purchase output.
    let conn = seeded("failed", "pending_send");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(
            json!({"info": {"name": NAME, "owner": {"hash": PURCHASE_TXID, "index": 0}}}),
        )
        .with_get_coin(|txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0));
            Ok(None)
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.lost_reason, None);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

#[tokio::test]
async fn rebroadcast_skipped_on_remote_node_without_opt_in() {
    let conn = seeded("broadcasted", "unconfirmed");
    let remote = |height: i64| {
        missing_at(height)
            .with_source(ChainSource::RemoteNode)
            .with_send_raw_transaction(PURCHASE_TXID.to_string())
    };
    run(&conn, &remote(1000)).await;

    let rpc = remote(1006);
    run(&conn, &rpc).await;
    let p = row(&conn);
    assert_eq!(sends(&rpc), 0, "no opt-in, no send");
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.rebroadcast_count, 0);
    assert_eq!(p.missing_since_height, Some(1000));
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));

    queries::set_setting(&conn, "allow_remote_broadcast", "true").unwrap();
    let rpc = remote(1007);
    run(&conn, &rpc).await;
    assert_eq!(sends(&rpc), 1, "sent once the user opts in");
    assert_eq!(row(&conn).rebroadcast_count, 1);
}

#[tokio::test]
async fn rebroadcast_reply_not_from_hsd_is_retried() {
    for err in [
        "node returned non-JSON body (status 502 Bad Gateway): error decoding response body",
        "malformed RPC envelope: invalid type; body=\"bad gateway\"",
    ] {
        let conn = seeded("broadcasted", "unconfirmed");
        run(&conn, &missing_at(1000)).await;

        let rpc = missing_at(1006).with_send_raw_transaction_rpc_err(err);
        run(&conn, &rpc).await;
        let p = row(&conn);
        assert_eq!(sends(&rpc), 1);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::Unconfirmed,
            "{err}"
        );
        assert_eq!(p.rebroadcast_count, 0, "retried next sync");
        assert_eq!(p.lost_reason, None);
        assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));

        let rpc = missing_at(1007).with_send_raw_transaction_transport_err("connection reset");
        run(&conn, &rpc).await;
        assert_eq!(sends(&rpc), 1, "tried again");
        assert_eq!(
            row(&conn).state,
            crate::db::queries::PurchaseState::Unconfirmed
        );
    }
}

#[tokio::test]
async fn rebroadcast_refused_by_hsd_is_lost() {
    let conn = seeded("broadcasted", "unconfirmed");
    run(&conn, &missing_at(1000)).await;

    let rpc =
        // hsd 8.0.0's own error to `sendrawtransaction` (`rpc.js`, TYPE_ERROR);
        // a mempool refusal is no error at all.
        missing_at(1006).with_send_raw_transaction_refused("Invalid hex string.", -3);
    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(sends(&rpc), 1);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    assert_eq!(
        p.lost_reason.as_deref(),
        Some("the node refused to take the purchase, so nothing was paid")
    );
    assert_eq!(p.rebroadcast_count, 1);
    assert_eq!(reservation(&conn), None, "funding coin released");
}

#[tokio::test]
async fn dropped_purchase_is_never_rebroadcast() {
    let conn = seeded("dropped", "unconfirmed");
    let at = |height: i64| missing_at(height).with_send_raw_transaction(PURCHASE_TXID.to_string());
    run(&conn, &at(1000)).await;
    assert_eq!(
        row(&conn).state,
        crate::db::queries::PurchaseState::Unconfirmed
    );

    let rpc = at(1006);
    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(
        sends(&rpc),
        0,
        "a dropped draft's coins may be spent elsewhere"
    );
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    assert!(p.lost_reason.is_some_and(|r| r.contains("never confirmed")));
}

/// `getnameinfo` while the purchase's TRANSFER is still the name's owner.
fn owned_by_purchase() -> Value {
    json!({"info": {"name": NAME, "owner": {"hash": PURCHASE_TXID, "index": 0}}})
}

/// A node where the purchase is mined at 1000 and its TRANSFER, committing to
/// our destination, is still unspent; `name_info` is the name's state.
fn mined_and_committed(name_info: Value) -> MockNodeRpc {
    let dest = destination();
    MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_name_info(name_info)
        .with_get_coin(move |txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0));
            let mut c = coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest));
            c.height = Some(1000);
            Ok(Some(c))
        })
}

/// A purchase never finalized whose name expired (hsd's `getnameinfo` then
/// has no `info`), or was opened and won again after expiring (the owner is
/// no longer the purchase's TRANSFER), can never be finalized: it is lost,
/// not "Ready to finalize" for ever. It was mined, so it did pay: its draft
/// is not marked dropped.
#[tokio::test]
async fn mined_purchase_whose_name_expired_before_finalize_is_lost() {
    for name_info in [json!({"info": null}), foreign_owner()] {
        let conn = seeded("broadcasted", "awaiting_finalize");
        run(&conn, &mined_and_committed(name_info.clone())).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::Lost,
            "{name_info}"
        );
        let reason = p.lost_reason.unwrap();
        assert!(
            reason.contains("expired before it was finalized"),
            "{reason}"
        );
        assert!(!reason.contains("nothing was paid"), "{reason}");
        assert_eq!(draft_status(&conn).0, "broadcasted", "{name_info}");
    }
}

/// Our FINALIZE can land between the read of the TRANSFER coin (still
/// unspent) and `getnameinfo` (owner: the FINALIZE). The name did not move
/// on without the purchase: it arrived. `lost` is final and releases nothing
/// back, so the purchase is never declared lost on two reads the chain moved
/// between; it keeps its state and the next sync traces it to owned.
#[tokio::test]
async fn finalize_landing_between_two_reads_never_loses_a_purchase() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    const FINALIZE_TXID: &str = OTHER_TXID;
    // Through the chain refresh of a sent purchase, and through the trace of
    // one whose draft failed.
    for (draft, state) in [
        ("broadcasted", "unconfirmed"),
        ("broadcasted", "awaiting_finalize"),
        ("failed", "pending_send"),
    ] {
        let conn = seeded(draft, state);
        let dest = destination();
        let transfer_reads = std::sync::Arc::new(AtomicUsize::new(0));
        let reads = transfer_reads.clone();
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1300))
            .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
            .with_name_info(
                json!({"info": {"name": NAME, "owner": {"hash": FINALIZE_TXID, "index": 0}}}),
            )
            .with_get_coin(move |txid, vout| match (txid, vout) {
                // Unspent at the first read; the FINALIZE spends it right after.
                (PURCHASE_TXID, 0) => Ok((reads.fetch_add(1, Ordering::SeqCst) == 0).then(|| {
                    let mut c = coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest));
                    c.height = Some(1000);
                    c
                })),
                (FINALIZE_TXID, 0) => Ok(Some(coin(
                    FINALIZE_TXID,
                    0,
                    &destination(),
                    json!({"type": 10, "items": []}),
                ))),
                other => panic!("unexpected coin lookup {other:?}"),
            });

        let _ = refresh_purchases_with_client(
            &conn,
            &rpc,
            PROFILE,
            Network::Main,
            Rebroadcast::Allowed,
        )
        .await;

        let p = row(&conn);
        assert_ne!(p.state, crate::db::queries::PurchaseState::Lost, "{state}");
        assert_eq!(p.lost_reason, None, "{state}");
        assert!(transfer_reads.load(Ordering::SeqCst) >= 1, "{state}");
    }
}

// --- a mined TRANSFER spent on chain, the name not at our destination -------

/// A mined purchase whose TRANSFER coin is spent on chain while the name's
/// owner coin pays someone else; `history` answers `GET /tx/address` for our
/// destination. Only our FINALIZE can spend the TRANSFER: the lock script
/// allows a TRANSFER signed by the seller or a FINALIZE, and hsd never lets a
/// TRANSFER be spent into another TRANSFER.
fn transfer_spent_and_name_elsewhere(history: Vec<Value>) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_blockchain_info(tip(1300))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_name_info(foreign_owner())
        .with_txs_by_address(history)
        .with_get_coin(|txid, vout| match (txid, vout) {
            (PURCHASE_TXID, 0) => Ok(None),
            (OTHER_TXID, 0) => Ok(Some(coin(
                OTHER_TXID,
                0,
                &elsewhere(),
                json!({"type": 10, "items": []}),
            ))),
            other => panic!("unexpected coin lookup {other:?}"),
        })
}

/// A transaction mined at `height` whose input 0 spends the purchase's
/// TRANSFER into output 0 with `covenant` at `to`, followed by `extra`
/// outputs, as hsd's `GET /tx/address` lists it.
fn spender(height: i64, covenant: u8, to: &str, extra: Vec<Value>) -> Value {
    let mut outputs = vec![json!({"address": to, "covenant": {"type": covenant, "items": []}})];
    outputs.extend(extra);
    json!({
        "hash": "6".repeat(64),
        "height": height,
        "inputs": [{"prevout": {"hash": PURCHASE_TXID, "index": 0}}],
        "outputs": outputs,
    })
}

/// Our FINALIZE (covenant 10), mined at `height`.
fn finalize_at(height: i64) -> Value {
    spender(height, 10, &destination(), vec![])
}

/// Our FINALIZE is mined and the name has moved on since (the buyer sent it
/// elsewhere): the purchase was owned. It is not left awaiting a finalize
/// the backend refuses for ever.
#[tokio::test]
async fn finalized_name_moved_on_since_is_owned() {
    let conn = seeded("broadcasted", "awaiting_finalize");
    let rpc = transfer_spent_and_name_elsewhere(vec![finalize_at(1290)]);
    run(&conn, &rpc).await;
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Owned);
    assert_eq!(p.purchase_height, Some(1000));
    assert_eq!(p.lost_reason, None);
    assert_eq!(
        rpc.calls_matching(|c| matches!(c, RpcCall::TxsByAddress(_))),
        vec![RpcCall::TxsByAddress(destination())]
    );
}

/// Owned is decided by the FINALIZE itself, never by its absence or by an
/// output that only happens to pay us: a history without a spender of our
/// TRANSFER (cut short by a proxy, or not indexed yet), a spender whose
/// linked output (the same index) is not a FINALIZE — which the lock script
/// does not allow — even when another of its outputs pays our destination,
/// an entry hsd would never send (no inputs, no prevout, no height, no linked
/// output or covenant), or a FINALIZE still in the mempool: no verdict that
/// ends the purchase, and nothing lost.
#[tokio::test]
async fn no_verdict_without_our_finalize_in_hsds_shape() {
    let mut no_inputs = finalize_at(1290);
    no_inputs.as_object_mut().unwrap().remove("inputs");
    let mut no_prevout = finalize_at(1290);
    no_prevout["inputs"][0]
        .as_object_mut()
        .unwrap()
        .remove("prevout");
    let mut no_height = finalize_at(1290);
    no_height.as_object_mut().unwrap().remove("height");
    let mut no_linked_output = finalize_at(1290);
    no_linked_output["outputs"] = json!([]);
    let mut no_covenant = finalize_at(1290);
    no_covenant["outputs"][0]
        .as_object_mut()
        .unwrap()
        .remove("covenant");
    let dust = json!({"address": destination(), "covenant": {"type": 0, "items": []}});
    let cases = [
        ("nothing", vec![]),
        (
            "an UPDATE paying us dust",
            vec![spender(1200, 7, "rs1qlock", vec![dust])],
        ),
        ("a REVOKE", vec![spender(1200, 11, "rs1qlock", vec![])]),
        ("no inputs", vec![no_inputs]),
        ("no prevout", vec![no_prevout]),
        ("no height", vec![no_height]),
        ("no linked output", vec![no_linked_output]),
        ("no covenant", vec![no_covenant]),
    ];
    for (case, history) in cases {
        let conn = seeded("broadcasted", "awaiting_finalize");
        run(&conn, &transfer_spent_and_name_elsewhere(history)).await;
        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::AwaitingFinalize,
            "{case}"
        );
        assert_eq!(p.lost_reason, None, "{case}");
    }
}

/// Our FINALIZE only in the mempool: the purchase awaits it.
#[tokio::test]
async fn finalize_in_the_mempool_keeps_the_purchase_awaiting() {
    let conn = seeded("broadcasted", "awaiting_finalize");
    run(
        &conn,
        &transfer_spent_and_name_elsewhere(vec![finalize_at(-1)]),
    )
    .await;
    assert_eq!(
        row(&conn).state,
        crate::db::queries::PurchaseState::AwaitingFinalize
    );
}

#[tokio::test]
async fn mined_purchase_still_owning_the_name_stays_awaiting_finalize() {
    let conn = seeded("broadcasted", "awaiting_finalize");
    run(&conn, &mined_and_committed(owned_by_purchase())).await;
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.lost_reason, None);
}

/// The TRANSFER coin is our own signed output, so a covenant the node leaves
/// out, cuts short, or reports as another type (our txid commits to a
/// TRANSFER) is not hsd saying it commits elsewhere: the purchase stays as it
/// is, its coins reserved, and Finalize stays possible.
#[tokio::test]
async fn transfer_coin_without_readable_covenant_never_loses_a_purchase() {
    let dest = destination();
    let (version, _) = address::decode(Network::Main, &dest).unwrap();
    let covenants = [
        None,
        Some(json!({"type": 9, "items": ["00".repeat(32), "01000000", hex::encode([version])]})),
        Some(json!({"type": 9, "items": []})),
        Some(json!({"type": 0, "items": []})),
        Some(
            json!({"type": 10, "items": ["00".repeat(32), "01000000", "dexreviews", "00", "00000000", "00000000", "00".repeat(32)]}),
        ),
    ];
    for covenant in covenants {
        let conn = seeded("confirmed", "unconfirmed");
        let cov = covenant.clone();
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1010))
            .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
            .with_name_info(owned_by_purchase())
            .with_get_coin(move |txid, vout| {
                assert_eq!((txid, vout), (PURCHASE_TXID, 0));
                let mut c = json!({
                    "hash": PURCHASE_TXID,
                    "index": 0,
                    "value": 0,
                    "address": "hs1qlock",
                    "height": 1000,
                });
                if let Some(cov) = cov.clone() {
                    c["covenant"] = cov;
                }
                Ok(Some(serde_json::from_value(c).unwrap()))
            });

        run(&conn, &rpc).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::Unconfirmed,
            "{covenant:?}"
        );
        assert_eq!(p.lost_reason, None, "{covenant:?}");
        assert_eq!(
            reservation(&conn).as_deref(),
            Some(DRAFT_ID),
            "{covenant:?}"
        );
    }
}

/// A TRANSFER that commits elsewhere exists on chain, so the purchase was
/// sent and paid even while its draft still reads `broadcasted`.
#[tokio::test]
async fn transfer_elsewhere_keeps_an_unconfirmed_draft_as_it_is() {
    let conn = seeded("broadcasted", "unconfirmed");
    let other = elsewhere();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_get_coin(move |_, _| {
            Ok(Some(coin(
                PURCHASE_TXID,
                0,
                "hs1qlock",
                transfer_to(&other),
            )))
        });
    run(&conn, &rpc).await;
    assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Lost);
    assert_eq!(draft_status(&conn).0, "broadcasted");
}

/// A file-backed copy of [`seeded`] whose purchase has been missing from the
/// node long enough to be rebroadcast, synced against a mock hsd at `url`.
fn missing_purchase_db(url: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "namehold_daemon_send_{}_{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).unwrap();
    seed_db(&conn, "broadcasted", "unconfirmed");
    conn.execute(
        "UPDATE shakedex_purchases SET missing_since_height = 1000 WHERE id = ?1",
        params![PURCHASE_ID],
    )
    .unwrap();
    queries::set_setting(&conn, "node_rpc_url", url).unwrap();
    path
}

/// A mock hsd at the tip that never saw the purchase while the seller's lock
/// coin is unspent. `sends` is how many `sendrawtransaction` calls it expects;
/// that mock comes first. `verificationprogress` below 1 is a node still
/// catching up.
async fn missing_purchase_node(
    server: &mut mockito::Server,
    sends: usize,
    verificationprogress: f64,
) -> Vec<mockito::Mock> {
    let rpc = |method: &str| mockito::Matcher::Regex(format!("\"method\":\\s*\"{method}\""));
    let lock_coin = coin_json(LOCK_TXID, 0, "hs1qlock", json!({"type": 10, "items": []}));
    let elsewhere_coin = coin_json(
        OTHER_TXID,
        0,
        &elsewhere(),
        json!({"type": 10, "items": []}),
    );
    vec![
        server
            .mock("POST", "/")
            .match_body(rpc("sendrawtransaction"))
            .with_body(json!({"result": PURCHASE_TXID, "error": null, "id": 1}).to_string())
            .expect(sends)
            .create_async()
            .await,
        server
            .mock("POST", "/")
            .match_body(rpc("getblockchaininfo"))
            .with_body(
                json!({"result": {"chain": "main", "blocks": 1021, "headers": 1021,
                    "verificationprogress": verificationprogress, "mediantime": LATE_MTP},
                    "error": null, "id": 1})
                .to_string(),
            )
            .create_async()
            .await,
        server
            .mock("POST", "/")
            .match_body(rpc("getnameinfo"))
            .with_body(json!({"result": foreign_owner(), "error": null, "id": 1}).to_string())
            .create_async()
            .await,
        server
            .mock("GET", format!("/tx/{PURCHASE_TXID}").as_str())
            .with_status(404)
            .create_async()
            .await,
        server
            .mock("GET", format!("/coin/{PURCHASE_TXID}/0").as_str())
            .with_status(404)
            .create_async()
            .await,
        server
            .mock("GET", format!("/coin/{LOCK_TXID}/0").as_str())
            .with_body(lock_coin.to_string())
            .create_async()
            .await,
        server
            .mock("GET", format!("/coin/{OTHER_TXID}/0").as_str())
            .with_body(elsewhere_coin.to_string())
            .create_async()
            .await,
    ]
}

/// The daemon runs the same sync steps as the app, and the purchase refresh in
/// them holds the one path that can send. The daemon's real entry point makes
/// no send call (SECURITY.md, "Daemon is read-only"), where the app's sync of
/// the same wallet against the same node sends the purchase's one rebroadcast.
#[tokio::test]
async fn daemon_sync_makes_no_send_call_where_the_apps_sync_does() {
    use crate::commands::sync::{run_sync_steps, SyncCaller, SyncStatus};

    for caller in [SyncCaller::Daemon, SyncCaller::App] {
        let mut server = mockito::Server::new_async().await;
        let sends = usize::from(caller == SyncCaller::App);
        let mocks = missing_purchase_node(&mut server, sends, 1.0).await;
        let path = missing_purchase_db(&server.url());
        let db_path = path.to_str().unwrap();

        match caller {
            SyncCaller::Daemon => crate::daemon::sync_profile(db_path, PROFILE).await,
            SyncCaller::App => {
                let status = std::sync::Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
                run_sync_steps(&status, db_path, PROFILE, SyncCaller::App).await;
            }
        }

        mocks[0].assert_async().await;
        let conn = Connection::open(&path).unwrap();
        assert_eq!(row(&conn).rebroadcast_count, sends as i64, "{caller:?}");
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}

/// The purchase step runs only against an authoritative node (R13): one
/// still catching up would report a mined purchase as unknown, and the app's
/// sync would rebroadcast it or declare it lost. The same wallet and node that
/// get the rebroadcast above, with the node behind, get no send and no change.
#[tokio::test]
async fn app_sync_leaves_purchases_alone_on_a_node_still_catching_up() {
    use crate::commands::sync::{run_sync_steps, SyncCaller, SyncStatus};

    let mut server = mockito::Server::new_async().await;
    let mocks = missing_purchase_node(&mut server, 0, 0.5).await;
    let path = missing_purchase_db(&server.url());
    let status = std::sync::Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

    run_sync_steps(&status, path.to_str().unwrap(), PROFILE, SyncCaller::App).await;

    mocks[0].assert_async().await;
    let conn = Connection::open(&path).unwrap();
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.rebroadcast_count, 0);
    assert_eq!(p.missing_since_height, Some(1000));
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

/// The daemon's sync never rebroadcasts (SECURITY.md); the app's may.
#[test]
fn only_the_apps_sync_may_rebroadcast() {
    use crate::commands::sync::SyncCaller;
    assert_eq!(SyncCaller::Daemon.rebroadcast(), Rebroadcast::Never);
    assert_eq!(SyncCaller::App.rebroadcast(), Rebroadcast::Allowed);
}

/// hsd always reports a live name's owner, and the owner coin's address. A
/// reply that leaves either out proves nothing: the purchase keeps its state
/// and its coins until a later sync can tell, rather than being declared
/// lost on a guess, which would release its coins.
#[tokio::test]
async fn name_reply_without_owner_never_loses_a_purchase() {
    let replies = [
        json!({}),
        json!({"info": {"name": NAME}}),
        json!({"info": {"name": NAME, "owner": {"index": 0}}}),
        json!({"info": {"name": NAME, "owner": {"hash": OTHER_TXID}}}),
    ];
    for name_info in replies {
        let conn = seeded("failed", "pending_send");
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1010))
            .with_name_info(name_info.clone())
            .with_get_coin(|txid, vout| {
                assert_eq!((txid, vout), (PURCHASE_TXID, 0));
                Ok(None)
            });

        run(&conn, &rpc).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::PendingSend,
            "{name_info}"
        );
        assert_eq!(p.lost_reason, None, "{name_info}");
        assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID), "{name_info}");
        assert_eq!(draft_status(&conn), ("failed".to_string(), None));
    }
}

#[tokio::test]
async fn owner_coin_without_address_never_loses_a_purchase() {
    let conn = seeded("failed", "pending_send");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_name_info(foreign_owner())
        .with_get_coin(|txid, vout| match (txid, vout) {
            (PURCHASE_TXID, 0) => Ok(None),
            (OTHER_TXID, 0) => Ok(Some(
                serde_json::from_value(json!({"hash": OTHER_TXID, "index": 0, "value": 0}))
                    .unwrap(),
            )),
            other => panic!("unexpected coin lookup {other:?}"),
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::PendingSend);
    assert_eq!(p.lost_reason, None);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

/// hsd's `GET /coin` answers "no coin" for a coin spent in its mempool
/// (`fullnode.js` `getCoin`: `mempool.isSpent` → null), while `getnameinfo`
/// still names it as the owner from the chain. Our FINALIZE mined and an
/// UPDATE of it in the mempool look like this on a node without a
/// transaction index (the purchase, its TRANSFER and the lock coin all read
/// as gone): the owner coin says nothing about whose it is, so the purchase
/// is not lost until the spend is mined.
#[tokio::test]
async fn owner_coin_spent_in_the_mempool_never_loses_a_purchase() {
    let conn = seeded("broadcasted", "unconfirmed");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_tx_by_hash(Value::Null)
        .with_name_info(foreign_owner())
        .with_get_coin(|txid, vout| match (txid, vout) {
            (LOCK_TXID, 0) | (PURCHASE_TXID, 0) | (OTHER_TXID, 0) => Ok(None),
            other => panic!("unexpected coin lookup {other:?}"),
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.lost_reason, None);
    assert_eq!(reservation(&conn).as_deref(), Some(DRAFT_ID));
}

/// hsd answers `"info": null` for an expired name; a reply without the key
/// at all is not that answer, so a mined purchase keeps waiting.
#[tokio::test]
async fn name_reply_without_info_key_is_not_an_expiry() {
    let conn = seeded("broadcasted", "awaiting_finalize");
    run(&conn, &mined_and_committed(json!({}))).await;
    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.lost_reason, None);
}

/// A purchase marked lost although it paid nothing, whose draft was dropped
/// with `reason` — `updated` is SQLite's modifier for when that happened.
fn lost_unpaid(updated: &str) -> Connection {
    let conn = seeded("dropped", "lost");
    conn.execute(
        "UPDATE shakedex_purchases
            SET lost_reason = 'the purchase never confirmed — nothing was paid',
                updated_at = datetime('now', ?1)
          WHERE id = ?2",
        params![updated, PURCHASE_ID],
    )
    .unwrap();
    conn.execute(
        "UPDATE wallet_tx_drafts
            SET error_message = 'the purchase never confirmed — nothing was paid'
          WHERE id = ?1",
        params![DRAFT_ID],
    )
    .unwrap();
    queries::release_reserved_utxos_for_draft(&conn, DRAFT_ID).unwrap();
    conn
}

/// hsd forgets a transaction after 72 hours in its mempool, but another
/// node may still mine it — or a reorg may bring back the purchase that a
/// competing buyer's had replaced. A purchase lost while paying nothing is
/// checked again for a week: once its TRANSFER is mined to us it paid, and
/// it is awaiting finalize again, its draft no longer saying otherwise.
#[tokio::test]
async fn unpaid_lost_purchase_that_mines_later_awaits_finalize_again() {
    let conn = lost_unpaid("-1 day");
    let rpc = mined_and_committed(owned_by_purchase());

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.purchase_height, Some(1000));
    assert_eq!(p.blocks_remaining, Some(1000 + LOCKUP - (1010 + 1)));
    assert_eq!(p.lost_reason, None);
    assert_eq!(draft_status(&conn), ("broadcasted".to_string(), None));
}

/// Our TRANSFER only in the mempool proves nothing yet (and the listing may
/// be bought again meanwhile): the purchase stays lost until it is mined.
#[tokio::test]
async fn unpaid_lost_purchase_only_in_the_mempool_stays_lost() {
    let conn = lost_unpaid("-1 day");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_get_coin(move |txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0));
            Ok(Some(coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest))))
        });

    run(&conn, &rpc).await;

    assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Lost);
    assert_eq!(draft_status(&conn).0, "dropped");
}

/// Mined after it was given up, and finalized since by whoever sent the
/// FINALIZE (anyone may, once the lockup is over — another copy of this
/// wallet, the seller): the TRANSFER coin is spent and the name sits at our
/// destination. The purchase paid and the name is ours, so it is owned, and
/// its draft no longer says nothing was paid.
#[tokio::test]
async fn unpaid_lost_purchase_finalized_since_is_owned() {
    let conn = lost_unpaid("-3 days");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1400))
        .with_tx_by_hash(json!({"hash": PURCHASE_TXID, "height": 1000}))
        .with_name_info(foreign_owner())
        .with_get_coin(move |txid, vout| match (txid, vout) {
            (PURCHASE_TXID, 0) => Ok(None),
            (OTHER_TXID, 0) => Ok(Some(coin(
                OTHER_TXID,
                0,
                &dest,
                json!({"type": 10, "items": []}),
            ))),
            other => panic!("unexpected coin lookup {other:?}"),
        });

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Owned);
    assert_eq!(p.purchase_height, Some(1000));
    assert_eq!(p.lost_reason, None);
    assert_eq!(draft_status(&conn), ("broadcasted".to_string(), None));
}

/// The name at our destination proves nothing about this purchase unless
/// its own transaction is mined: a later purchase of the same listing may
/// have been given the same unused address, and only one purchase of a lock
/// coin can ever be mined. Without it, the lost purchase stays lost.
#[tokio::test]
async fn unpaid_lost_purchase_is_not_owned_by_another_purchases_finalize() {
    let conn = lost_unpaid("-3 days");
    let dest = destination();
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1400))
        .with_tx_by_hash(Value::Null)
        .with_name_info(foreign_owner())
        .with_get_coin(move |txid, vout| match (txid, vout) {
            (PURCHASE_TXID, 0) => Ok(None),
            (OTHER_TXID, 0) => Ok(Some(coin(
                OTHER_TXID,
                0,
                &dest,
                json!({"type": 10, "items": []}),
            ))),
            other => panic!("unexpected coin lookup {other:?}"),
        });

    run(&conn, &rpc).await;

    assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Lost);
    assert_eq!(draft_status(&conn).0, "dropped");
}

/// The TRANSFER coin spent and the name at someone else's address: nothing
/// traces to this purchase, and it stays lost with nothing paid.
#[tokio::test]
async fn unpaid_lost_purchase_whose_name_is_elsewhere_stays_lost() {
    let conn = lost_unpaid("-3 days");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1400))
        .with_name_info(foreign_owner())
        .with_get_coin(|txid, vout| untraced(txid, vout, None));

    run(&conn, &rpc).await;

    assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Lost);
    assert_eq!(draft_status(&conn).0, "dropped");
}

/// After a week nothing can still bring the purchase back, and a lost
/// purchase that did pay (its draft reached the chain) is final already:
/// neither costs a lookup.
#[tokio::test]
async fn old_or_paid_lost_purchases_are_not_looked_up() {
    let old = lost_unpaid("-8 days");
    let paid = seeded("broadcasted", "lost");
    for conn in [old, paid] {
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1010))
            .with_get_coin(|txid, vout| panic!("unexpected coin lookup {txid}:{vout}"));
        run(&conn, &rpc).await;
        assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Lost);
    }
}

/// Mined after it was given up, but the name expired meanwhile: it paid, so
/// it is lost for that reason, and its draft no longer says nothing was paid.
#[tokio::test]
async fn unpaid_lost_purchase_mined_after_the_name_expired_says_it_paid() {
    let conn = lost_unpaid("-1 day");
    run(&conn, &mined_and_committed(json!({"info": null}))).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::Lost);
    let reason = p.lost_reason.unwrap();
    assert!(
        reason.contains("expired before it was finalized"),
        "{reason}"
    );
    assert_eq!(draft_status(&conn), ("broadcasted".to_string(), None));
}

// --- owned, and a reorg that undoes the FINALIZE (R13) -----------------------

/// A purchase that reached `owned` `updated` ago (SQLite's modifier).
fn owned(updated: &str) -> Connection {
    let conn = seeded("confirmed", "owned");
    conn.execute(
        "UPDATE shakedex_purchases
            SET purchase_height = 1000, blocks_remaining = 0,
                updated_at = datetime('now', ?1)
          WHERE id = ?2",
        params![updated, PURCHASE_ID],
    )
    .unwrap();
    conn
}

/// The FINALIZE spends the purchase's TRANSFER coin. If that coin is
/// unspent again, a reorg took the FINALIZE away: the purchase is awaiting
/// finalize again (R13: "A reorg moves the state back"), so Finalize is
/// offered once more instead of the name being shown as ours.
#[tokio::test]
async fn owned_purchase_whose_finalize_was_reorged_away_awaits_finalize_again() {
    let conn = owned("-1 day");
    let rpc = mined_and_committed(owned_by_purchase());

    run(&conn, &rpc).await;

    let p = row(&conn);
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.purchase_height, Some(1000));
    assert_eq!(p.blocks_remaining, Some(1000 + LOCKUP - (1010 + 1)));
    assert_eq!(p.lost_reason, None);
}

/// Spent TRANSFER: the FINALIZE (or whatever the owner did with the name
/// since) stands, and the purchase stays owned. The name's later moves are
/// not looked at — a name the buyer has since sold is still a purchase that
/// was owned.
#[tokio::test]
async fn owned_purchase_whose_transfer_stays_spent_stays_owned() {
    let conn = owned("-1 day");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_get_coin(|txid, vout| {
            assert_eq!((txid, vout), (PURCHASE_TXID, 0));
            Ok(None)
        });

    run(&conn, &rpc).await;

    assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Owned);
    assert_eq!(rpc.count_matching(|c| matches!(c, RpcCall::NameInfo(_))), 0);
}

/// hsd always sends a coin's height, -1 in the mempool (`Coin.getJSON`). An
/// unspent TRANSFER whose reply leaves it out, or sends a height hsd never
/// does, says nothing about where the purchase is: the owned purchase stays
/// owned rather than being moved back to unconfirmed on a guess.
#[tokio::test]
async fn owned_purchase_whose_transfer_reply_has_no_height_stays_owned() {
    for height in [None, Some(-2)] {
        let conn = owned("-1 day");
        let dest = destination();
        let rpc = MockNodeRpc::new()
            .with_blockchain_info(tip(1010))
            .with_get_coin(move |txid, vout| {
                assert_eq!((txid, vout), (PURCHASE_TXID, 0));
                let mut c = coin(PURCHASE_TXID, 0, "hs1qlock", transfer_to(&dest));
                c.height = height;
                Ok(Some(c))
            });

        run(&conn, &rpc).await;

        let p = row(&conn);
        assert_eq!(
            p.state,
            crate::db::queries::PurchaseState::Owned,
            "{height:?}"
        );
        assert_eq!(p.purchase_height, Some(1000), "{height:?}");
    }
}

/// Past the window a reorg that deep is not looked for: no lookups.
#[tokio::test]
async fn old_owned_purchases_are_not_looked_up() {
    let conn = owned("-8 days");
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(tip(1010))
        .with_get_coin(|txid, vout| panic!("unexpected coin lookup {txid}:{vout}"));
    run(&conn, &rpc).await;
    assert_eq!(row(&conn).state, crate::db::queries::PurchaseState::Owned);
}
