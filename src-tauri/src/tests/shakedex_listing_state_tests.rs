//! R22 and the listing's states after the FINALIZE into the lock (T4): the
//! store's guarded writes, and the after-lock job — SalePending and Sold
//! from a purchase of the lock coin found on chain, their reorgs back,
//! Finalizing when the FINALIZE into the lock is nowhere, and Expired.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::db::queries::{self, ListingMode, ListingState, ListingWrite, ShakedexListing};
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

/// Write `w` on the fixture's listing with arguments that satisfy every
/// guard but the state one: its own lock outpoint and key, a file, a lock
/// TRANSFER, and `to` where the write takes a target. `Err` is a target the
/// write refuses.
fn apply(f: &Fx, w: ListingWrite, to: ListingState) -> Result<usize, crate::error::AppError> {
    let (c, id, b1) = (&f.conn, f.id.as_str(), txid("b1"));
    let own = (f.lock_txid.as_str(), f.lock_vout);
    match w {
        ListingWrite::Ready => queries::mark_listing_ready(c, id),
        ListingWrite::LockingAgain => queries::mark_listing_locking_again(c, id),
        ListingWrite::Finalize => {
            let tx = c.unchecked_transaction().unwrap();
            let n = queries::mark_listing_finalizing_in_tx(
                &tx,
                id,
                &queries::FinalizingListing {
                    finalize_draft_id: "fin",
                    lock_txid: &f.lock_txid,
                    lock_vout: f.lock_vout,
                    steps_json: "[]",
                    listing_file_json: "{}",
                    expires_at: 1,
                },
            );
            tx.commit().unwrap();
            n
        }
        ListingWrite::RevertToReady => queries::revert_listing_to_ready(c, id),
        ListingWrite::Listed => queries::mark_listing_listed(c, id),
        ListingWrite::FinalizingAgain => queries::mark_listing_finalizing_again(c, id),
        ListingWrite::Abort => queries::abort_shakedex_listing(c, id),
        ListingWrite::ExpireBeforeLock => queries::expire_shakedex_listing(c, id),
        ListingWrite::AdoptElsewhere => {
            queries::adopt_lock_finalized_elsewhere(c, id, own.0, own.1)
        }
        ListingWrite::Unabort => queries::unabort_shakedex_listing(c, id),
        ListingWrite::SalePending => queries::mark_listing_sale_pending(c, id, &b1, own),
        ListingWrite::Sell => queries::sell_shakedex_listing(c, id, &b1, own),
        ListingWrite::ProvenLockSale => {
            queries::sell_listing_through_proven_lock(c, id, &b1, (&txid("f2"), 1))
        }
        ListingWrite::Resell => queries::resell_sold_listing(c, id, to, &b1, own),
        ListingWrite::Unsell => queries::unsell_shakedex_listing(c, id, to),
        ListingWrite::ExpireLocked => queries::expire_locked_listing(c, id),
        ListingWrite::Unadopt => queries::unadopt_restored_lock(c, id),
        ListingWrite::Upgrade => queries::upgrade_restored_lock(
            c,
            id,
            own.0,
            own.1,
            &listing(f).lock_pubkey_hex,
            &queries::UpgradedListing {
                mode: ListingMode::BuyNow,
                payment_address: &f.payment,
                steps_json: "[]",
                listing_file_json: "{}",
                expires_at: None,
            },
        ),
    }
}

