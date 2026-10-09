//! R22 and the listing's states after the FINALIZE into the lock (T4): the
//! store's guarded writes, and the after-lock job — SalePending and Sold
//! from a purchase of the lock coin found on chain, their reorgs back,
//! Finalizing when the FINALIZE into the lock is nowhere, and Expired.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::db::queries::{self, ListingMode, ListingState, ShakedexListing};
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::ExtendedPrivKey;
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::NodeCoin;
use crate::noncustodial::shakedex::lock_key::derive_lock_key;
use crate::noncustodial::shakedex::sell::LOCK_FINALIZE_ACTION;
use crate::noncustodial::sync::{COV_FINALIZE, COV_TRANSFER};
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};
use crate::tests::shakedex_cmd_tests::{seed, seeded, PROFILE};

const NET: Network = Network::Regtest;
const NAME: &str = "dexstate";
const NAME_HEIGHT: u32 = 50;
const TIP: i64 = 3_000;
const PRICE: i64 = 5_000_000;

fn txid(byte: &str) -> String {
    byte.repeat(32)
}

/// A regtest profile with a listing of NAME in `state`: its lock coin
/// `(f1…, 0)` at the lock address of the seed's key for NAME, its lock
/// TRANSFER `e1…`, a confirmed FINALIZE draft, a listing file, and reserved
/// payment and cancel addresses of ours.
struct Fx {
    conn: Connection,
    id: String,
    lock: String,
    lock_txid: String,
    lock_vout: u32,
    transfer_txid: String,
    payment: String,
    cancel: String,
    buyer: String,
}

fn fx(state: ListingState) -> Fx {
    let conn = seeded("regtest", "mnemonic_hot", "http://127.0.0.1:9");
    let payment = derivation::reserve_receive_address(&conn, PROFILE).unwrap();
    let cancel = derivation::reserve_receive_address(&conn, PROFILE).unwrap();
    let key = derive_lock_key(&ExtendedPrivKey::from_seed(&seed()).unwrap(), NET, 0, NAME).unwrap();
    queries::insert_tx_draft(
        &conn,
        "fin",
        PROFILE,
        LOCK_FINALIZE_ACTION,
        "00",
        "{}",
        "{}",
    )
    .unwrap();
    queries::update_tx_draft_confirmation(&conn, "fin", TIP - 20, Some(&txid("f1"))).unwrap();
    let l = ShakedexListing {
        id: "l1".into(),
        wallet_profile_id: PROFILE.into(),
        name: NAME.into(),
        mode: ListingMode::BuyNow,
        state,
        lock_pubkey_hex: hex::encode(key.pubkey),
        lock_transfer_draft_id: None,
        lock_finalize_draft_id: Some("fin".into()),
        lock_transfer_txid: Some(txid("e1")),
        lock_txid: Some(txid("f1")),
        lock_vout: Some(0),
        payment_address: Some(payment.address.clone()),
        cancel_address: Some(cancel.address.clone()),
        cancel_child_index: Some(i64::from(cancel.child_index)),
        steps_json: r#"[{"price":5000000,"lockTime":1,"signature":"ab"}]"#.into(),
        listing_file_json: Some("{}".into()),
        publish: false,
        market_status: None,
        market_retry_at: None,
        expires_at: Some(1_731_536_000),
        abort_draft_id: None,
        abort_txid: None,
        sold_txid: None,
        cancel_txid: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    Fx {
        conn,
        id: "l1".into(),
        lock: key.address,
        lock_txid: txid("f1"),
        lock_vout: 0,
        transfer_txid: txid("e1"),
        payment: payment.address,
        cancel: cancel.address,
        buyer: address::encode_p2wpkh(NET, &[9; 20]).unwrap(),
    }
}

fn listing(fx: &Fx) -> ShakedexListing {
    queries::get_shakedex_listing(&fx.conn, &fx.id)
        .unwrap()
        .unwrap()
}

fn set_state(fx: &Fx, state: ListingState) {
    fx.conn
        .execute(
            "UPDATE shakedex_listings SET state = ?1 WHERE id = ?2",
            params![state, fx.id],
        )
        .unwrap();
}

/// Our coin of `txid` at the payment address, as the wallet's sync records
/// it: `height` -1 while it is in the mempool; `spent` once a later sync no
/// longer finds it.
fn paid(fx: &Fx, txid: &str, vout: u32, height: i64, spent: bool) {
    fx.conn
        .execute(
            "INSERT OR REPLACE INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex, value_doos,
                 height, covenant_type, spend_class, spent_by_txid)
             VALUES (?1, ?2, ?3, ?4, '00', ?5, ?6, 0, 'liquid_hns', ?7)",
            params![
                txid,
                vout,
                PROFILE,
                fx.payment,
                PRICE,
                height,
                spent.then_some("spent")
            ],
        )
        .unwrap();
}

