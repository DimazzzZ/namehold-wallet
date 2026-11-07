//! The trade engine, honest and adversarial (spec §3.3, §3.4, R8–R11, R13).
//! Every case runs the real engine on holdings the real Commit code signed
//! (`exchange_fixture`); sent transactions are decoded and, on Bitcoin, run
//! through Bitcoin Core's interpreter.

use bitcoin::consensus::validation::verify_transaction;

use super::exchange_fixture::{self as fx, Trade, START};
use crate::noncustodial::exchange::btc_htlc;
use crate::noncustodial::exchange::engine::{
    self, Action, Attention, Chain, Holdings, SpendKind, Status, Step,
};
use crate::noncustodial::exchange::hns_htlc;
use crate::noncustodial::exchange::watch::WatchJob;
use crate::noncustodial::tx::Transaction;

fn run(
    h: &Holdings,
    hns: Option<&engine::HnsSeen>,
    btc: Option<&engine::BtcSeen>,
    now: u64,
) -> Step {
    engine::step(h, hns, btc, now, &fx::policy())
}

fn only_action(s: &Step) -> &Action {
    assert_eq!(s.actions.len(), 1, "{:?}", s);
    &s.actions[0]
}

fn hns_tx(a: &Action, kind: SpendKind) -> Transaction {
    match a {
        Action::SendHns { kind: k, tx_hex } if *k == kind => {
            Transaction::decode(&hex::decode(tx_hex).unwrap()).unwrap()
        }
        other => panic!("expected {kind:?} on HNS, got {other:?}"),
    }
}

fn btc_tx(a: &Action, kind: SpendKind) -> (bitcoin::Transaction, usize) {
    match a {
        Action::SendBtc {
            kind: k,
            tx_hex,
            rung,
        } if *k == kind => (btc_htlc::decode(tx_hex).unwrap(), *rung),
        other => panic!("expected {kind:?} on BTC, got {other:?}"),
    }
}

fn core_ok(t: &Trade, tx: &bitcoin::Transaction) {
    let out = t.btc_lock_txout();
    verify_transaction(tx, |_| Some(out.clone())).unwrap();
}

/// A median time past at which the HNS refund is open.
fn hns_open(t: &Trade) -> u64 {
    hns_htlc::time_lock_secs(t.terms.hns_params().unwrap().refund_lock_time).unwrap() + 1
}

fn btc_open(t: &Trade) -> u64 {
    t.terms.btc_refund_time + 1
}

// ---------------------------------------------------------------- Buyer

#[test]
fn buyer_before_commit_waits_then_expires() {
    let t = fx::trade();
    let mut h = t.buyer();
    h.committed = false;
    assert_eq!(
        run(&h, None, None, START).status,
        Status::AwaitingBuyerCommit {
            deadline: t.terms.buyer_fund_deadline
        }
    );
    assert_eq!(
        run(&h, None, None, t.terms.buyer_fund_deadline + 1).status,
        Status::Expired
    );
}

#[test]
fn buyer_resends_its_funding_while_the_btc_lock_is_unconfirmed() {
    let t = fx::trade();
    let s = run(
        &t.buyer(),
        Some(&Trade::hns_empty(START)),
        Some(&t.btc_locked(START, 0)),
        START + 60,
    );
    assert_eq!(
        s.status,
        Status::AwaitingBtcLock {
            confirmations: 0,
            needed: 1
        }
    );
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcFunding);
    assert_eq!(tx.compute_txid().to_string(), t.buyer.btc_lock.txid);
}

#[test]
fn buyer_waits_for_the_seller_then_for_confirmations() {
    let t = fx::trade();
    let s = run(
        &t.buyer(),
        Some(&Trade::hns_empty(START)),
        Some(&t.btc_locked(START, 1)),
        START + 60,
    );
    assert_eq!(
        s.status,
        Status::AwaitingSellerCommit {
            deadline: t.terms.seller_fund_deadline
        }
    );
    assert!(s.actions.is_empty());
    let s = run(
        &t.buyer(),
        Some(&t.hns_locked(START, 0)),
        Some(&t.btc_locked(START, 1)),
        START + 60,
    );
    assert_eq!(
        s.status,
        Status::AwaitingHnsLock {
            confirmations: 0,
            needed: 1
        }
    );
    assert!(s.actions.is_empty(), "no reveal before the confirmations");
}