/// Every write that moves a listing's state moves exactly the (from, to)
/// pairs below, pinned as literals: the table ([`ListingWrite::transition`])
/// is checked against them, and each write is run on a listing in every
/// state, toward every state, with every other guard satisfied.
#[test]
fn each_listing_write_moves_exactly_its_transitions() {
    use ListingState as S;
    let before_lock_end = [S::Locking, S::ReadyToFinalize, S::Finalizing];
    let in_lock = [S::Finalizing, S::Listed, S::SalePending, S::Restored];
    let mut expected: Vec<(ListingWrite, S, S)> = vec![
        (ListingWrite::Ready, S::Locking, S::ReadyToFinalize),
        (ListingWrite::LockingAgain, S::ReadyToFinalize, S::Locking),
        (ListingWrite::Finalize, S::ReadyToFinalize, S::Finalizing),
        (
            ListingWrite::RevertToReady,
            S::Finalizing,
            S::ReadyToFinalize,
        ),
        (ListingWrite::Listed, S::Finalizing, S::Listed),
        (ListingWrite::FinalizingAgain, S::Listed, S::Finalizing),
        (ListingWrite::Unabort, S::Aborted, S::Locking),
        (ListingWrite::ProvenLockSale, S::Locking, S::Sold),
        (ListingWrite::ProvenLockSale, S::ReadyToFinalize, S::Sold),
        (ListingWrite::Resell, S::Sold, S::SalePending),
        (ListingWrite::Resell, S::Sold, S::Sold),
        (ListingWrite::Unadopt, S::Restored, S::Locking),
        (ListingWrite::Upgrade, S::Restored, S::Listed),
    ];
    for from in before_lock_end {
        expected.push((ListingWrite::Abort, from, S::Aborted));
        expected.push((ListingWrite::ExpireBeforeLock, from, S::Expired));
        expected.push((ListingWrite::AdoptElsewhere, from, S::Restored));
    }
    for from in in_lock {
        expected.push((ListingWrite::SalePending, from, S::SalePending));
        expected.push((ListingWrite::Sell, from, S::Sold));
    }
    for from in [S::SalePending, S::Sold] {
        for to in [S::Listed, S::Finalizing, S::Restored] {
            expected.push((ListingWrite::Unsell, from, to));
        }
    }
    for from in [S::Listed, S::SalePending, S::Restored] {
        expected.push((ListingWrite::ExpireLocked, from, S::Expired));
    }
    let mut table: Vec<(ListingWrite, S, S)> = Vec::new();
    for w in ListingWrite::ALL {
        for from in w.from() {
            for to in w.to() {
                table.push((w, *from, *to));
            }
        }
    }
    let key = |t: &(ListingWrite, S, S)| format!("{:?} {:?} {:?}", t.0, t.1, t.2);
    let mut a: Vec<String> = table.iter().map(key).collect();
    let mut b: Vec<String> = expected.iter().map(key).collect();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b, "the transition table");

    // One fixture; each case starts from its listing re-inserted in `from`.
    let f = fx(S::Listed);
    let base = listing(&f);
    for w in ListingWrite::ALL {
        let targets: Vec<S> = if w.to().len() > 1 {
            S::ALL.to_vec()
        } else {
            w.to().to_vec()
        };
        for from in S::ALL {
            for to in &targets {
                // A Finalizing listing ends before the lock only once its
                // FINALIZE draft is dead (checked below); the proven-lock
                // sale needs a listing without an outpoint.
                let proven = w == ListingWrite::ProvenLockSale;
                let row = ShakedexListing {
                    state: from,
                    lock_txid: base.lock_txid.clone().filter(|_| !proven),
                    lock_vout: base.lock_vout.filter(|_| !proven),
                    ..base.clone()
                };
                f.conn.execute("DELETE FROM shakedex_listings", []).unwrap();
                queries::insert_shakedex_listing(&f.conn, &row).unwrap();
                f.conn
                    .execute(
                        "UPDATE wallet_tx_drafts SET status = 'failed' WHERE id = 'fin'",
                        [],
                    )
                    .unwrap();
                let allowed = expected.contains(&(w, from, *to));
                let n = match apply(&f, w, *to) {
                    Ok(n) => n,
                    Err(_) => {
                        assert!(!w.to().contains(to), "{w:?} {from:?} -> {to:?}: refused");
                        0
                    }
                };
                assert_eq!(n == 1, allowed, "{w:?} {from:?} -> {to:?}");
                let state = listing(&f).state;
                assert_eq!(
                    state,
                    if allowed { *to } else { from },
                    "{w:?} {from:?} -> {to:?}"
                );
            }
        }
    }
    // Ending a listing before the lock takes a Finalizing one only while its
    // FINALIZE draft is dead.
    for w in [
        ListingWrite::Abort,
        ListingWrite::ExpireBeforeLock,
        ListingWrite::AdoptElsewhere,
    ] {
        let f = fx(S::Finalizing);
        assert_eq!(apply(&f, w, S::Aborted).unwrap(), 0, "{w:?}: live FINALIZE");
        assert_eq!(listing(&f).state, S::Finalizing);
    }
}

/// A stored lock output index that is not a `u32` is a corrupted row: the
/// after-lock job reads no coin for it and leaves the listing as it is.
#[tokio::test]
async fn a_listing_with_a_corrupted_lock_output_is_left_as_it_is() {
    let f = fx(ListingState::Finalizing);
    f.conn
        .execute("UPDATE shakedex_listings SET lock_vout = -1", [])
        .unwrap();
    let mined = lock_coin(&f, TIP - 20);
    let rpc = MockNodeRpc::new()
        .with_name_info(info((&f.lock_txid, 0)))
        .with_get_coin(move |_, _| Ok(Some(mined.clone())));
    run(&f, &rpc).await;
    assert_eq!(listing(&f).state, ListingState::Finalizing);
}