#[test]
fn sale_writes_move_only_a_listing_in_our_lock() {
    // The sets are pinned as literals: no terminal or cancel state is ever
    // moved by a sale or an expiry.
    let set = |a: &[ListingState]| {
        let mut v: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(
        set(&ListingState::SALE_FROM),
        [
            "finalizing",
            "listed",
            "ready_to_finalize",
            "restored",
            "sale_pending"
        ]
    );
    assert_eq!(
        set(&ListingState::LOCKED_EXPIRABLE),
        ["listed", "restored", "sale_pending"]
    );
    for state in ListingState::ALL {
        let f = fx(state);
        let pending =
            queries::mark_listing_sale_pending(&f.conn, &f.id, &txid("b1"), None).unwrap();
        assert_eq!(
            pending == 1,
            ListingState::SALE_FROM.contains(&state),
            "{state:?}: pending"
        );
        set_state(&f, state);
        let sold = queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), None).unwrap();
        assert_eq!(
            sold == 1,
            ListingState::SALE_FROM.contains(&state),
            "{state:?}: sold"
        );
        set_state(&f, state);
        let expired = queries::expire_locked_listing(&f.conn, &f.id).unwrap();
        assert_eq!(
            expired == 1,
            ListingState::LOCKED_EXPIRABLE.contains(&state),
            "{state:?}: expired"
        );
        set_state(&f, state);
        let unadopted = queries::unadopt_restored_lock(&f.conn, &f.id).unwrap();
        assert_eq!(
            unadopted == 1,
            state == ListingState::Restored,
            "{state:?}: unadopt"
        );
    }
    // A sale of the outpoint the listing has is written; a purchase of
    // another outpoint is another lock's and changes nothing.
    let f = fx(ListingState::Listed);
    let other = (txid("c1"), 3);
    let o = Some((other.0.as_str(), other.1));
    assert_eq!(
        queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), o).unwrap(),
        0
    );
    assert_eq!(
        queries::mark_listing_sale_pending(&f.conn, &f.id, &txid("b1"), o).unwrap(),
        0
    );
    let l = listing(&f);
    assert_eq!((l.state, l.sold_txid), (ListingState::Listed, None));
    assert_eq!(
        (l.lock_txid.as_deref(), l.lock_vout),
        (Some(txid("f1").as_str()), Some(0))
    );
    let vout = Some((f.lock_txid.as_str(), 1));
    assert_eq!(
        queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), vout).unwrap(),
        0
    );
    let same = Some((f.lock_txid.as_str(), 0));
    assert_eq!(
        queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), same).unwrap(),
        1
    );
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(txid("b1").as_str()))
    );
    // A listing without an outpoint (a dead FINALIZE's) learns it from the
    // purchase, both halves together; without one from either side no sale is
    // written.
    for pending in [false, true] {
        let f = fx(ListingState::ReadyToFinalize);
        f.conn
            .execute(
                "UPDATE shakedex_listings SET lock_txid = NULL, lock_vout = NULL",
                [],
            )
            .unwrap();
        let write = |lock| {
            if pending {
                queries::mark_listing_sale_pending(&f.conn, &f.id, &txid("b1"), lock).unwrap()
            } else {
                queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), lock).unwrap()
            }
        };
        assert_eq!(write(None), 0, "no outpoint anywhere (pending {pending})");
        assert_eq!(listing(&f).state, ListingState::ReadyToFinalize);
        assert_eq!(write(o), 1, "pending {pending}");
        let l = listing(&f);
        assert_eq!(
            (l.lock_txid.as_deref(), l.lock_vout),
            (Some(txid("c1").as_str()), Some(3))
        );
    }
    // A Restored lock taken back by a reorg is Locking without its outpoint;
    // one restored by name (no lock TRANSFER known) is not moved.
    let f = fx(ListingState::Restored);
    queries::unadopt_restored_lock(&f.conn, &f.id).unwrap();
    let l = listing(&f);
    assert_eq!((l.state, l.lock_txid), (ListingState::Locking, None));
    let f = fx(ListingState::Restored);
    f.conn
        .execute("UPDATE shakedex_listings SET lock_transfer_txid = NULL", [])
        .unwrap();
    assert_eq!(queries::unadopt_restored_lock(&f.conn, &f.id).unwrap(), 0);
    // A file upgrades only a Restored lock tracking that very outpoint.
    let upgrade_key = |f: &Fx, vout: u32, key: &str| {
        queries::upgrade_restored_lock(
            &f.conn,
            &f.id,
            &f.lock_txid,
            vout,
            key,
            &queries::UpgradedListing {
                mode: ListingMode::BuyNow,
                payment_address: &f.payment,
                steps_json: "[]",
                listing_file_json: "{}",
                expires_at: None,
            },
        )
        .unwrap()
    };
    let upgrade = |f: &Fx, vout: u32| {
        let key = listing(f).lock_pubkey_hex;
        upgrade_key(f, vout, &key)
    };
    assert_eq!(upgrade(&fx(ListingState::Listed), 0), 0, "not Restored");
    assert_eq!(
        upgrade_key(&fx(ListingState::Restored), 0, "02ab"),
        0,
        "another key"
    );
    assert_eq!(
        upgrade(&fx(ListingState::Restored), 1),
        0,
        "another outpoint"
    );
    let f = fx(ListingState::Restored);
    assert_eq!(upgrade(&f, 0), 1);
    assert_eq!(listing(&f).state, ListingState::Listed);
}