#[test]
fn buyer_reveals_with_a_claim_of_exactly_the_lock() {
    let t = fx::trade();
    let s = run(
        &t.buyer(),
        Some(&t.hns_locked(START, 1)),
        Some(&t.btc_locked(START, 1)),
        START + 600,
    );
    assert_eq!(s.status, Status::Revealing);
    let tx = hns_tx(only_action(&s), SpendKind::HnsClaim);
    let hp = t.terms.hns_params().unwrap();
    assert_eq!(hex::encode(tx.inputs[0].prevout.hash), t.hns_lock().txid);
    assert_eq!(tx.inputs[0].prevout.index, t.hns_lock().vout);
    assert_eq!(hns_htlc::secret_from_spend(&tx, 0, &hp), Some(t.secret()));
    // The output is the Buyer's own address.
    assert_eq!(tx.outputs.len(), 1);
}

#[test]
fn buyer_never_reveals_at_or_after_the_reveal_deadline() {
    let t = fx::trade();
    for now in [
        t.terms.reveal_deadline,
        t.terms.reveal_deadline + 1,
        btc_open(&t),
    ] {
        let s = run(
            &t.buyer(),
            Some(&t.hns_locked(START, 5)),
            Some(&t.btc_locked(START, 5)),
            now,
        );
        assert!(
            !s.actions.iter().any(|a| matches!(
                a,
                Action::SendHns {
                    kind: SpendKind::HnsClaim,
                    ..
                }
            )),
            "revealed at {now}: {s:?}"
        );
    }
}

#[test]
fn buyer_never_reveals_when_the_hns_chain_says_the_refund_is_near() {
    // The local clock is early, but the chain's median time is within the
    // reveal margin of the HNS refund time.
    let t = fx::trade();
    let mtp = t.terms.hns_refund_time - fx::policy().reveal_margin;
    let s = run(
        &t.buyer(),
        Some(&t.hns_locked(mtp, 5)),
        Some(&t.btc_locked(START, 5)),
        START + 60,
    );
    assert!(s.actions.is_empty(), "{s:?}");
}

#[test]
fn buyer_does_not_reveal_for_a_lock_of_another_amount_and_refunds_later() {
    let t = fx::trade();
    let mut wrong = Trade::hns_empty(START);
    wrong.mismatched_payment = true;
    let s = run(
        &t.buyer(),
        Some(&wrong),
        Some(&t.btc_locked(START, 5)),
        START + 600,
    );
    assert_eq!(
        s.status,
        Status::NeedsAttention {
            reason: Attention::SellerLockedWrongAmount,
            chain: Chain::Hns
        }
    );
    assert!(s.actions.is_empty());
    let s = run(
        &t.buyer(),
        Some(&wrong),
        Some(&t.btc_locked(btc_open(&t), 5)),
        btc_open(&t),
    );
    assert_eq!(s.status, Status::Refunding { chain: Chain::Btc });
    let (tx, rung) = btc_tx(only_action(&s), SpendKind::BtcRefund);
    assert_eq!(rung, 0);
    core_ok(&t, &tx);
}

#[test]
fn buyer_refunds_only_once_the_bitcoin_median_time_is_past() {
    let t = fx::trade();
    let late = t.terms.reveal_deadline + 10;
    let s = run(
        &t.buyer(),
        Some(&Trade::hns_empty(late)),
        Some(&t.btc_locked(t.terms.btc_refund_time, 5)),
        btc_open(&t) + 999,
    );
    assert_eq!(
        s.status,
        Status::AwaitingRefund {
            chain: Chain::Btc,
            opens_at: t.terms.btc_refund_time
        },
        "the median time is exactly the refund time: not yet"
    );
    let s = run(
        &t.buyer(),
        Some(&Trade::hns_empty(late)),
        Some(&t.btc_locked(btc_open(&t), 5)),
        btc_open(&t),
    );
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcRefund);
    core_ok(&t, &tx);
}

#[test]
fn buyer_never_refunds_btc_once_the_hns_lock_is_claimed() {
    // The honest-wallet rule: even long past the BTC refund time, with the
    // Seller never claiming, the BTC stays the Seller's.
    let t = fx::trade();
    let mut h = t.buyer();
    h.sent.hns_claim_at = Some(START + 700);
    let s = run(
        &h,
        Some(&t.hns_claimed(btc_open(&t), 3)),
        Some(&t.btc_locked(btc_open(&t) + 86_400, 5)),
        btc_open(&t) + 86_400,
    );
    assert_eq!(s.status, Status::Completed);
    assert!(s.actions.is_empty());
}