/// One txid case policy: every txid a listing write stores is lowercase,
/// however the caller spelled it, and the writes and lookups compare stored
/// txids exactly with a lowercased argument.
#[test]
fn listing_txids_are_stored_lowercase() {
    let upper = |t: &str| t.to_ascii_uppercase();
    let lc = |l: &ShakedexListing| {
        [
            l.lock_transfer_txid.clone(),
            l.lock_txid.clone(),
            l.abort_txid.clone(),
            l.sold_txid.clone(),
            l.cancel_txid.clone(),
        ]
        .into_iter()
        .flatten()
        .all(|t| t == t.to_ascii_lowercase())
    };
    // Inserted.
    let f = fx(ListingState::Listed);
    let mut row = listing(&f);
    row.id = "l2".into();
    row.name = "othername".into();
    row.lock_transfer_txid = Some(upper(&txid("a1")));
    row.lock_txid = Some(upper(&txid("a2")));
    row.abort_txid = Some(upper(&txid("a3")));
    row.sold_txid = Some(upper(&txid("a4")));
    row.cancel_txid = Some(upper(&txid("a5")));
    queries::insert_shakedex_listing(&f.conn, &row).unwrap();
    let got = queries::get_shakedex_listing(&f.conn, "l2")
        .unwrap()
        .unwrap();
    assert!(lc(&got), "{got:?}");
    assert!(
        queries::shakedex_listing_holds_lock_coin(&f.conn, PROFILE, &upper(&txid("a2")), 0)
            .unwrap()
    );
    // A sale named in upper case: found by the stored outpoint, stored lower.
    let lock = upper(&f.lock_txid);
    assert_eq!(
        queries::sell_shakedex_listing(&f.conn, &f.id, &upper(&txid("b1")), (&lock, 0)).unwrap(),
        1
    );
    assert_eq!(listing(&f).sold_txid.as_deref(), Some(txid("b1").as_str()));
    assert_eq!(
        queries::resell_sold_listing(
            &f.conn,
            &f.id,
            ListingState::Sold,
            &upper(&txid("b2")),
            (&lock, 0)
        )
        .unwrap(),
        1
    );
    assert_eq!(listing(&f).sold_txid.as_deref(), Some(txid("b2").as_str()));
    // Adopted, and sold through a proven lock.
    let f = fx(ListingState::ReadyToFinalize);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET lock_txid = NULL, lock_vout = NULL",
            [],
        )
        .unwrap();
    assert_eq!(
        queries::adopt_lock_finalized_elsewhere(&f.conn, &f.id, &upper(&txid("c1")), 0).unwrap(),
        1
    );
    assert_eq!(listing(&f).lock_txid.as_deref(), Some(txid("c1").as_str()));
    let f = fx(ListingState::Locking);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET lock_txid = NULL, lock_vout = NULL",
            [],
        )
        .unwrap();
    let proven = upper(&txid("c2"));
    queries::sell_listing_through_proven_lock(&f.conn, &f.id, &upper(&txid("b3")), (&proven, 1))
        .unwrap();
    assert!(lc(&listing(&f)), "{:?}", listing(&f));
    // A Cancel transfer linked by its lock TRANSFER named in upper case.
    let f = fx(ListingState::Locking);
    let n = queries::link_shakedex_listing_abort(
        &f.conn,
        PROFILE,
        NAME,
        &upper(&f.transfer_txid),
        0,
        "fin",
        &upper(&txid("d1")),
    )
    .unwrap();
    assert_eq!(n, 1);
    assert_eq!(listing(&f).abort_txid.as_deref(), Some(txid("d1").as_str()));
    // Finalize & sign's lock outpoint, and a file upgrading a Restored lock
    // named in upper case.
    let f = fx(ListingState::ReadyToFinalize);
    let tx = f.conn.unchecked_transaction().unwrap();
    let fin = queries::FinalizingListing {
        finalize_draft_id: "fin",
        lock_txid: &upper(&txid("e2")),
        lock_vout: 0,
        steps_json: "[]",
        listing_file_json: "{}",
        expires_at: 1,
    };
    assert_eq!(
        queries::mark_listing_finalizing_in_tx(&tx, &f.id, &fin).unwrap(),
        1
    );
    tx.commit().unwrap();
    assert_eq!(listing(&f).lock_txid.as_deref(), Some(txid("e2").as_str()));
    let f = fx(ListingState::Restored);
    let n = queries::upgrade_restored_lock(
        &f.conn,
        &f.id,
        &upper(&f.lock_txid),
        f.lock_vout,
        &listing(&f).lock_pubkey_hex,
        &queries::UpgradedListing {
            mode: ListingMode::BuyNow,
            payment_address: &f.payment,
            steps_json: "[]",
            listing_file_json: "{}",
            expires_at: None,
        },
    )
    .unwrap();
    assert_eq!(n, 1);
}