#[test]
fn a_sold_listing_goes_back_only_while_no_other_listing_of_the_name_is_open() {
    let f = fx(ListingState::Sold);
    f.conn
        .execute("UPDATE shakedex_listings SET sold_txid = ?1", [txid("b1")])
        .unwrap();
    // Any `to` but these three is a bug in the caller.
    assert!(queries::unsell_shakedex_listing(&f.conn, &f.id, ListingState::Locking).is_err());
    let mut newer = listing(&f);
    newer.id = "l2".into();
    newer.state = ListingState::Locking;
    newer.lock_txid = None;
    newer.lock_vout = None;
    queries::insert_shakedex_listing(&f.conn, &newer).unwrap();
    assert_eq!(
        queries::unsell_shakedex_listing(&f.conn, &f.id, ListingState::Listed).unwrap(),
        0
    );
    assert_eq!(listing(&f).state, ListingState::Sold);
    f.conn
        .execute("DELETE FROM shakedex_listings WHERE id = 'l2'", [])
        .unwrap();
    for to in [
        ListingState::Listed,
        ListingState::Finalizing,
        ListingState::Restored,
    ] {
        set_state(&f, ListingState::Sold);
        assert_eq!(
            queries::unsell_shakedex_listing(&f.conn, &f.id, to).unwrap(),
            1,
            "{to:?}"
        );
        let l = listing(&f);
        assert_eq!((l.state, l.sold_txid), (to, None), "{to:?}");
    }
    set_state(&f, ListingState::SalePending);
    assert_eq!(
        queries::unsell_shakedex_listing(&f.conn, &f.id, ListingState::Listed).unwrap(),
        1
    );
    set_state(&f, ListingState::Listed);
    assert_eq!(
        queries::unsell_shakedex_listing(&f.conn, &f.id, ListingState::Listed).unwrap(),
        0
    );
}

