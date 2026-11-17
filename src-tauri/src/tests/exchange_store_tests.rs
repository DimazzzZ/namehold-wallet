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
    use crate::exchange_io::job::{tick, Ctx};
    use crate::exchange_io::meeting::MeetingPoint;
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
    // Not the first minute of a 300-second period: the first announcement
    // of a process does not wait for one (QA Q8).
    let now = fx::START + 300 * 7 + 150;
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: Some(&relay as &dyn MeetingPoint),
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

/// The same step over Nostr (spec 2026-11-12): the announcement is an event
/// of the trade's relay key, addressed to the Seller's key and readable only
/// by the two of them.
#[tokio::test]
async fn a_committed_buyer_announces_its_btc_lock_over_nostr_to_the_seller_only() {
    use super::exchange_fake_nostr::{FakeRelay, Mode};
    use crate::exchange_io::job::{tick, Ctx};
    use crate::exchange_io::meeting::MeetingPoint;
    use crate::exchange_io::nostr::client::NostrClient;
    use crate::exchange_io::nostr::transport::{NostrMeetingPoint, MESSAGE_KIND};
    use crate::exchange_io::nostr::{event, nip44};
    use crate::noncustodial::exchange::relay_msg::{self, Body, Envelope};
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
    // Its own trade id: the job remembers announcements per (profile,
    // trade) for the whole process, and the test above announces "ab…".
    let mut t = new_trade();
    t.trade_id = "ce".repeat(16);
    store::insert_trade(&c, &t).unwrap();
    assert!(store::update_trade(&c, &mut t, fx::START).unwrap());
    let mut t = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    t.holdings = Some(fx::trade().buyer());
    t.status = Some(Status::AwaitingSellerCommit {
        deadline: fx::START + 1200,
    });
    assert!(store::commit_trade(&c, &mut t, 1).unwrap());
    let nostr = FakeRelay::start(Mode::Accept).await;
    fn clock() -> u64 {
        fx::START + 300 * 7 + 150
    }
    let point = NostrMeetingPoint::with_clock(NostrClient::new(vec![nostr.url.clone()]), clock);
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: Some(&point as &dyn MeetingPoint),
        watchtower: None,
        now: clock(),
    };
    tick(&ctx).await;
    let sent: Vec<_> = nostr
        .events()
        .into_iter()
        .filter(|e| e.kind == MESSAGE_KIND)
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let e = &sent[0];
    let seller = t.terms.as_ref().unwrap().seller_relay_pubkey.clone();
    let seller_x = event::x_only_of(&seller).unwrap();
    assert_eq!(e.tag("p"), Some(seller_x.as_str()));
    assert_eq!(
        Some(e.pubkey.clone()),
        event::x_only_of(&relay_msg::relay_pubkey_hex(&t.relay_key))
    );
    // NIP-44's key is shared: the Buyer's side opens what the Seller will.
    let ck = nip44::conversation_key(
        &t.relay_key,
        &hex::decode(&seller_x).unwrap().try_into().unwrap(),
    )
    .unwrap();
    let env: Envelope = serde_json::from_str(&nip44::decrypt(&e.content, &ck).unwrap()).unwrap();
    assert!(env.verify());
    assert_eq!(env.message.to, seller);
    assert!(matches!(env.message.body, Body::BtcLocked(_)));
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

/// QA 2026-11-10, Q7: with the Bitcoin source down a refunded trade read
/// "can't check" (and its coins as locked) until the source came back.
#[test]
fn an_ended_trade_keeps_its_status_through_a_failed_read() {
    use crate::exchange_io::job::settled_status;
    use crate::noncustodial::exchange::engine::Chain;
    let refunded = Status::Refunded { chain: Chain::Btc };
    let cannot = Status::CannotCheck { chain: Chain::Btc };
    // A failed read never undoes an ending.
    assert_eq!(
        settled_status(Some(&refunded), cannot.clone(), true),
        refunded
    );
    assert_eq!(
        settled_status(Some(&Status::Completed), cannot.clone(), true),
        Status::Completed
    );
    // A clean read decides: a reorg may reopen the trade.
    assert_eq!(
        settled_status(Some(&refunded), cannot.clone(), false),
        cannot
    );
    // An open trade does report that it cannot check.
    let open = Status::AwaitingBtcLock {
        confirmations: 0,
        needed: 1,
    };
    assert_eq!(settled_status(Some(&open), cannot.clone(), true), cannot);
    assert_eq!(settled_status(None, cannot.clone(), true), cannot);
    // One ending may replace another even on a failed read.
    assert_eq!(
        settled_status(Some(&Status::Expired), refunded.clone(), true),
        refunded
    );
}