/// The guards of the sale, resell, unadopt and upgrade writes beside the
/// state: the listing's own lock outpoint, its lock TRANSFER, its key.
#[test]
fn sale_writes_move_only_a_listing_in_our_lock() {
    // Sold is moved only to SalePending or Sold, and only for its own lock
    // outpoint.
    let f = fx(ListingState::Sold);
    let b2 = txid("b2");
    assert!(queries::resell_sold_listing(
        &f.conn,
        &f.id,
        ListingState::Listed,
        &b2,
        (&f.lock_txid, 0)
    )
    .is_err());
    for (to, other) in [
        (ListingState::SalePending, (txid("c1"), 0)),
        (ListingState::Sold, (txid("c1"), 0)),
        (ListingState::Sold, (txid("f1"), 1)),
    ] {
        assert_eq!(
            queries::resell_sold_listing(&f.conn, &f.id, to, &b2, (&other.0, other.1)).unwrap(),
            0,
            "{to:?} {other:?}"
        );
    }
    assert_eq!(
        (listing(&f).state, listing(&f).sold_txid),
        (ListingState::Sold, None)
    );
    // A sale of the outpoint the listing has is written; a purchase of
    // another outpoint is another lock's and changes nothing.
    let f = fx(ListingState::Listed);
    let other = (txid("c1"), 3);
    let o = (other.0.as_str(), other.1);
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
    let vout = (f.lock_txid.as_str(), 1);
    assert_eq!(
        queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), vout).unwrap(),
        0
    );
    let same = (f.lock_txid.as_str(), 0);
    assert_eq!(
        queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), same).unwrap(),
        1
    );
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(txid("b1").as_str()))
    );
    // A listing without a stored outpoint is never sold or pending through
    // these writes, whatever outpoint the purchase names: only
    // `sell_listing_through_proven_lock` gives a listing an outpoint on sale.
    for pending in [false, true] {
        let f = fx(ListingState::Listed);
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
        assert_eq!(
            write(o),
            0,
            "an outpoint only the purchase names (pending {pending})"
        );
        let l = listing(&f);
        assert_eq!(
            (l.state, l.lock_txid, l.lock_vout),
            (ListingState::Listed, None, None),
            "pending {pending}"
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

/// The Sold write through a lock coin the job proved to be the listing's
/// (`shakedex_jobs::finalize_into_lock`): only from Locking or
/// ReadyToFinalize (pinned as a literal; the stored-outpoint sale keeps
/// Locking out),
/// only while the listing has no lock outpoint, and the proven coin becomes
/// it.
#[test]
fn proven_lock_sale_moves_only_a_listing_before_the_lock_without_an_outpoint() {
    let mut set: Vec<&str> = ListingWrite::ProvenLockSale
        .from()
        .iter()
        .map(|s| s.as_str())
        .collect();
    set.sort_unstable();
    assert_eq!(set, ["locking", "ready_to_finalize"]);
    assert!(!ListingWrite::Sell.from().contains(&ListingState::Locking));
    let proven = (txid("f2"), 1);
    for state in ListingState::ALL {
        let f = fx(state);
        // With its own outpoint: never.
        let n = queries::sell_listing_through_proven_lock(
            &f.conn,
            &f.id,
            &txid("b1"),
            (&proven.0, proven.1),
        )
        .unwrap();
        assert_eq!(n, 0, "{state:?}: with an outpoint");
        f.conn
            .execute(
                "UPDATE shakedex_listings SET lock_txid = NULL, lock_vout = NULL WHERE id = ?1",
                [&f.id],
            )
            .unwrap();
        let n = queries::sell_listing_through_proven_lock(
            &f.conn,
            &f.id,
            &txid("b1"),
            (&proven.0, proven.1),
        )
        .unwrap();
        let moves = ListingWrite::ProvenLockSale.from().contains(&state);
        assert_eq!(n == 1, moves, "{state:?}: without an outpoint");
        let l = listing(&f);
        if moves {
            assert_eq!(l.state, ListingState::Sold);
            assert_eq!(l.sold_txid.as_deref(), Some(txid("b1").as_str()));
            assert_eq!(
                (l.lock_txid.as_deref(), l.lock_vout),
                (Some(proven.0.as_str()), Some(1))
            );
        } else {
            assert_eq!((l.state, l.lock_txid), (state, None), "{state:?}");
        }
    }
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

/// A lock restored by name (R32): no payment address, no file, no lock
/// TRANSFER of this device.
fn restored_by_name(f: &Fx) {
    f.conn
        .execute(
            "UPDATE shakedex_listings SET lock_transfer_txid = NULL, payment_address = NULL,
                 lock_finalize_draft_id = NULL, listing_file_json = NULL, steps_json = '[]'",
            [],
        )
        .unwrap();
}

/// R22 for a lock restored by name, which knows no payment address: a
/// TRANSFER out of our lock coin needs the lock key's signature, so one
/// committing to an address not ours can only be a price step (`0x84`),
/// which commits to its payment output. Mined, and linked from the stored
/// lock outpoint (input k the lock coin, output k the TRANSFER of the name
/// at our lock), it is Sold with that txid, found through the owner without
/// the transaction index (the block at the owner coin's height). Committing
/// to an address of ours (our cancel: T5), in the mempool, linked from
/// another coin, or not readable: no verdict.
#[tokio::test]
async fn restored_lock_by_name_is_sold_by_a_mined_transfer_out_of_its_lock() {
    let buy = txid("b1");
    let stranger = address::encode_p2wpkh(NET, &[8; 20]).unwrap();
    // Mined, read with `GET /tx`, and again from the block (no index).
    for indexed in [true, false] {
        let f = fx(ListingState::Restored);
        restored_by_name(&f);
        let rest = purchase_rest(&f, &buy, TIP, &stranger);
        let tx = if indexed { rest.clone() } else { Value::Null };
        let rpc = node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            tx,
        )
        .with_block_hash("bb".repeat(32))
        .with_block(block_with(&rest, TIP));
        run(&f, &rpc).await;
        let l = listing(&f);
        assert_eq!(
            (l.state, l.sold_txid.as_deref(), l.lock_txid.as_deref()),
            (
                ListingState::Sold,
                Some(buy.as_str()),
                Some(f.lock_txid.as_str())
            ),
            "indexed {indexed}"
        );
    }
    // Committing to an address of ours: a cancel, T5's.
    let f = fx(ListingState::Restored);
    restored_by_name(&f);
    let mut rest = purchase_rest(&f, &buy, TIP, &stranger);
    let (_, ours) = address::decode(NET, &f.cancel).unwrap();
    rest["outputs"][0]["covenant"]["items"][3] = hex::encode(ours).into();
    run(
        &f,
        &node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.cancel, TIP)],
            rest,
        ),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Restored, "our cancel");
    // In the mempool: the owner is still the lock coin.
    let f = fx(ListingState::Restored);
    restored_by_name(&f);
    run(
        &f,
        &node(
            info((&f.lock_txid, f.lock_vout)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, -1)],
            purchase_rest(&f, &buy, -1, &stranger),
        ),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Restored, "mempool");
    // Linked from another coin: input 0 is not our lock coin.
    let f = fx(ListingState::Restored);
    restored_by_name(&f);
    let mut rest = purchase_rest(&f, &buy, TIP, &stranger);
    rest["inputs"][0]["prevout"]["hash"] = txid("c1").into();
    run(
        &f,
        &node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            rest,
        ),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Restored, "another coin");
    // The TRANSFER's transaction not found anywhere: no verdict.
    let f = fx(ListingState::Restored);
    restored_by_name(&f);
    run(
        &f,
        &node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            Value::Null,
        )
        .with_block_hash("bb".repeat(32))
        .with_block(json!({ "height": TIP, "tx": [] })),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Restored, "not found");
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
        // No transaction index: the owner's transaction is read from the
        // block at the owner coin's height.
        let chain = node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            Value::Null,
        )
        .with_block_hash("ab".repeat(32))
        .with_block(block_with(&purchase_rest(&f, &buy, TIP, &f.payment), TIP));
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
    // (e) the owner a mined TRANSFER at our lock committing to the buyer, in
    // a transaction that pays our payment address, but spending another lock
    // coin of the name (same seed, same lock address: a re-listing elsewhere).
    // The owner's transaction is read from its block and judged against our
    // lock outpoint: not this listing's sale.
    let f = fx(ListingState::Listed);
    let x = txid("e8");
    paid(&f, &x, 2, TIP, false);
    let mut relisted = purchase_rest(&f, &x, TIP, &f.payment);
    relisted["inputs"][0]["prevout"] = json!({ "hash": txid("c2"), "index": 0 });
    let chain = node(
        info((&x, 0)),
        vec![transfer_out_of_lock(&f, &x, &f.buyer, TIP)],
        Value::Null,
    )
    .with_block_hash("ab".repeat(32))
    .with_block(block_with(&relisted, TIP));
    run(&f, &chain).await;
    assert_eq!(
        listing(&f).state,
        ListingState::Listed,
        "another lock coin bought"
    );
    assert!(
        chain.count_matching(|c| matches!(c, RpcCall::BlockHash(h) if *h == TIP)) > 0,
        "the owner's transaction was read from its block"
    );
}