#[test]
fn own_coins_at_the_payment_address_include_spent_ones() {
    let f = fx(ListingState::Listed);
    paid(&f, &txid("b1"), 2, TIP, true);
    paid(&f, &txid("b2"), 1, -1, false);
    f.conn
        .execute(
            "INSERT INTO tracked_utxos (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, height, covenant_type, spend_class)
             VALUES (?1, 0, ?2, ?3, '00', 1, 5, 0, 'liquid_hns')",
            params![txid("b3"), PROFILE, f.cancel],
        )
        .unwrap();
    let got = queries::own_coins_at(&f.conn, PROFILE, &f.payment).unwrap();
    assert_eq!(got, vec![(txid("b1"), Some(TIP)), (txid("b2"), Some(-1))]);
    assert!(queries::own_coin_in_tx(&f.conn, PROFILE, &txid("b1"), &f.payment).unwrap());
    assert!(!queries::own_coin_in_tx(&f.conn, PROFILE, &txid("b3"), &f.payment).unwrap());
    assert!(queries::own_coins_at(&f.conn, "other", &f.payment)
        .unwrap()
        .is_empty());
}

#[test]
fn after_lock_job_rechecks_sold_listings_only_within_the_window() {
    let f = fx(ListingState::Sold);
    assert_eq!(
        queries::list_shakedex_listings_after_lock(&f.conn, PROFILE, 7)
            .unwrap()
            .len(),
        1
    );
    f.conn
        .execute(
            "UPDATE shakedex_listings SET updated_at = datetime('now', '-8 days')",
            [],
        )
        .unwrap();
    assert!(
        queries::list_shakedex_listings_after_lock(&f.conn, PROFILE, 7)
            .unwrap()
            .is_empty()
    );
    set_state(&f, ListingState::Restored);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET updated_at = datetime('now', '-80 days')",
            [],
        )
        .unwrap();
    assert_eq!(
        queries::list_shakedex_listings_after_lock(&f.conn, PROFILE, 7)
            .unwrap()
            .len(),
        1
    );
}

fn name_hash() -> String {
    hex::encode(names::hash_name(NAME).unwrap())
}

fn height_item(h: u32) -> String {
    hex::encode(h.to_le_bytes())
}

/// A coin as hsd's `GET /coin` sends it.
fn coin(
    txid: &str,
    vout: u32,
    address: &str,
    cov: u8,
    items: Vec<String>,
    height: i64,
) -> NodeCoin {
    serde_json::from_value(json!({
        "version": 0, "height": height, "value": 1_000_000, "address": address,
        "covenant": { "type": cov, "action": "", "items": items },
        "coinbase": false, "hash": txid, "index": vout
    }))
    .unwrap()
}

/// The lock coin: a FINALIZE of NAME at the lock address.
fn lock_coin(f: &Fx, height: i64) -> NodeCoin {
    coin(
        &f.lock_txid,
        f.lock_vout,
        &f.lock,
        COV_FINALIZE,
        vec![name_hash(), height_item(NAME_HEIGHT)],
        height,
    )
}

/// Output 0 of `txid`: a TRANSFER of NAME at our lock committing to `to`.
fn transfer_out_of_lock(f: &Fx, txid: &str, to: &str, height: i64) -> NodeCoin {
    let (v, h) = address::decode(NET, to).unwrap();
    coin(
        txid,
        0,
        &f.lock,
        COV_TRANSFER,
        vec![
            name_hash(),
            height_item(NAME_HEIGHT),
            hex::encode([v]),
            hex::encode(h),
        ],
        height,
    )
}

/// `getnameinfo` with `owner` the name's owner outpoint.
fn info(owner: (&str, u32)) -> Value {
    json!({ "info": {
        "name": NAME, "state": "CLOSED", "height": NAME_HEIGHT, "renewal": 1_000,
        "renewals": 0, "claimed": 0, "weak": false, "transfer": 0, "revoked": 0,
        "owner": { "hash": owner.0, "index": owner.1 }, "value": 1_000_000
    }, "start": null })
}

/// The purchase `txid` as hsd's `GET /tx` sends it (`height` -1 in the
/// mempool): input 0 our lock coin, output 0 the TRANSFER at the lock
/// committing to the buyer, output 1 change, output 2 the price to `pay`.
fn purchase_rest(f: &Fx, txid: &str, height: i64, pay: &str) -> Value {
    let (_, h) = address::decode(NET, &f.buyer).unwrap();
    json!({
        "hash": txid, "height": height, "hex": "00",
        "inputs": [ { "prevout": { "hash": f.lock_txid, "index": f.lock_vout } },
                    { "prevout": { "hash": "aa".repeat(32), "index": 1 } } ],
        "outputs": [
            { "value": 1_000_000, "address": f.lock, "covenant": { "type": COV_TRANSFER,
              "action": "TRANSFER",
              "items": [name_hash(), height_item(NAME_HEIGHT), "00", hex::encode(h)] } },
            { "value": 1, "address": f.buyer,
              "covenant": { "type": 0, "action": "NONE", "items": [] } },
            { "value": PRICE, "address": pay,
              "covenant": { "type": 0, "action": "NONE", "items": [] } }
        ]
    })
}

