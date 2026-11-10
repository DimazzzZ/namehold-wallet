//! The exchange's stored rows under concurrent writers (review C2, M1).

use rusqlite::Connection;

use super::exchange_fixture as fx;
use crate::exchange_io::job::uncommitted;
use crate::exchange_io::store::{self, Trade};
use crate::noncustodial::exchange::engine::{Role, Status};
use crate::noncustodial::exchange::relay_msg;

fn conn() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&c).unwrap();
    c
}

/// A Buyer-taker's trade as `exchange_take` builds it, before any write.
fn new_trade() -> Trade {
    let f = fx::trade();
    let k = relay_msg::new_relay_key();
    let offer = crate::noncustodial::exchange::relay_msg::Offer {
        version: 1,
        network: "regtest".into(),
        side: relay_msg::Side::Sell,
        price_msat_per_hns: fx::PRICE,
        min_hns: 1,
        max_hns: u64::MAX / 2,
        maker_relay_pubkey: relay_msg::relay_pubkey_hex(&k),
        created_at: fx::START,
        expires_at: fx::START + 99,
        nonce: relay_msg::random_hex16(),
    }
    .sign(&k)
    .unwrap();
    Trade {
        profile_id: "p".into(),
        trade_id: "ab".repeat(16),
        network: "regtest".into(),
        role: Role::Buyer,
        maker: false,
        offer,
        trade_index: 0,
        relay_key: k,
        counterparty_key: None,
        hns_amount: fx::HNS_AMOUNT,
        btc_sats: 100_000,
        price_msat: fx::PRICE,
        terms: Some(f.terms.clone()),
        holdings: Some(uncommitted(Role::Buyer, &f.terms)),
        status: None,
        declined: None,
        last_error: None,
        watchtower: "none".into(),
        hns_lock: None,
        request: None,
        secret_nonce: Some(fx::NONCE),
        terminal_at: None,
        finished: false,
        version: 0,
        created_at: fx::START,
        updated_at: fx::START,
    }
}

fn trade(c: &Connection) -> Trade {
    let mut t = new_trade();
    store::insert_trade(c, &t).unwrap();
    assert!(store::update_trade(c, &mut t, fx::START).unwrap());
    store::trade(c, "p", &t.trade_id).unwrap().unwrap()
}

/// `exchange_take` stores a taker's trade with `insert_trade` alone, and a
/// Buyer's Commit needs the secret nonce chosen at the take: the insert must
/// keep it (QA 2026-11-09, Q4: the nonce was lost and every taker-Buyer
/// Commit was refused with "this trade has no secret nonce").
#[test]
fn the_insert_alone_keeps_a_buyers_secret_nonce() {
    let c = conn();
    let t = new_trade();
    store::insert_trade(&c, &t).unwrap();
    let stored = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    assert_eq!(stored.secret_nonce, Some(fx::NONCE));
}

#[test]
fn a_step_that_read_the_trade_before_a_commit_cannot_write_it_back() {
    let c = conn();
    let mut step_copy = trade(&c);
    let mut commit_copy = step_copy.clone();
    // The Commit lands while the step is busy on the network.
    commit_copy.holdings = Some(fx::trade().buyer());
    assert!(store::commit_trade(&c, &mut commit_copy, 1).unwrap());
    // The step writes its stale, uncommitted copy: refused.
    step_copy.status = Some(Status::Expired);
    assert!(!store::update_trade(&c, &mut step_copy, 2).unwrap());
    let stored = store::trade(&c, "p", &step_copy.trade_id).unwrap().unwrap();
    assert!(
        stored.holdings.unwrap().committed,
        "the presigned transactions are kept"
    );
    assert_ne!(stored.status, Some(Status::Expired));
}

#[test]
fn a_second_commit_of_the_same_trade_stores_nothing() {
    let c = conn();
    let mut a = trade(&c);
    let mut b = a.clone();
    a.holdings = Some(fx::trade().buyer());
    b.holdings = Some(fx::trade().buyer());
    assert!(store::commit_trade(&c, &mut a, 1).unwrap());
    assert!(!store::commit_trade(&c, &mut b, 1).unwrap());
    // Even a fresh read cannot commit again.
    let mut fresh = store::trade(&c, "p", &a.trade_id).unwrap().unwrap();
    assert!(!store::commit_trade(&c, &mut fresh, 2).unwrap());
}