/// hsd's `getblock <hash> true true` holding the one transaction `rest`
/// (hsd's `GET /tx` shape), at `height`.
fn block_with(rest: &Value, height: i64) -> Value {
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
    json!({ "height": height, "tx": [ { "txid": rest["hash"], "vin": vin, "vout": vout } ] })
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
        // The lock TRANSFER the FINALIZE into the lock spent: hsd's 404.
        let _lock_transfer = node
            .mock("GET", format!("/coin/{}/0", f.transfer_txid).as_str())
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
        // No transaction index: hsd's empty 404 for the purchase, and its
        // block at the owner coin's height.
        let _no_tx = node
            .mock("GET", format!("/tx/{buy}").as_str())
            .with_status(404)
            .create_async()
            .await;
        let block_hash = "ab".repeat(32);
        let _hash = rpc(&mut node, "getblockhash", json!(block_hash))
            .create_async()
            .await;
        let _block = rpc(
            &mut node,
            "getblock",
            block_with(&purchase_rest(&f, &buy, TIP, &f.payment), TIP),
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
    let block_of =
        |f: &Fx, height: i64| block_with(&purchase_rest(f, &buy, height, &f.payment), height);
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

/// A transaction that cannot be read does not hold the listing for ever:
/// the owner's transaction and an earlier lead whose block entry is not
/// hsd's whole answer are skipped (skipping only withholds a verdict), and
/// the purchase found from the next lead decides.
#[tokio::test]
async fn an_unreadable_transaction_is_skipped() {
    let f = fx(ListingState::Listed);
    let bad = txid("a0");
    let buy = txid("b1");
    paid(&f, &bad, 2, TIP, false);
    paid(&f, &buy, 2, TIP, false);
    let mut block = block_with(&purchase_rest(&f, &buy, TIP, &f.payment), TIP);
    let good = block["tx"][0].clone();
    block["tx"] = json!([{ "txid": bad, "vin": [] }, good]);
    let chain = node(
        info((&bad, 0)),
        vec![transfer_out_of_lock(&f, &bad, &f.buyer, TIP)],
        Value::Null,
    )
    .with_block_hash("ab".repeat(32))
    .with_block(block);
    run(&f, &chain).await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(buy.as_str()))
    );
}