#[test]
fn buyer_keeps_sending_its_claim_while_it_is_in_the_mempool() {
    let t = fx::trade();
    let mut h = t.buyer();
    h.sent.hns_claim_at = Some(START + 700);
    let s = run(
        &h,
        Some(&t.hns_claimed(START + 800, 0)),
        Some(&t.btc_locked(START, 5)),
        START + 900,
    );
    assert_eq!(s.status, Status::Revealing);
    hns_tx(only_action(&s), SpendKind::HnsClaim);
}

#[test]
fn buyer_never_reveals_after_it_started_refunding() {
    let t = fx::trade();
    let mut h = t.buyer();
    h.sent.btc_refund_first_at = Some(START + 100);
    let s = run(
        &h,
        Some(&t.hns_locked(START, 5)),
        Some(&t.btc_locked(START, 5)),
        START + 600,
    );
    assert!(
        !s.actions
            .iter()
            .any(|a| matches!(a, Action::SendHns { .. })),
        "{s:?}"
    );
}

#[test]
fn buyer_claims_hns_at_once_if_the_secret_is_already_public_on_bitcoin() {
    let t = fx::trade();
    let now = t.terms.reveal_deadline + 60; // past the deadline: no matter
    let s = run(
        &t.buyer(),
        Some(&t.hns_locked(now, 5)),
        Some(&t.btc_spent(now, 1, true)),
        now,
    );
    assert_eq!(s.status, Status::Revealing);
    hns_tx(only_action(&s), SpendKind::HnsClaim);
}

#[test]
fn buyer_whose_reveal_was_beaten_by_a_refund_refunds_btc_when_it_can() {
    let t = fx::trade();
    let mut h = t.buyer();
    h.sent.hns_claim_at = Some(START + 2600);
    let s = run(
        &h,
        Some(&t.hns_refunded(hns_open(&t), 1)),
        Some(&t.btc_locked(hns_open(&t), 5)),
        hns_open(&t),
    );
    assert_eq!(
        s.status,
        Status::NeedsAttention {
            reason: Attention::HnsRefundedAfterReveal,
            chain: Chain::Btc
        }
    );
    let s = run(
        &h,
        Some(&t.hns_refunded(btc_open(&t), 1)),
        Some(&t.btc_locked(btc_open(&t), 5)),
        btc_open(&t),
    );
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcRefund);
    core_ok(&t, &tx);
}

#[test]
fn buyer_without_the_hns_chain_never_refunds() {
    // Review L5: a reveal's record can be lost in a crash between the send
    // and the store, so "I never revealed" is not known for sure; without
    // the HNS chain nothing is decided.
    let t = fx::trade();
    for revealed in [false, true] {
        let mut h = t.buyer();
        if revealed {
            h.sent.hns_claim_at = Some(START + 600);
        }
        let s = run(&h, None, Some(&t.btc_locked(btc_open(&t), 5)), btc_open(&t));
        assert_eq!(s.status, Status::CannotCheck { chain: Chain::Hns });
        assert!(s.actions.is_empty());
    }
}

#[test]
fn the_refund_ladder_climbs_with_time() {
    let t = fx::trade();
    let p = fx::policy();
    let mut h = t.buyer();
    let first = btc_open(&t);
    h.sent.btc_refund_first_at = Some(first);
    let at = first + 2 * p.ladder_escalate_after;
    let s = run(
        &h,
        Some(&Trade::hns_empty(at)),
        Some(&t.btc_locked(at, 5)),
        at,
    );
    let (tx, rung) = btc_tx(only_action(&s), SpendKind::BtcRefund);
    assert_eq!(rung, 2);
    core_ok(&t, &tx);
    assert!(t.buyer.btc_refunds[2].fee > t.buyer.btc_refunds[1].fee);
}

#[test]
fn buyer_refund_confirmed_is_refunded() {
    let t = fx::trade();
    let s = run(
        &t.buyer(),
        Some(&Trade::hns_empty(btc_open(&t))),
        Some(&t.btc_spent(btc_open(&t), 1, false)),
        btc_open(&t),
    );
    assert_eq!(s.status, Status::Refunded { chain: Chain::Btc });
}

// --------------------------------------------------------------- Seller

fn seller_uncommitted(t: &Trade) -> Holdings {
    let mut h = t.seller();
    h.committed = false;
    h
}

#[test]
fn seller_waits_for_the_buyer_and_expires_without_it() {
    let t = fx::trade();
    let h = seller_uncommitted(&t);
    assert_eq!(
        run(&h, None, Some(&Trade::btc_empty(START)), START).status,
        Status::AwaitingBuyerCommit {
            deadline: t.terms.buyer_fund_deadline
        }
    );
    assert_eq!(
        run(
            &h,
            None,
            Some(&Trade::btc_empty(START)),
            t.terms.buyer_fund_deadline + 1
        )
        .status,
        Status::Expired
    );
}