/// QA 2026-11-11, Q8: the announcement no longer depends on a step landing
/// in one minute of each period.
#[test]
fn a_lock_is_announced_at_once_and_then_once_a_period() {
    use crate::exchange_io::job::{announce_due, REANNOUNCE_EVERY_SECS};
    assert!(announce_due(None, 12_345), "never announced: now");
    let at = 10_000;
    assert!(!announce_due(Some(at), at));
    assert!(!announce_due(Some(at), at + 70), "the next step");
    assert!(!announce_due(Some(at), at + REANNOUNCE_EVERY_SECS - 1));
    assert!(announce_due(Some(at), at + REANNOUNCE_EVERY_SECS));
    assert!(announce_due(Some(at), at + REANNOUNCE_EVERY_SECS + 130));
    // A clock that went back never blocks the announcement for ever.
    assert!(!announce_due(Some(at), at - 50));
    assert!(announce_due(Some(u64::MAX), u64::MAX));
}

/// A meeting point whose every call takes `delay` and then succeeds with
/// nothing new, as six relays of which one never answers (the client waits
/// for each relay's timeout); it counts the calls and the inbox keys asked
/// for, in order. With `delay: None` a call never ends.
struct SlowPoint {
    delay: Option<std::time::Duration>,
    calls: std::sync::Mutex<usize>,
    inboxes: std::sync::Mutex<Vec<String>>,
}