/// Review Focus 3: a reorg takes the FINALIZE into the lock away after the
/// steps are signed. Back in the mempool (the lock coin at -1) the listing is
/// Finalizing; in no block and no mempool (hsd's 404 for the lock coin, and
/// the lock TRANSFER it spent a coin again) Finalizing too; mined again,
/// Listed. A coin at the payment address from an unrelated transaction never
/// makes it SalePending or Sold meanwhile; a Sold listing whose FINALIZE is
/// taken away goes back to Finalizing as well.
#[tokio::test]
async fn listing_follows_a_reorg_of_its_lock_finalize() {
    let f = fx(ListingState::Listed);
    let other = txid("d1");
    paid(&f, &other, 0, TIP, false);
    let gift = json!({ "hash": other, "height": TIP, "hex": "00",
        "inputs": [ { "prevout": { "hash": "aa".repeat(32), "index": 0 } } ],
        "outputs": [ { "value": PRICE, "address": f.payment,
                       "covenant": { "type": 0, "action": "NONE", "items": [] } } ] });
    let transfer_back = coin(
        &f.transfer_txid,
        0,
        &f.payment,
        COV_TRANSFER,
        vec![],
        TIP - 40,
    );
    let lock_owner = info((&f.lock_txid, 0));
    let transfer_owner = info((&f.transfer_txid, 0));
    for (case, reply, coins, want) in [
        (
            "in the mempool",
            lock_owner.clone(),
            vec![lock_coin(&f, -1)],
            ListingState::Finalizing,
        ),
        (
            "mined",
            lock_owner.clone(),
            vec![lock_coin(&f, TIP)],
            ListingState::Listed,
        ),
        (
            "nowhere",
            transfer_owner.clone(),
            vec![transfer_back.clone()],
            ListingState::Finalizing,
        ),
        (
            "still nowhere",
            transfer_owner,
            vec![transfer_back.clone()],
            ListingState::Finalizing,
        ),
        (
            "mined again",
            lock_owner,
            vec![lock_coin(&f, TIP + 1)],
            ListingState::Listed,
        ),
    ] {
        run(&f, &node(reply, coins, gift.clone())).await;
        let l = listing(&f);
        assert_eq!(l.state, want, "{case}");
        assert_eq!(l.sold_txid, None, "{case}");
    }
    // Sold, and the FINALIZE into the lock reorged out with the purchase.
    set_state(&f, ListingState::Sold);
    run(
        &f,
        &node(info((&f.transfer_txid, 0)), vec![transfer_back], gift),
    )
    .await;
    assert_eq!(
        listing(&f).state,
        ListingState::Finalizing,
        "sold, FINALIZE nowhere"
    );
}

/// R22, a reorg of the purchase. Its block taken away and the purchase in no
/// mempool: hsd shows the lock coin as a coin again (it answers 404 for a
/// coin any mempool transaction spends) → Listed, the purchase forgotten.
/// Back in the mempool → SalePending; mined again → Sold. A Sold listing
/// whose name has another open listing by now stays Sold.
#[tokio::test]
async fn sold_reverts_on_a_reorg_of_the_purchase() {
    let f = fx(ListingState::Listed);
    let buy = txid("b1");
    paid(&f, &buy, 2, TIP, false);
    let sold = node(
        info((&buy, 0)),
        vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
        purchase_rest(&f, &buy, TIP, &f.payment),
    );
    run(&f, &sold).await;
    assert_eq!(listing(&f).state, ListingState::Sold);

    run(
        &f,
        &node(
            info((&f.lock_txid, 0)),
            vec![lock_coin(&f, TIP - 20)],
            Value::Null,
        ),
    )
    .await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid),
        (ListingState::Listed, None),
        "purchase nowhere"
    );

    paid(&f, &buy, 2, -1, false);
    run(
        &f,
        &node(
            info((&f.lock_txid, 0)),
            vec![],
            purchase_rest(&f, &buy, -1, &f.payment),
        ),
    )
    .await;
    assert_eq!(
        listing(&f).state,
        ListingState::SalePending,
        "back in the mempool"
    );

    paid(&f, &buy, 2, TIP + 2, false);
    let again = node(
        info((&buy, 0)),
        vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP + 2)],
        purchase_rest(&f, &buy, TIP + 2, &f.payment),
    );
    run(&f, &again).await;
    assert_eq!(listing(&f).state, ListingState::Sold, "mined again");

    // A newer listing of the name is open: the old sale stays Sold.
    let mut newer = listing(&f);
    newer.id = "l2".into();
    newer.state = ListingState::Locking;
    newer.lock_txid = None;
    newer.lock_vout = None;
    queries::insert_shakedex_listing(&f.conn, &newer).unwrap();
    run(
        &f,
        &node(
            info((&f.lock_txid, 0)),
            vec![lock_coin(&f, TIP - 20)],
            Value::Null,
        ),
    )
    .await;
    assert_eq!(
        listing(&f).state,
        ListingState::Sold,
        "another listing is open"
    );
}