/// A node answering `info`, `GET /coin` from `coins` (hsd's empty 404 for
/// any other outpoint), and `GET /tx` with `tx` (`null`: hsd's not-found).
fn node(info: Value, coins: Vec<NodeCoin>, tx: Value) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_name_info(info)
        .with_tx_by_hash(tx)
        .with_get_coin(move |txid, vout| {
            Ok(coins
                .iter()
                .find(|c| c.txid == txid && c.vout == vout)
                .cloned())
        })
}

/// Run the listing step's body on the fixture's database; it sends nothing.
async fn run(f: &Fx, rpc: &MockNodeRpc) {
    crate::shakedex_jobs::refresh_listings_with_client(&f.conn, rpc, PROFILE)
        .await
        .expect("listing step runs");
    assert_eq!(
        rpc.count_matching(|c| matches!(c, RpcCall::SendRawTransaction(_))),
        0,
        "the listing step sends nothing (it runs in the daemon too)"
    );
}

/// R22: the lock coin bought. hsd moves the name's owner to the purchase's
/// TRANSFER (output 0) out of our lock, committing to an address not ours,
/// and the wallet has a coin of that transaction at the payment address:
/// Sold with the purchase's txid, the lock outpoint kept — from Listed, from
/// a purchase seen pending, from a Restored lock that has a payment address,
/// and from Finalizing (bought before a sync saw the FINALIZE mined).
#[tokio::test]
async fn sold_on_purchase() {
    for from in [
        ListingState::Listed,
        ListingState::SalePending,
        ListingState::Restored,
        ListingState::Finalizing,
    ] {
        let f = fx(from);
        let buy = txid("b1");
        paid(&f, &buy, 2, TIP, false);
        let chain = node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            Value::Null,
        );
        run(&f, &chain).await;
        let l = listing(&f);
        assert_eq!(l.state, ListingState::Sold, "{from:?}");
        assert_eq!(l.sold_txid.as_deref(), Some(buy.as_str()), "{from:?}");
        assert_eq!(
            (l.lock_txid.as_deref(), l.lock_vout),
            (Some(f.lock_txid.as_str()), Some(0)),
            "{from:?}"
        );
    }
}