impl SlowPoint {
    fn new(delay: Option<std::time::Duration>) -> SlowPoint {
        SlowPoint {
            delay,
            calls: std::sync::Mutex::new(0),
            inboxes: std::sync::Mutex::new(Vec::new()),
        }
    }
    async fn wait(&self) -> Result<(), crate::error::AppError> {
        *self.calls.lock().unwrap() += 1;
        match self.delay {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending::<()>().await,
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::exchange_io::meeting::MeetingPoint for SlowPoint {
    async fn publish_offer(
        &self,
        _o: &relay_msg::SignedOffer,
        _key: &secp256k1::SecretKey,
    ) -> Result<(), crate::error::AppError> {
        self.wait().await
    }
    async fn withdraw_offer(
        &self,
        _o: &relay_msg::SignedOffer,
        _signature: &str,
        _key: &secp256k1::SecretKey,
    ) -> Result<(), crate::error::AppError> {
        self.wait().await
    }
    async fn offers(
        &self,
        _network: &str,
        _now: u64,
    ) -> Result<(Vec<relay_msg::SignedOffer>, usize), crate::error::AppError> {
        self.wait().await.map(|_| (Vec::new(), 0))
    }
    async fn send(
        &self,
        _e: &relay_msg::Envelope,
        _key: &secp256k1::SecretKey,
    ) -> Result<(), crate::error::AppError> {
        self.wait().await
    }
    async fn inbox(
        &self,
        key: &secp256k1::SecretKey,
        _network: &str,
        _cursor: Option<&str>,
    ) -> Result<(Vec<relay_msg::Envelope>, Option<String>), crate::error::AppError> {
        self.inboxes
            .lock()
            .unwrap()
            .push(relay_msg::relay_pubkey_hex(key));
        self.wait().await.map(|_| (Vec::new(), None))
    }
}

fn buyer_account(c: &Connection) {
    let m = fx::buyer_master();
    store::insert_account(
        c,
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
}

/// A trade still agreeing its terms (no holdings), with its own id and key,
/// taken at `at`.
fn negotiating(c: &Connection, n: u8, at: u64) -> String {
    let mut t = new_trade();
    t.trade_id = format!("{n:02x}").repeat(16);
    t.created_at = at;
    t.holdings = None;
    t.terms = None;
    store::insert_trade(c, &t).unwrap();
    t.trade_id
}

/// Claims and refunds must not wait on the meeting point: a Nostr relay
/// that holds every connection would otherwise hold every trade's step,
/// up to the deadline a claim must meet.
#[tokio::test(start_paused = true)]
async fn the_trades_step_runs_before_the_meeting_point_is_asked_anything() {
    use crate::exchange_io::job::{tick, Ctx};
    use crate::exchange_io::meeting::MeetingPoint;
    let c = conn();
    buyer_account(&c);
    let late = negotiating(&c, 0xd1, fx::START);
    // Another trade still inside its window keeps an inbox to read.
    negotiating(&c, 0xd2, fx::START + fx::policy().buyer_fund_window);
    let point = SlowPoint::new(None);
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: Some(&point as &dyn MeetingPoint),
        watchtower: None,
        now: fx::START + fx::policy().buyer_fund_window + 1,
    };
    let ended = tokio::time::timeout(std::time::Duration::from_secs(600), tick(&ctx)).await;
    assert!(
        ended.is_err(),
        "the silent meeting point holds the rest of the tick"
    );
    assert!(
        *point.calls.lock().unwrap() >= 1,
        "the meeting point was asked"
    );
    let t = store::trade(&c, "p", &late).unwrap().unwrap();
    assert_eq!(
        t.status,
        Some(Status::Expired),
        "stepped before the meeting point"
    );
}

/// However many inboxes and offers a profile has, a tick spends at most
/// `MEETING_BUDGET_SECS` (plus the one call under way) on the meeting point,
/// so the next profile's step and the next sync are not held back.
#[tokio::test(start_paused = true)]
async fn a_tick_spends_at_most_its_budget_on_a_slow_meeting_point() {
    use crate::exchange_io::job::{tick, Ctx, MEETING_BUDGET_SECS};
    use crate::exchange_io::meeting::MeetingPoint;
    let c = conn();
    buyer_account(&c);
    for n in 0..12u8 {
        negotiating(&c, 0xa0 + n, fx::START);
    }
    let call = std::time::Duration::from_secs(25);
    let point = SlowPoint::new(Some(call));
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: Some(&point as &dyn MeetingPoint),
        watchtower: None,
        now: fx::START + 1,
    };
    let start = tokio::time::Instant::now();
    tick(&ctx).await;
    let spent = start.elapsed();
    assert!(
        spent <= std::time::Duration::from_secs(MEETING_BUDGET_SECS) + call,
        "{spent:?} on the meeting point"
    );
    let calls = *point.calls.lock().unwrap();
    assert!((1..12).contains(&calls), "{calls} calls");
}

/// Under the budget, the inboxes are read in turn: a tick starts where the
/// order moved on, so no trade's inbox waits for ever behind the others.
#[tokio::test(start_paused = true)]
async fn inboxes_left_unread_by_one_tick_are_read_first_later() {
    use crate::exchange_io::job::{tick, Ctx};
    use crate::exchange_io::meeting::MeetingPoint;
    let c = conn();
    buyer_account(&c);
    for n in 0..12u8 {
        // Inside its window for all twelve ticks.
        negotiating(&c, 0xb0 + n, fx::START + 3600);
    }
    let call = std::time::Duration::from_secs(25);
    let mut first = std::collections::BTreeSet::new();
    let mut read = std::collections::BTreeSet::new();
    for minute in 0..12u64 {
        let point = SlowPoint::new(Some(call));
        let ctx = Ctx {
            conn: &c,
            profile_id: "p",
            network: fx::NET,
            policy: fx::policy(),
            node: None,
            sources: &[],
            relay: Some(&point as &dyn MeetingPoint),
            watchtower: None,
            now: fx::START + 1 + minute * 60,
        };
        tick(&ctx).await;
        let asked = point.inboxes.lock().unwrap().clone();
        assert!(asked.len() < 12, "the budget cut the tick short");
        first.insert(asked[0].clone());
        read.extend(asked);
    }
    assert_eq!(read.len(), 12, "every inbox was read within twelve ticks");
    assert!(
        first.len() > 1,
        "the ticks did not all start at the same inbox"
    );
}

/// The chains are stepped before the meeting point, and again when it
/// handed something over: the maker's terms read in a tick ask the Buyer to
/// confirm in that tick, not one sync later.
#[tokio::test]
async fn terms_read_in_a_tick_ask_for_the_commit_in_the_same_tick() {
    use super::exchange_fake_btc::FakeRelay;
    use crate::exchange_io::job::{tick, Ctx, Event};
    use crate::exchange_io::meeting::MeetingPoint;
    use crate::noncustodial::exchange::keys;
    use crate::noncustodial::exchange::relay_msg::{Body, Message, TakeRequest};
    use crate::noncustodial::exchange::terms::{Terms, TermsDraft};
    let c = conn();
    buyer_account(&c);
    let maker = relay_msg::new_relay_key();
    let mut t = new_trade();
    t.trade_id = "e1".repeat(16);
    t.offer = crate::noncustodial::exchange::relay_msg::Offer {
        maker_relay_pubkey: relay_msg::relay_pubkey_hex(&maker),
        ..t.offer.offer.clone()
    }
    .sign(&maker)
    .unwrap();
    let b = keys::trade_keys(&fx::buyer_master(), fx::NET, 0, 0).unwrap();
    let s = keys::trade_keys(&fx::seller_master(), fx::NET, 0, 0).unwrap();
    let pk = |k: &secp256k1::SecretKey| {
        secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), k).serialize()
    };
    let terms = Terms::from_draft(
        &TermsDraft {
            network: fx::NET.as_str(),
            offer_id: &t.offer.offer.id(),
            trade_id: &t.trade_id,
            hns_amount: fx::HNS_AMOUNT,
            price_msat_per_hns: fx::PRICE,
            hash: b.hash(&fx::NONCE),
            seller_hns_refund_pubkey: s.hns_pubkey,
            buyer_hns_claim_pubkey: b.hns_pubkey,
            buyer_btc_refund_pubkey: b.btc_pubkey,
            seller_btc_claim_pubkey: s.btc_pubkey,
            seller_relay_pubkey: pk(&maker),
            buyer_relay_pubkey: pk(&t.relay_key),
            start: fx::START,
        },
        &fx::policy(),
    )
    .unwrap();
    t.request = Some(TakeRequest {
        hns_amount: fx::HNS_AMOUNT,
        hns_pubkey: terms.buyer_hns_claim_pubkey.clone(),
        btc_pubkey: terms.buyer_btc_refund_pubkey.clone(),
        hash: Some(terms.hash.clone()),
        start: fx::START,
    });
    t.terms = None;
    t.holdings = None;
    store::insert_trade(&c, &t).unwrap();
    let relay = FakeRelay::default();
    relay.messages.lock().unwrap().push(
        Message {
            version: 1,
            network: "regtest".into(),
            offer_id: t.offer.offer.id(),
            trade_id: t.trade_id.clone(),
            from: relay_msg::relay_pubkey_hex(&maker),
            to: relay_msg::relay_pubkey_hex(&t.relay_key),
            sent_at: fx::START,
            body: Body::Accept {
                terms: Box::new(terms),
            },
        }
        .sign(&maker)
        .unwrap(),
    );
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: Some(&relay as &dyn MeetingPoint),
        watchtower: None,
        now: fx::START + 30,
    };
    let report = tick(&ctx).await;
    let got = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    assert!(
        got.terms.is_some(),
        "the terms were taken: {:?}",
        report.errors
    );
    assert!(
        report
            .events
            .iter()
            .any(|e| matches!(e, Event::NeedsCommit { trade_id, .. } if *trade_id == t.trade_id)),
        "{:?} / {:?}",
        report.events,
        got.status
    );
}