/// R22, R31: a name that expired while locked ends the listing as Expired,
/// never Sold, whatever arrived at the payment address: hsd reports no live
/// state (`info: null`), or the name was opened again (hsd's name height is
/// not the lock coin's). From Listed and from a Restored lock, with the lock
/// coin unspent; and from Listed with the lock coin spent.
#[tokio::test]
async fn expired_lock_is_expired_not_sold() {
    let other = txid("d1");
    let mut reopened = info((&"00".repeat(32), u32::MAX));
    reopened["info"]["height"] = 7_000.into();
    for state in [ListingState::Listed, ListingState::Restored] {
        for (case, reply) in [
            ("info null", json!({ "info": null, "start": null })),
            ("reopened", reopened.clone()),
        ] {
            let f = fx(state);
            paid(&f, &other, 0, TIP, false);
            run(&f, &node(reply, vec![lock_coin(&f, TIP - 20)], Value::Null)).await;
            let l = listing(&f);
            assert_eq!(
                (l.state, l.sold_txid),
                (ListingState::Expired, None),
                "{state:?} {case}"
            );
            assert_eq!(
                l.lock_txid.as_deref(),
                Some(f.lock_txid.as_str()),
                "{state:?} {case}"
            );
        }
    }
    let f = fx(ListingState::Listed);
    let buy = txid("b1");
    paid(&f, &buy, 2, TIP, false);
    run(
        &f,
        &node(
            json!({ "info": null, "start": null }),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            purchase_rest(&f, &buy, TIP, &f.payment),
        ),
    )
    .await;
    assert_eq!(
        listing(&f).state,
        ListingState::Expired,
        "lock coin spent, name expired"
    );
    // A purchase pending when the name expired under it: SalePending is
    // Expired too, keeping its lock outpoint (R22, `ListingWrite::ExpireLocked`).
    let f = fx(ListingState::SalePending);
    f.conn
        .execute("UPDATE shakedex_listings SET sold_txid = ?1", [&buy])
        .unwrap();
    paid(&f, &buy, 2, -1, false);
    run(
        &f,
        &node(
            json!({ "info": null, "start": null }),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, -1)],
            purchase_rest(&f, &buy, -1, &f.payment),
        ),
    )
    .await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.lock_txid.as_deref(), l.lock_vout),
        (ListingState::Expired, Some(f.lock_txid.as_str()), Some(0)),
        "sale pending, name expired"
    );
    // A live name whose height is the lock coin's: Listed as before.
    let f = fx(ListingState::Listed);
    run(
        &f,
        &node(
            info((&f.lock_txid, 0)),
            vec![lock_coin(&f, TIP - 20)],
            Value::Null,
        ),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Listed);
}

/// The Restored lock's follower (T3 carry): a reorg that takes away the
/// FINALIZE it was adopted from (the lock TRANSFER a coin again) makes it
/// Locking without the outpoint, for the before-lock job (a lock restored by
/// name, bought: `restored_lock_by_name_is_sold_by_a_mined_transfer_out_of_its_lock`).
#[tokio::test]
async fn restored_lock_follows_its_coin() {
    let f = fx(ListingState::Restored);
    let back = coin(
        &f.transfer_txid,
        0,
        &f.payment,
        COV_TRANSFER,
        vec![],
        TIP - 40,
    );
    run(
        &f,
        &node(info((&f.transfer_txid, 0)), vec![back], Value::Null),
    )
    .await;
    let l = listing(&f);
    assert_eq!((l.state, l.lock_txid), (ListingState::Locking, None));

    // A lock restored by name (no payment address) and bought:
    // `restored_lock_by_name_is_sold_by_a_mined_transfer_out_of_its_lock`.
    let buy = txid("b1");

    // A Restored lock adopted from another device's FINALIZE keeps this
    // device's payment address but has no listing file: Sold from a purchase
    // paying it, and back to Restored (not Listed) when a reorg makes the
    // lock coin a coin again.
    let f = fx(ListingState::Restored);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET lock_finalize_draft_id = NULL, listing_file_json = NULL",
            [],
        )
        .unwrap();
    paid(&f, &buy, 2, TIP, false);
    run(
        &f,
        &node(
            info((&buy, 0)),
            vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
            purchase_rest(&f, &buy, TIP, &f.payment),
        ),
    )
    .await;
    assert_eq!(listing(&f).state, ListingState::Sold, "adopted lock bought");
    run(
        &f,
        &node(
            info((&f.lock_txid, 0)),
            vec![lock_coin(&f, TIP - 20)],
            Value::Null,
        ),
    )
    .await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid),
        (ListingState::Restored, None),
        "adopted lock, purchase nowhere"
    );

    // Sold from that adopted lock, and a reorg takes both the FINALIZE it
    // was adopted from and the purchase away: the lock TRANSFER is a coin
    // again → Restored, the purchase forgotten (the next sync takes it to
    // Locking).
    let f = fx(ListingState::Restored);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1,
                 lock_finalize_draft_id = NULL, listing_file_json = NULL",
            [&buy],
        )
        .unwrap();
    let back = coin(
        &f.transfer_txid,
        0,
        &f.payment,
        COV_TRANSFER,
        vec![],
        TIP - 40,
    );
    run(
        &f,
        &node(info((&f.transfer_txid, 0)), vec![back], Value::Null),
    )
    .await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid),
        (ListingState::Restored, None),
        "adopted lock, FINALIZE and purchase nowhere"
    );
}