#[test]
fn seller_commits_only_on_a_matching_confirmed_unspent_lock_before_its_deadline() {
    let t = fx::trade();
    let h = seller_uncommitted(&t);
    let now = START + 60;
    assert!(engine::seller_may_commit(&h, Some(&t.btc_locked(now, 1)), now).is_ok());
    assert_eq!(
        run(&h, None, Some(&t.btc_locked(now, 1)), now).status,
        Status::AwaitingSellerCommit {
            deadline: t.terms.seller_fund_deadline
        }
    );
    // Too few confirmations.
    let mut few = t.btc_locked(now, 0);
    assert!(engine::seller_may_commit(&h, Some(&few), now).is_err());
    // Not the lock (another amount or script).
    few.lock.as_mut().unwrap().confirmations = 5;
    few.lock.as_mut().unwrap().matches = false;
    assert!(engine::seller_may_commit(&h, Some(&few), now).is_err());
    assert_eq!(
        run(&h, None, Some(&few), now).status,
        Status::NeedsAttention {
            reason: Attention::BuyerLockedWrongAmount,
            chain: Chain::Btc
        }
    );
    // Spent.
    assert!(engine::seller_may_commit(&h, Some(&t.btc_spent(now, 1, false)), now).is_err());
    // No Bitcoin view.
    assert!(engine::seller_may_commit(&h, None, now).is_err());
    // Past the deadline.
    let late = t.terms.seller_fund_deadline + 1;
    assert!(engine::seller_may_commit(&h, Some(&t.btc_locked(late, 3)), late).is_err());
    assert_eq!(
        run(&h, None, Some(&t.btc_locked(late, 3)), late).status,
        Status::Expired
    );
}

#[test]
fn seller_resends_its_funding_until_the_lock_is_seen() {
    let t = fx::trade();
    let s = run(
        &t.seller(),
        Some(&Trade::hns_empty(START)),
        Some(&t.btc_locked(START, 1)),
        START + 100,
    );
    let tx = hns_tx(only_action(&s), SpendKind::HnsFunding);
    assert_eq!(tx.txid(), t.hns_lock().txid);
}

#[test]
fn seller_waits_for_the_reveal() {
    let t = fx::trade();
    let s = run(
        &t.seller(),
        Some(&t.hns_locked(START + 900, 2)),
        Some(&t.btc_locked(START, 1)),
        START + 900,
    );
    assert_eq!(
        s.status,
        Status::AwaitingReveal {
            refund_opens_at: t.terms.hns_refund_time
        }
    );
    assert!(s.actions.is_empty());
}

#[test]
fn seller_claims_btc_with_the_revealed_secret_even_from_the_mempool() {
    let t = fx::trade();
    let s = run(
        &t.seller(),
        Some(&t.hns_claimed(START + 900, 0)),
        Some(&t.btc_locked(START, 1)),
        START + 900,
    );
    assert_eq!(s.status, Status::Claiming { rung: 0 });
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcClaim);
    core_ok(&t, &tx);
}

#[test]
fn seller_never_refunds_hns_once_a_claim_is_seen() {
    // The refund time is open, but the Buyer's claim is in the mempool:
    // claim BTC, never race the Buyer for the HNS.
    let t = fx::trade();
    let s = run(
        &t.seller(),
        Some(&t.hns_claimed(hns_open(&t), 0)),
        Some(&t.btc_locked(hns_open(&t), 3)),
        hns_open(&t),
    );
    assert!(s.actions.iter().all(|a| !matches!(
        a,
        Action::SendHns {
            kind: SpendKind::HnsRefund,
            ..
        }
    )));
    btc_tx(only_action(&s), SpendKind::BtcClaim);
}

#[test]
fn seller_refunds_hns_only_once_the_hns_median_time_is_past() {
    let t = fx::trade();
    let boundary = hns_open(&t) - 1;
    let s = run(
        &t.seller(),
        Some(&t.hns_locked(boundary, 9)),
        Some(&t.btc_locked(boundary, 3)),
        boundary + 999,
    );
    assert!(s.actions.is_empty(), "{s:?}");
    assert_eq!(
        s.status,
        Status::AwaitingRefund {
            chain: Chain::Hns,
            opens_at: t.terms.hns_refund_time
        }
    );
    let s = run(
        &t.seller(),
        Some(&t.hns_locked(hns_open(&t), 9)),
        Some(&t.btc_locked(hns_open(&t), 3)),
        hns_open(&t),
    );
    assert_eq!(s.status, Status::Refunding { chain: Chain::Hns });
    let tx = hns_tx(only_action(&s), SpendKind::HnsRefund);
    assert_eq!(tx.locktime, t.terms.hns_params().unwrap().refund_lock_time);
}