/// QA 2026-11-15: the background sync rewrote every open trade at every
/// step, so a Commit window open across a step (60 s in the daemon, and the
/// app's own sync) was refused with "changed while the window was open". A
/// step that changes nothing writes nothing; the Commit read before it still
/// stores.
#[tokio::test]
async fn a_step_that_changes_nothing_does_not_refuse_a_commit_read_before_it() {
    use crate::exchange_io::job::{tick, Ctx};
    let c = conn();
    buyer_account(&c);
    let mut t = trade(&c);
    t.status = Some(Status::AwaitingBuyerCommit {
        deadline: fx::START + 600,
    });
    assert!(store::update_trade(&c, &mut t, fx::START).unwrap());
    let ctx = Ctx {
        conn: &c,
        profile_id: "p",
        network: fx::NET,
        policy: fx::policy(),
        node: None,
        sources: &[],
        relay: None,
        watchtower: None,
        now: fx::START + 30,
    };
    // One step settles whatever the step itself reports; the next ones read
    // the same chains and change nothing.
    tick(&ctx).await;
    let window = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    for now in [fx::START + 90, fx::START + 150] {
        tick(&Ctx {
            now,
            policy: fx::policy(),
            ..ctx
        })
        .await;
    }
    let mut commit = window.clone();
    commit.holdings = Some(fx::trade().buyer());
    assert!(
        store::commit_trade(&c, &mut commit, fx::START + 160).unwrap(),
        "the Commit was refused after steps that changed nothing"
    );
}