/// R22, a real reorg of the purchase's block: hsd puts the purchase back in
/// its mempool (`mempool._removeBlock`), so the lock coin is hsd's 404, the
/// name's owner is the lock coin again and the purchase is at -1: a Sold
/// listing is SalePending again — unless another listing of the name is open
/// by now. A different purchase of the same lock coin mined instead replaces
/// the sale's txid. Its FINALIZE taken back to the mempool with the purchase
/// gone, a Sold listing is Finalizing at once (its file is not exported over
/// an unmined FINALIZE).
#[tokio::test]
async fn sold_follows_its_purchase_back_to_the_mempool_and_a_competing_purchase() {
    let buy = txid("b1");
    let sold = |f: &Fx| {
        f.conn
            .execute(
                "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
                [&buy],
            )
            .unwrap();
    };
    let f = fx(ListingState::Listed);
    sold(&f);
    paid(&f, &buy, 2, -1, false);
    let back = node(
        info((&f.lock_txid, 0)),
        vec![],
        purchase_rest(&f, &buy, -1, &f.payment),
    );
    run(&f, &back).await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::SalePending, Some(buy.as_str())),
        "purchase back in the mempool"
    );

    // Another listing of the name open by now: stays Sold.
    let f = fx(ListingState::Listed);
    sold(&f);
    paid(&f, &buy, 2, -1, false);
    let mut newer = listing(&f);
    newer.id = "l2".into();
    newer.state = ListingState::Locking;
    newer.lock_txid = None;
    newer.lock_vout = None;
    newer.lock_transfer_txid = Some(txid("e2"));
    queries::insert_shakedex_listing(&f.conn, &newer).unwrap();
    // The write itself refuses it (0 rows), before the one-open-listing
    // index would turn it into an error.
    let pending = ListingState::SalePending;
    let lock = (f.lock_txid.as_str(), 0);
    assert_eq!(
        queries::resell_sold_listing(&f.conn, &f.id, pending, &buy, lock).unwrap(),
        0
    );
    run(&f, &back).await;
    assert_eq!(
        listing(&f).state,
        ListingState::Sold,
        "another listing is open"
    );

    // A competing purchase of the same lock coin mined instead.
    let f = fx(ListingState::Listed);
    sold(&f);
    let rival = txid("c7");
    paid(&f, &rival, 2, TIP + 3, false);
    let won = node(
        info((&rival, 0)),
        vec![transfer_out_of_lock(&f, &rival, &f.buyer, TIP + 3)],
        purchase_rest(&f, &rival, TIP + 3, &f.payment),
    );
    run(&f, &won).await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(rival.as_str())),
        "a competing purchase mined"
    );

    // The FINALIZE back in the mempool, the purchase gone.
    let f = fx(ListingState::Listed);
    sold(&f);
    run(
        &f,
        &node(
            info((&f.lock_txid, 0)),
            vec![lock_coin(&f, -1)],
            Value::Null,
        ),
    )
    .await;
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid),
        (ListingState::Finalizing, None),
        "FINALIZE in the mempool"
    );
}

/// A Sold listing whose purchase is still the one hsd shows is not written
/// again: each sync leaves `updated_at` as it was, so the listing leaves
/// the after-lock set once [`crate::shakedex_jobs::SOLD_RECHECK_DAYS`]
/// pass, instead of being re-checked for ever.
#[tokio::test]
async fn a_stable_sold_listing_is_not_written_again() {
    let f = fx(ListingState::Sold);
    let buy = txid("b1");
    paid(&f, &buy, 2, TIP, false);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET sold_txid = ?1,
                 updated_at = datetime('now', '-6 days', '-1 hours') WHERE id = ?2",
            params![buy, f.id],
        )
        .unwrap();
    let before = listing(&f).updated_at;
    let chain = node(
        info((&buy, 0)),
        vec![transfer_out_of_lock(&f, &buy, &f.buyer, TIP)],
        purchase_rest(&f, &buy, TIP, &f.payment),
    );
    for _ in 0..2 {
        run(&f, &chain).await;
    }
    assert!(
        chain.count_matching(|c| matches!(c, RpcCall::TxByHash(t) if *t == buy)) > 0,
        "the purchase was read, so the job reached the sale"
    );
    let l = listing(&f);
    assert_eq!(
        (l.state, l.sold_txid.as_deref()),
        (ListingState::Sold, Some(buy.as_str()))
    );
    assert_eq!(l.updated_at, before, "not written again");
    // A day later the window is over.
    let window = crate::shakedex_jobs::SOLD_RECHECK_DAYS - 1;
    assert!(
        queries::list_shakedex_listings_after_lock(&f.conn, PROFILE, window)
            .unwrap()
            .is_empty()
    );
}