/// R22: coins arriving at the payment address on their own mean nothing.
/// (a) a coin there while the lock coin is unspent; (b) the lock coin spent
/// into a TRANSFER at our lock committing to our own cancel address — even
/// with a coin of ours in that very transaction at the payment address, so
/// the owner path reads the commitment and says "ours"; (c) the lock coin
/// bought by a transaction that does not pay us, and a payment from another
/// one that buys nothing, read from hsd (so the "no" comes from the rule,
/// not from a read the mock does not answer).
#[tokio::test]
async fn payment_alone_is_not_sold() {
    let other = txid("d1");
    // (a)
    let f = fx(ListingState::Listed);
    paid(&f, &other, 0, TIP, false);
    let mut gift = purchase_rest(&f, &other, TIP, &f.payment);
    gift["outputs"][0] = json!({ "value": 1, "address": f.buyer,
        "covenant": { "type": 0, "action": "NONE", "items": [] } });
    let chain = node(
        info((&f.lock_txid, 0)),
        vec![lock_coin(&f, TIP - 20)],
        gift.clone(),
    );
    run(&f, &chain).await;
    assert_eq!(listing(&f).state, ListingState::Listed, "lock coin unspent");
    // (b)
    let f = fx(ListingState::Listed);
    let cancel = txid("c1");
    paid(&f, &cancel, 1, TIP, false);
    let chain = node(
        info((&cancel, 0)),
        vec![transfer_out_of_lock(&f, &cancel, &f.cancel, TIP)],
        gift.clone(),
    );
    run(&f, &chain).await;
    assert_eq!(listing(&f).state, ListingState::Listed, "our cancel");
    assert!(
        chain.count_matching(|c| matches!(c, RpcCall::GetCoin(t, 0) if *t == cancel)) > 0,
        "the owner coin was read"
    );
    // (c)
    let f = fx(ListingState::Listed);
    let buy = txid("b1");
    paid(&f, &other, 0, TIP, false);
    let chain = node(
        info((&buy, 0)),
        vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
        gift,
    );
    run(&f, &chain).await;
    assert_eq!(
        listing(&f).state,
        ListingState::Listed,
        "bought, but not paying us"
    );
    assert!(
        chain.count_matching(|c| matches!(c, RpcCall::TxByHash(t) if *t == other)) > 0,
        "the paying transaction was read"
    );
    // (d) the owner a TRANSFER committing to the buyer in a transaction that
    // pays us, but not out of our lock: at another address, or of another
    // name at our lock.
    let (v, h) = address::decode(NET, &f.buyer).unwrap();
    let commit = |name: String| {
        vec![
            name,
            height_item(NAME_HEIGHT),
            hex::encode([v]),
            hex::encode(h.clone()),
        ]
    };
    let other_name = hex::encode(names::hash_name("othername").unwrap());
    for (case, at, items) in [
        ("not at our lock", f.buyer.clone(), commit(name_hash())),
        ("another name", f.lock.clone(), commit(other_name)),
    ] {
        let f = fx(ListingState::Listed);
        let x = txid("e7");
        paid(&f, &x, 2, TIP, false);
        let chain = node(
            info((&x, 0)),
            vec![coin(&x, 0, &at, COV_TRANSFER, items, TIP)],
            gift_of(&f, &x),
        );
        run(&f, &chain).await;
        assert_eq!(listing(&f).state, ListingState::Listed, "{case}");
    }
}

/// A transaction that pays the payment address and buys nothing.
fn gift_of(f: &Fx, txid: &str) -> Value {
    let mut gift = purchase_rest(f, txid, TIP, &f.payment);
    gift["outputs"][0] = json!({ "value": 1, "address": f.buyer,
        "covenant": { "type": 0, "action": "NONE", "items": [] } });
    gift
}

/// R22: the purchase only in the mempool — hsd answers 404 for the lock coin
/// a mempool transaction spends, and the name's owner is still the lock
/// coin (it moves when a block is connected); our coin of the purchase is
/// at -1, and `GET /tx` finds it in the mempool: SalePending. A purchase
/// that `GET /tx` reports mined while the owner is still the lock coin is
/// two hsd facts that disagree: no verdict. Mined, Sold.
#[tokio::test]
async fn sale_pending_while_the_purchase_is_in_the_mempool() {
    let f = fx(ListingState::Listed);
    let buy = txid("b1");
    paid(&f, &buy, 2, -1, false);
    let lock_owner = info((&f.lock_txid, 0));
    let disagree = node(
        lock_owner.clone(),
        vec![],
        purchase_rest(&f, &buy, TIP, &f.payment),
    );
    run(&f, &disagree).await;
    assert_eq!(
        listing(&f).state,
        ListingState::Listed,
        "mined, owner not moved"
    );

    // In the mempool while the owner has moved elsewhere: disagree too.
    let moved = node(
        info((&txid("0e"), 0)),
        vec![],
        purchase_rest(&f, &buy, -1, &f.payment),
    );
    run(&f, &moved).await;
    assert_eq!(
        listing(&f).state,
        ListingState::Listed,
        "pending, owner moved"
    );
    // The owner the purchase's TRANSFER, but hsd shows that coin in the
    // mempool: not a mined purchase.
    let unmined_owner = node(
        info((&buy, 0)),
        vec![transfer_out_of_lock(&f, &buy, &f.buyer, -1)],
        Value::Null,
    );
    run(&f, &unmined_owner).await;
    assert_eq!(
        listing(&f).state,
        ListingState::Listed,
        "owner coin unmined"
    );

    let pending = node(lock_owner, vec![], purchase_rest(&f, &buy, -1, &f.payment));
    run(&f, &pending).await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::SalePending, Some(buy.as_str()))
    );

    paid(&f, &buy, 2, TIP + 1, false);
    let mined = node(
        info((&buy, 0)),
        vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP + 1)],
        purchase_rest(&f, &buy, TIP + 1, &f.payment),
    );
    run(&f, &mined).await;
    assert_eq!(listing(&f).state, ListingState::Sold);
}