#[test]
fn a_write_never_undoes_a_commit_and_versions_advance() {
    let c = conn();
    let mut t = trade(&c);
    t.holdings = Some(fx::trade().buyer());
    assert!(store::commit_trade(&c, &mut t, 1).unwrap());
    let v = t.version;
    // A later write with the current version but uncommitted holdings.
    t.holdings = Some(uncommitted(Role::Buyer, t.terms.as_ref().unwrap()));
    assert!(!store::update_trade(&c, &mut t, 2).unwrap());
    // A normal later write (committed holdings, current version) goes in.
    let mut t = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    assert_eq!(t.version, v);
    t.last_error = Some("x".into());
    assert!(store::update_trade(&c, &mut t, 3).unwrap());
    assert_eq!(t.version, v + 1);
}

#[test]
fn a_commit_without_committed_holdings_is_an_error() {
    let c = conn();
    let mut t = trade(&c);
    assert!(store::commit_trade(&c, &mut t, 1).is_err());
}

#[tokio::test]
async fn a_committed_buyer_announces_its_btc_lock_again_while_the_seller_waits() {
    // QA finding: one announcement at the Commit was all the Seller would
    // ever get; a relay down at that moment lost the trade.
    use super::exchange_fake_btc::FakeRelay;
    use crate::exchange_io::job::{tick, Ctx, Relay};
    use crate::noncustodial::exchange::relay_msg::Body;
    let c = conn();
    let m = fx::buyer_master();
    store::insert_account(
        &c,
        &store::Account {
            profile_id: "p".into(),
            network: "regtest".into(),
            btc_xpub: crate::noncustodial::exchange::btc_account::account_xpub(&m, fx::NET, 0)
                .unwrap(),
            swap_xpub: crate::noncustodial::exchange::keys::branch_xpub(&m, fx::NET, 0).unwrap(),
            account: 0,
            next_trade_index: 0,
            next_btc_receive: 0,
            next_btc_change: 0,
        },
        fx::START,
    )
    .unwrap();
    let mut t = trade(&c);
    t.holdings = Some(fx::trade().buyer());
    t.status = Some(Status::AwaitingSellerCommit {
        deadline: fx::START + 1200,
    });
    assert!(store::commit_trade(&c, &mut t, 1).unwrap());
    let relay = FakeRelay::default();
    let now = fx::START + 300 * 7; // the first minute of a period
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: Some(&relay as &dyn Relay),
        watchtower: None,
        now,
    };
    tick(&ctx).await;
    let sent = relay.messages.lock().unwrap().clone();
    assert!(
        sent.iter()
            .any(|e| matches!(e.message.body, Body::BtcLocked(_))),
        "{sent:?}"
    );
}

/// An offer's three slots are for trades still negotiating or awaiting the
/// Buyer's Commit. One that expired is followed for a day more, and must
/// not hold its slot for that day (QA 2026-11-10, Q5).
#[test]
fn an_ended_trade_frees_its_offers_slot_at_once() {
    use crate::exchange_io::job::holds_a_slot;
    let mut t = new_trade();
    assert!(holds_a_slot(&t), "awaiting the Buyer's Commit");
    t.holdings = None;
    assert!(holds_a_slot(&t), "negotiating");
    t.status = Some(Status::Expired);
    assert!(!t.finished);
    assert!(!holds_a_slot(&t), "expired, still followed");
    t.status = None;
    t.holdings = Some(fx::trade().buyer());
    assert!(!holds_a_slot(&t), "committed");
    t.holdings = None;
    t.finished = true;
    assert!(!holds_a_slot(&t), "finished");
}

/// The card's "Agreeing on the terms…" line follows `negotiating`: true while
/// a take waits for the maker's terms, false once that trade has ended
/// without them (QA 2026-11-10, Q6: a cancelled take still said it was
/// negotiating under "Trade expired").
#[test]
fn a_trade_that_ended_without_terms_is_not_negotiating() {
    use crate::commands::exchange::trade_row;
    let mut t = new_trade();
    t.terms = None;
    t.holdings = None;
    assert!(trade_row(&t).negotiating);
    t.status = Some(Status::Expired);
    assert!(!trade_row(&t).negotiating);
    let mut agreed = new_trade();
    agreed.status = None;
    assert!(!trade_row(&agreed).negotiating, "terms are agreed");
}
