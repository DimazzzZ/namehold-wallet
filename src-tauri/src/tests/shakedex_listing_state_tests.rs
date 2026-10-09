//! R22 and the listing's states after the FINALIZE into the lock (T4): the
//! store's guarded writes, and the after-lock job — SalePending and Sold
//! from a purchase of the lock coin found on chain, their reorgs back,
//! Finalizing when the FINALIZE into the lock is nowhere, and Expired.

use rusqlite::{params, Connection};

use crate::db::queries::{self, ListingMode, ListingState, ShakedexListing};
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::ExtendedPrivKey;
use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::lock_key::derive_lock_key;
use crate::noncustodial::shakedex::sell::LOCK_FINALIZE_ACTION;
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
    for state in ListingState::ALL {
        let f = fx(state);
        let pending = queries::mark_listing_sale_pending(&f.conn, &f.id, &txid("b1")).unwrap();
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
    // A sale keeps the lock outpoint it has and fills one it lacks (a dead
    // FINALIZE's listing, whose outpoint was dropped).
    let f = fx(ListingState::Listed);
    queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), Some((&txid("c1"), 3))).unwrap();
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(txid("b1").as_str()))
    );
    assert_eq!(
        (l.lock_txid.as_deref(), l.lock_vout),
        (Some(txid("f1").as_str()), Some(0))
    );
    let f = fx(ListingState::ReadyToFinalize);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET lock_txid = NULL, lock_vout = NULL",
            [],
        )
        .unwrap();
    queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), Some((&txid("c1"), 3))).unwrap();
    let l = listing(&f);
    assert_eq!(
        (l.lock_txid.as_deref(), l.lock_vout),
        (Some(txid("c1").as_str()), Some(3))
    );
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
    let upgrade = |f: &Fx, vout: u32| {
        queries::upgrade_restored_lock(
            &f.conn,
            &f.id,
            &f.lock_txid,
            vout,
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
    assert_eq!(upgrade(&fx(ListingState::Listed), 0), 0, "not Restored");
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