/// A node reply missing a field the verdict reads leaves the state as it
/// is, with every other fact of a purchase in place: a name reply without
/// `info`, without the owner, or without its index; and, for a listing whose
/// lock coin is unspent, a name reply without the name's height.
#[tokio::test]
async fn name_reply_without_owner_leaves_the_listing_as_it_is() {
    let buy = txid("b1");
    for state in [
        ListingState::Listed,
        ListingState::SalePending,
        ListingState::Restored,
    ] {
        let f = fx(state);
        paid(&f, &buy, 2, TIP, false);
        let full = info((&buy, 0));
        let mut no_owner = full.clone();
        no_owner["info"].as_object_mut().unwrap().remove("owner");
        let mut no_index = full.clone();
        no_index["info"]["owner"]
            .as_object_mut()
            .unwrap()
            .remove("index");
        for (case, reply) in [
            ("no info", json!({ "start": null })),
            ("no owner", no_owner),
            ("no index", no_index),
        ] {
            let chain = node(
                reply,
                vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
                purchase_rest(&f, &buy, TIP, &f.payment),
            );
            run(&f, &chain).await;
            assert_eq!(listing(&f).state, state, "{state:?}: {case}");
        }
    }
    let f = fx(ListingState::Listed);
    let mut no_height = info((&f.lock_txid, 0));
    no_height["info"].as_object_mut().unwrap().remove("height");
    run(
        &f,
        &node(no_height, vec![lock_coin(&f, TIP - 20)], Value::Null),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Listed, "no name height");
}

/// R22 and SECURITY.md (the daemon never signs or broadcasts): the listing
/// step runs in `run_sync_steps`, in `namehold-syncd`'s sync and in the
/// app's, finds the sale, and makes no send. The market half is a forward
/// guard (T6 adds market jobs): the profile's LearnHNS base URL points at a
/// server that must see no request from this step.
#[tokio::test]
async fn daemon_refresh_makes_no_send_and_no_market_call() {
    use crate::commands::sync::{run_sync_steps, SyncCaller, SyncStatus};

    for caller in [SyncCaller::Daemon, SyncCaller::App] {
        let f = fx(ListingState::Listed);
        let buy = txid("b1");
        paid(&f, &buy, 2, TIP, false);
        let mut node = mockito::Server::new_async().await;
        let mut market = mockito::Server::new_async().await;
        let path = std::env::temp_dir().join(format!(
            "namehold_sold_sync_{}_{caller:?}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let db_path = path.to_str().unwrap().to_string();
        queries::set_setting(&f.conn, "node_rpc_url", &node.url()).unwrap();
        queries::set_setting(&f.conn, "learnhns_base_url", &market.url()).unwrap();
        f.conn.execute("VACUUM INTO ?1", params![db_path]).unwrap();
        let rpc = |s: &mut mockito::ServerGuard, method: &str, result: Value| {
            s.mock("POST", "/")
                .match_body(mockito::Matcher::PartialJson(json!({ "method": method })))
                .with_header("content-type", "application/json")
                .with_body(crate::tests::shakedex_cmd_tests::rpc_ok(result))
        };
        let _chain = rpc(
            &mut node,
            "getblockchaininfo",
            json!({
                "chain": "regtest", "blocks": TIP, "headers": TIP,
                "verificationprogress": 1.0, "mediantime": 1_700_000_000
            }),
        )
        .create_async()
        .await;
        let _name = rpc(&mut node, "getnameinfo", info((&buy, 0)))
            .create_async()
            .await;
        let send = node
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("sendrawtransaction".into()))
            .expect(0)
            .create_async()
            .await;
        let _lock = node
            .mock("GET", format!("/coin/{}/0", f.lock_txid).as_str())
            .with_status(404)
            .create_async()
            .await;
        let transfer = transfer_out_of_lock(&f, &buy, &f.buyer, TIP);
        let _transfer = node
            .mock("GET", format!("/coin/{buy}/0").as_str())
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "version": 0, "height": TIP, "value": 1_000_000,
                    "address": transfer.address,
                    "covenant": { "type": COV_TRANSFER, "action": "TRANSFER",
                                  "items": transfer.covenant.as_ref().unwrap().items },
                    "coinbase": false, "hash": buy, "index": 0
                })
                .to_string(),
            )
            .create_async()
            .await;
        let no_market = market
            .mock("GET", mockito::Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let no_market_post = market
            .mock("POST", mockito::Matcher::Any)
            .expect(0)
            .create_async()
            .await;

        match caller {
            SyncCaller::Daemon => crate::daemon::sync_profile(&db_path, PROFILE).await,
            SyncCaller::App => {
                let status = std::sync::Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
                run_sync_steps(&status, &db_path, PROFILE, SyncCaller::App).await;
            }
        }

        send.assert_async().await;
        no_market.assert_async().await;
        no_market_post.assert_async().await;
        let conn = Connection::open(&path).unwrap();
        let l = queries::get_shakedex_listing(&conn, &f.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            (l.state, l.sold_txid.as_deref()),
            (ListingState::Sold, Some(buy.as_str())),
            "{caller:?}"
        );
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}