#[test]
fn seller_claim_jumps_to_the_top_rung_near_the_btc_refund_time() {
    let t = fx::trade();
    let p = fx::policy();
    let now = t.terms.btc_refund_time - p.ladder_top_before;
    let mut h = t.seller();
    h.sent.btc_claim_first_at = Some(now - 1);
    let s = run(
        &h,
        Some(&t.hns_claimed(now, 4)),
        Some(&t.btc_locked(now, 3)),
        now,
    );
    let (tx, rung) = btc_tx(only_action(&s), SpendKind::BtcClaim);
    assert_eq!(rung, t.seller.btc_claims.len() - 1);
    core_ok(&t, &tx);
}

#[test]
fn seller_completed_refunded_and_too_late() {
    let t = fx::trade();
    let now = START + 1000;
    assert_eq!(
        run(
            &t.seller(),
            Some(&t.hns_claimed(now, 2)),
            Some(&t.btc_spent(now, 1, true)),
            now
        )
        .status,
        Status::Completed
    );
    assert_eq!(
        run(
            &t.seller(),
            Some(&t.hns_claimed(now, 2)),
            Some(&t.btc_spent(now, 1, false)),
            now
        )
        .status,
        Status::NeedsAttention {
            reason: Attention::BtcRefundedAfterReveal,
            chain: Chain::Btc
        }
    );
    assert_eq!(
        run(
            &t.seller(),
            Some(&t.hns_refunded(hns_open(&t), 1)),
            Some(&t.btc_locked(hns_open(&t), 3)),
            hns_open(&t)
        )
        .status,
        Status::Refunded { chain: Chain::Hns }
    );
}

#[test]
fn seller_without_the_hns_chain_does_nothing() {
    let t = fx::trade();
    let s = run(
        &t.seller(),
        None,
        Some(&t.btc_locked(btc_open(&t), 3)),
        btc_open(&t),
    );
    assert_eq!(s.status, Status::CannotCheck { chain: Chain::Hns });
    assert!(s.actions.is_empty());
}

// ---------------------------------------------------------- Watchtowers

#[test]
fn a_watch_job_carries_no_secret_and_no_claim_of_the_buyers() {
    let t = fx::trade();
    let job = WatchJob::from_holdings(&t.buyer());
    let json = serde_json::to_string(&job).unwrap();
    assert!(!json.contains(&hex::encode(t.secret())));
    assert!(!json.contains(&t.buyer.hns_claim.signature_hex));
    job.check().unwrap();
    WatchJob::from_holdings(&t.seller()).check().unwrap();
    let h = job.holdings(Default::default());
    assert!(h.secret.is_none() && h.hns_claim.is_none());
}

#[test]
fn a_watch_job_of_the_wrong_shape_is_refused() {
    let t = fx::trade();
    let mut job = WatchJob::from_holdings(&t.seller());
    job.btc_claims.clear();
    assert!(job.check().is_err());
    let mut job = WatchJob::from_holdings(&t.buyer());
    job.btc_claims = t.seller.btc_claims.clone();
    assert!(job.check().is_err());
    let mut job = WatchJob::from_holdings(&t.seller());
    job.hns_refund_hex = t.buyer.holdings.btc_funding_hex.clone();
    assert!(job.check().is_err());
}

#[test]
fn the_sellers_watchtower_claims_btc_once_the_secret_is_public() {
    let t = fx::trade();
    let h = WatchJob::from_holdings(&t.seller()).holdings(Default::default());
    let s = run(
        &h,
        Some(&t.hns_claimed(START + 900, 1)),
        Some(&t.btc_locked(START, 1)),
        START + 900,
    );
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcClaim);
    core_ok(&t, &tx);
}