/// A committed Buyer's trade, stored, with its secret in the holdings.
fn committed_buyer(c: &Connection) -> Trade {
    let mut t = trade(c);
    t.holdings = Some(fx::trade().buyer());
    assert!(store::commit_trade(c, &mut t, 1).unwrap());
    let t = store::trade(c, "p", &t.trade_id).unwrap().unwrap();
    assert!(
        t.holdings.as_ref().unwrap().secret.is_some(),
        "the Commit stores the secret"
    );
    t
}

/// SECURITY.md: the Buyer's secret is on disk only while the daemon may
/// need it. Once a trade is no longer followed, nothing can use it, so it
/// is not kept in the file.
#[test]
fn a_finished_trade_no_longer_keeps_the_buyers_secret() {
    let c = conn();
    let mut t = committed_buyer(&c);
    t.status = Some(Status::Completed);
    t.terminal_at = Some(2);
    t.finished = true;
    assert!(store::update_trade(&c, &mut t, 3).unwrap());
    let stored = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    assert_eq!(stored.holdings.unwrap().secret, None);
}

/// An ended trade is followed for a day: a reorg can undo the HNS claim,
/// which is finished again with the secret, so it stays until then.
#[test]
fn an_ended_trade_still_followed_keeps_the_secret() {
    let c = conn();
    let mut t = committed_buyer(&c);
    t.status = Some(Status::Completed);
    t.terminal_at = Some(2);
    assert!(store::update_trade(&c, &mut t, 3).unwrap());
    let stored = store::trade(&c, "p", &t.trade_id).unwrap().unwrap();
    assert!(stored.holdings.unwrap().secret.is_some());
}

/// Trades finished before the rule above still hold their secret: the
/// migration forgets it there, and only there.
#[test]
fn the_migration_forgets_the_secrets_of_finished_trades_only() {
    let c = conn();
    let done = committed_buyer(&c);
    let mut open = new_trade();
    open.trade_id = "cd".repeat(16);
    store::insert_trade(&c, &open).unwrap();
    assert!(store::update_trade(&c, &mut open, fx::START).unwrap());
    let mut open = store::trade(&c, "p", &open.trade_id).unwrap().unwrap();
    open.holdings = Some(fx::trade().buyer());
    assert!(store::commit_trade(&c, &mut open, 1).unwrap());
    // A row written by an older build: finished, secret still there.
    c.execute(
        "UPDATE exchange_trades SET finished = 1 WHERE trade_id = ?1",
        [&done.trade_id],
    )
    .unwrap();
    c.execute_batch(include_str!("../sql/045_exchange_forget_secrets.sql"))
        .unwrap();
    let done = store::trade(&c, "p", &done.trade_id).unwrap().unwrap();
    let open = store::trade(&c, "p", &open.trade_id).unwrap().unwrap();
    assert_eq!(done.holdings.unwrap().secret, None);
    assert!(open.holdings.unwrap().secret.is_some());
}