/// R22 once the buyer has finalized before our sync: the name's owner is the
/// buyer's FINALIZE elsewhere and the purchase's TRANSFER is spent, so the
/// purchase is read from our coin of it at the payment address: through
/// `GET /tx` (a node with the index), or on hsd's not-found from the block at
/// the height that coin was seen mined (no index). The lock coin is read
/// from the purchase. Without a mined height, or a block that does not hold
/// it, no verdict: a coin row is a lead, not proof.
#[tokio::test]
async fn sold_after_the_buyer_finalized_is_found_from_the_purchase() {
    let buy = txid("b1");
    let fin = txid("a9");
    let block_of = |f: &Fx, height: i64| {
        let rest = purchase_rest(f, &buy, height, &f.payment);
        let vin: Vec<_> = rest["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| json!({ "txid": i["prevout"]["hash"], "vout": i["prevout"]["index"] }))
            .collect();
        let vout: Vec<_> = rest["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(n, o)| {
                json!({ "n": n, "value": 0.0,
                    "address": { "version": 0, "hash": "00", "string": o["address"] },
                    "covenant": o["covenant"] })
            })
            .collect();
        json!({ "height": height, "tx": [ { "txid": buy, "vin": vin, "vout": vout } ] })
    };
    let finalized = info((&fin, 0));
    let buyer_coin = |f: &Fx| coin(&fin, 0, &f.buyer, COV_FINALIZE, vec![name_hash()], TIP + 12);

    // With the index.
    let f = fx(ListingState::Listed);
    paid(&f, &buy, 2, TIP, true);
    run(
        &f,
        &node(
            finalized.clone(),
            vec![buyer_coin(&f)],
            purchase_rest(&f, &buy, TIP, &f.payment),
        ),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Sold, "GET /tx");

    // Without it: the block.
    let f = fx(ListingState::Listed);
    paid(&f, &buy, 2, TIP, true);
    let chain = node(finalized.clone(), vec![buyer_coin(&f)], Value::Null)
        .with_block_hash("ab".repeat(32))
        .with_block(block_of(&f, TIP));
    run(&f, &chain).await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(buy.as_str())),
        "getblock"
    );
    assert!(chain.count_matching(|c| matches!(c, RpcCall::BlockHash(h) if *h == TIP)) > 0);

    // Our coin seen only in the mempool: nothing to look up without the index.
    let f = fx(ListingState::Listed);
    paid(&f, &buy, 2, -1, true);
    run(
        &f,
        &node(finalized.clone(), vec![buyer_coin(&f)], Value::Null),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Listed, "no mined height");

    // The block at that height no longer holds it (a reorg moved it).
    let f = fx(ListingState::Listed);
    paid(&f, &buy, 2, TIP, true);
    let chain = node(finalized.clone(), vec![buyer_coin(&f)], Value::Null)
        .with_block_hash("ab".repeat(32))
        .with_block(json!({ "height": TIP, "tx": [] }));
    run(&f, &chain).await;
    assert_eq!(listing(&f).state, ListingState::Listed, "not in that block");
}