#[test]
fn the_buyers_watchtower_refunds_only_an_unclaimed_trade_and_never_reveals() {
    let t = fx::trade();
    let h = WatchJob::from_holdings(&t.buyer()).holdings(Default::default());
    // HNS lock confirmed and revealable: the watchtower cannot reveal.
    let s = run(
        &h,
        Some(&t.hns_locked(START, 5)),
        Some(&t.btc_locked(START, 5)),
        START + 600,
    );
    assert!(s.actions.is_empty(), "{s:?}");
    // Claimed: never refund.
    let s = run(
        &h,
        Some(&t.hns_claimed(btc_open(&t), 3)),
        Some(&t.btc_locked(btc_open(&t), 5)),
        btc_open(&t),
    );
    assert_eq!(s.status, Status::Completed);
    assert!(s.actions.is_empty());
    // No HNS view: nothing.
    let s = run(&h, None, Some(&t.btc_locked(btc_open(&t), 5)), btc_open(&t));
    assert!(s.actions.is_empty());
    // Never funded HNS: refund.
    let s = run(
        &h,
        Some(&Trade::hns_empty(btc_open(&t))),
        Some(&t.btc_locked(btc_open(&t), 5)),
        btc_open(&t),
    );
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcRefund);
    core_ok(&t, &tx);
}

// ------------------------------------------------------ Whole trades

/// Both parties and their watchtowers stepping on one simulated pair of
/// chains, as the daemon does, until nothing changes. `seller_commits`,
/// `buyer_online` and `seller_online` script who misbehaves; the result is
/// (Buyer's final status, Seller's final status, coins each side ends up
/// with: (HNS to Buyer, BTC to Seller, HNS back to Seller, BTC back to
/// Buyer)).
struct World {
    hns: engine::HnsSeen,
    btc: engine::BtcSeen,
    now: u64,
}

fn simulate(
    seller_commits: bool,
    buyer_online: bool,
    seller_online: bool,
    seller_watchtower: bool,
) -> (bool, bool, bool, bool) {
    let t = fx::trade();
    let mut buyer = t.buyer();
    let mut seller = t.seller();
    seller.committed = false;
    let buyer_job = WatchJob::from_holdings(&t.buyer());
    let seller_job = WatchJob::from_holdings(&t.seller());
    let mut w = World {
        hns: Trade::hns_empty(START),
        btc: t.btc_locked(START, 0),
        now: START,
    };
    let mut seller_watch_on = false;
    let (mut hns_to_buyer, mut btc_to_seller, mut hns_back, mut btc_back) =
        (false, false, false, false);
    while w.now < t.terms.btc_refund_time + 3 * 3600 {
        w.now += 300;
        w.hns.mtp = w.now - 1800;
        w.btc.mtp = w.now - 1800;
        if let Some(l) = w.btc.lock.as_mut() {
            l.confirmations += 1;
            if let Some(s) = l.spend.as_mut() {
                s.confirmations += 1;
            }
        }
        if let Some(l) = w.hns.lock.as_mut() {
            l.confirmations += 1;
            if let Some(s) = l.spend.as_mut() {
                s.confirmations += 1;
            }
        }
        // The Seller's Commit, as the person would confirm it.
        if seller_commits
            && !seller.committed
            && engine::seller_may_commit(&seller, Some(&w.btc), w.now).is_ok()
        {
            seller.committed = true;
            w.hns = t.hns_locked(w.hns.mtp, 0);
            seller_watch_on = seller_watchtower;
        }
        let mut steps = Vec::new();
        if buyer_online {
            steps.push(run(&buyer, Some(&w.hns), Some(&w.btc), w.now));
        }
        if seller_online && seller.committed {
            steps.push(run(&seller, Some(&w.hns), Some(&w.btc), w.now));
        }
        steps.push(run(
            &buyer_job.holdings(buyer.sent.clone()),
            Some(&w.hns),
            Some(&w.btc),
            w.now,
        ));
        if seller_watch_on {
            steps.push(run(
                &seller_job.holdings(seller.sent.clone()),
                Some(&w.hns),
                Some(&w.btc),
                w.now,
            ));
        }
        for s in steps {
            for a in s.actions {
                match a {
                    Action::SendHns {
                        kind: SpendKind::HnsClaim,
                        ..
                    } => {
                        let l = w.hns.lock.as_mut().unwrap();
                        if l.spend.is_none() {
                            l.spend = Some(engine::SpendSeen {
                                txid: "c1".repeat(32),
                                confirmations: 0,
                                secret: Some(t.secret()),
                            });
                            buyer.sent.hns_claim_at.get_or_insert(w.now);
                            hns_to_buyer = true;
                        }
                    }
                    Action::SendHns {
                        kind: SpendKind::HnsRefund,
                        ..
                    } => {
                        let l = w.hns.lock.as_mut().unwrap();
                        if l.spend.is_none() {
                            l.spend = Some(engine::SpendSeen {
                                txid: "r1".repeat(32),
                                confirmations: 0,
                                secret: None,
                            });
                            hns_back = true;
                        }
                    }
                    Action::SendBtc {
                        kind: SpendKind::BtcClaim,
                        ..
                    } => {
                        let l = w.btc.lock.as_mut().unwrap();
                        if l.spend.is_none() {
                            l.spend = Some(engine::SpendSeen {
                                txid: "c2".repeat(32),
                                confirmations: 0,
                                secret: Some(t.secret()),
                            });
                            seller.sent.btc_claim_first_at.get_or_insert(w.now);
                            btc_to_seller = true;
                        }
                    }
                    Action::SendBtc {
                        kind: SpendKind::BtcRefund,
                        ..
                    } => {
                        let l = w.btc.lock.as_mut().unwrap();
                        if l.spend.is_none() {
                            l.spend = Some(engine::SpendSeen {
                                txid: "r2".repeat(32),
                                confirmations: 0,
                                secret: None,
                            });
                            buyer.sent.btc_refund_first_at.get_or_insert(w.now);
                            btc_back = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    (hns_to_buyer, btc_to_seller, hns_back, btc_back)
}

#[test]
fn an_honest_trade_pays_both_sides() {
    assert_eq!(
        simulate(true, true, true, false),
        (true, true, false, false)
    );
}

#[test]
fn a_seller_who_never_commits_costs_the_buyer_only_time() {
    assert_eq!(
        simulate(false, true, true, false),
        (false, false, false, true)
    );
}

#[test]
fn a_buyer_who_goes_offline_before_the_reveal_gets_its_btc_back_and_the_seller_its_hns() {
    // The Buyer's watchtower refunds the BTC; the Seller refunds the HNS.
    assert_eq!(
        simulate(true, false, true, false),
        (false, false, true, true)
    );
}

#[test]
fn a_seller_offline_after_committing_is_saved_by_its_watchtower() {
    assert_eq!(
        simulate(true, true, false, true),
        (true, true, false, false)
    );
}

#[test]
fn in_every_scripted_failure_nobody_ends_with_both_coins_or_neither() {
    for sc in [true, false] {
        for bo in [true, false] {
            for so in [true, false] {
                for wt in [true, false] {
                    let (hb, bs, hback, bback) = simulate(sc, bo, so, wt);
                    // Buyer: exactly one of "got HNS" and "got BTC back" when it
                    // locked BTC (it always does here).
                    assert!(
                        hb ^ bback,
                        "buyer: hns {hb} btc back {bback} ({sc},{bo},{so},{wt})"
                    );
                    // Seller, when it locked HNS and it or its watchtower is
                    // running: exactly one of "got BTC" and "got HNS back".
                    // With neither running nothing is sent for it: its HNS
                    // stays in the lock, refundable by its presigned refund
                    // whenever it comes back (or, after a reveal, claimable
                    // BTC: the liveness gap of spec §3.4).
                    if sc && (so || wt) {
                        assert!(
                            bs ^ hback,
                            "seller: btc {bs} hns back {hback} ({sc},{bo},{so},{wt})"
                        );
                    }
                    // Never both sides of one lock.
                    assert!(!(hb && hback));
                    assert!(!(bs && bback));
                }
            }
        }
    }
}

#[test]
fn a_watch_job_signed_by_someone_else_is_refused() {
    // A counterparty who knows the terms must not be able to register a job
    // under the other party's id: every presigned spend in a job must carry
    // the lock key's signature.
    let t = fx::trade();
    let other = fx::trade_at(START + 1000); // same keys, other locks
    let mut job = WatchJob::from_holdings(&t.buyer());
    job.btc_refunds = other.buyer.btc_refunds.clone();
    assert!(job.check().is_err(), "refunds of another lock");
    let mut job = WatchJob::from_holdings(&t.seller());
    job.hns_refund_hex = other.seller.holdings.hns_refund_hex.clone();
    assert!(job.check().is_err(), "a refund of another lock");
    // A claim whose signature was replaced by garbage.
    let mut job = WatchJob::from_holdings(&t.seller());
    let mut tx = btc_htlc::decode(&job.btc_claims[0].tx_hex).unwrap();
    let mut items = tx.input[0].witness.to_vec();
    items[0][10] ^= 1;
    tx.input[0].witness = bitcoin::Witness::from_slice(&items);
    job.btc_claims[0].tx_hex = btc_htlc::encode_tx(&tx);
    assert!(job.check().is_err(), "a claim with a broken signature");
}

#[test]
fn a_bitcoind_source_error_never_shows_its_password() {
    let e = crate::exchange_io::btc_source::source_from_spec(
        "bitcoind+http://alice:hunter2@example.com:8332",
    )
    .err()
    .unwrap()
    .to_string();
    assert!(!e.contains("hunter2"), "{e}");
}

// ------------------------------------------------- Review C1, H1, H2

#[test]
fn the_seller_claims_btc_when_the_secret_is_revealed_on_another_coin() {
    // Review C1: the Buyer claims the Seller's coin while the view's lock is
    // something else; the secret revealed anywhere at the lock's script is
    // enough for the Seller to claim.
    let t = fx::trade();
    let mut v = t.hns_locked(START + 900, 2);
    v.revealed = Some(t.secret());
    let s = run(
        &t.seller(),
        Some(&v),
        Some(&t.btc_locked(START, 1)),
        START + 900,
    );
    let (tx, _) = btc_tx(only_action(&s), SpendKind::BtcClaim);
    core_ok(&t, &tx);
    // With the HNS refund time open it still claims the BTC, and takes back
    // its own coin nobody claimed (the decoy case of review E35).
    let mut v = t.hns_locked(hns_open(&t), 2);
    v.revealed = Some(t.secret());
    let s = run(
        &t.seller(),
        Some(&v),
        Some(&t.btc_locked(hns_open(&t), 1)),
        hns_open(&t),
    );
    assert!(s.actions.iter().any(|a| matches!(
        a,
        Action::SendBtc {
            kind: SpendKind::BtcClaim,
            ..
        }
    )));
    assert!(s.actions.iter().any(|a| matches!(
        a,
        Action::SendHns {
            kind: SpendKind::HnsRefund,
            ..
        }
    )));
    // But never while a claim of its own coin is seen.
    let s = run(
        &t.seller(),
        Some(&t.hns_claimed(hns_open(&t), 0)),
        Some(&t.btc_locked(hns_open(&t), 1)),
        hns_open(&t),
    );
    assert!(s
        .actions
        .iter()
        .all(|a| !matches!(a, Action::SendHns { .. })));
}

#[test]
fn the_buyer_never_refunds_once_a_secret_is_public_on_hns() {
    let t = fx::trade();
    let mut v = Trade::hns_empty(btc_open(&t));
    v.revealed = Some(t.secret());
    let s = run(
        &t.buyer(),
        Some(&v),
        Some(&t.btc_locked(btc_open(&t), 5)),
        btc_open(&t),
    );
    assert_eq!(s.status, Status::Revealing);
    assert!(s
        .actions
        .iter()
        .all(|a| !matches!(a, Action::SendBtc { .. })));
}

#[test]
fn an_unconfirmed_competing_refund_keeps_the_seller_on_its_top_rung() {
    // Review H1/L7: only a refund proven mined ends the Seller's claiming.
    let t = fx::trade();
    let s = run(
        &t.seller(),
        Some(&t.hns_claimed(START + 900, 1)),
        Some(&t.btc_spent(START + 900, 0, false)),
        START + 900,
    );
    let (_, rung) = btc_tx(only_action(&s), SpendKind::BtcClaim);
    assert_eq!(rung, t.seller.btc_claims.len() - 1);
}

#[test]
fn a_committed_buyer_whose_lock_no_source_shows_is_never_expired() {
    // Review H2: past every deadline, a BTC lock no source knows is not a
    // lock that does not exist.
    let t = fx::trade();
    let late = btc_open(&t) + 3600;
    let s = run(
        &t.buyer(),
        Some(&Trade::hns_empty(late)),
        Some(&Trade::btc_empty(late)),
        late,
    );
    assert_eq!(s.status, Status::CannotCheck { chain: Chain::Btc });
    btc_tx(only_action(&s), SpendKind::BtcFunding);
}

#[test]
fn a_seller_whose_own_coin_stayed_unspent_after_a_decoy_claim_takes_it_back_after_its_time() {
    // Review E35: the secret came from a claim of another coin; the
    // Seller's own coin, never claimed, comes back once its refund opens.
    let t = fx::trade();
    let mut v = t.hns_locked(hns_open(&t), 5);
    v.revealed = Some(t.secret());
    let s = run(
        &t.seller(),
        Some(&v),
        Some(&t.btc_spent(hns_open(&t), 1, true)),
        hns_open(&t),
    );
    assert_eq!(s.status, Status::Completed);
    hns_tx(only_action(&s), SpendKind::HnsRefund);
    // Before its refund time it is left for the Buyer to claim.
    let mut v = t.hns_locked(START + 900, 5);
    v.revealed = Some(t.secret());
    let s = run(
        &t.seller(),
        Some(&v),
        Some(&t.btc_spent(START + 900, 1, true)),
        START + 900,
    );
    assert_eq!(s.status, Status::Completed);
    assert!(s.actions.is_empty());
}
