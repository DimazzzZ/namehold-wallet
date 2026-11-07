//! Terms, prices and the policy (spec §3.5, R7).

use super::exchange_fixture::{self as fx, START};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::price;
use crate::noncustodial::exchange::terms::{OfferView, Terms};
use crate::noncustodial::network::Network;

fn offer(t: &Terms) -> OfferView<'_> {
    OfferView {
        offer_id: &t.offer_id,
        network: &t.network,
        price_msat_per_hns: t.price_msat_per_hns,
        min_hns: 1_000_000,
        max_hns: 10_000 * 1_000_000,
    }
}

#[test]
fn every_network_policy_keeps_its_safety_invariants() {
    for n in Network::ALL {
        let p = Policy::for_network(n);
        assert_eq!(p.broken_invariant(), None, "{n:?}");
    }
}

#[test]
fn mainnet_policy_is_the_specs() {
    let p = Policy::for_network(Network::Main);
    assert_eq!(p.buyer_fund_window, 3600);
    assert_eq!(p.seller_fund_window, 4 * 3600);
    assert_eq!(p.hns_refund_after, 12 * 3600);
    assert_eq!(p.reveal_margin, 3 * 3600);
    assert_eq!(p.btc_refund_after_hns, 12 * 3600);
    assert_eq!((p.min_btc_sats, p.max_btc_sats), (50_000, 10_000_000));
    assert_eq!(p.btc_confirmations(1_999_999), 2);
    assert_eq!(p.btc_confirmations(2_000_000), 3);
    assert_eq!(p.hns_confirmations(1_999_999), 6);
    assert_eq!(p.hns_confirmations(2_000_000), 12);
}

#[test]
fn a_broken_policy_is_detected() {
    let mut p = Policy::for_network(Network::Main);
    p.hns_refund_after = p.seller_fund_window + p.reveal_margin;
    assert!(p.broken_invariant().is_some());
    let mut p = Policy::for_network(Network::Main);
    p.btc_refund_after_hns = p.reveal_margin;
    assert!(p.broken_invariant().is_some());
}

#[test]
fn honest_terms_pass() {
    let t = fx::terms_at(START);
    t.check(&fx::policy(), &offer(&t), START + 10).unwrap();
    assert_eq!(t.btc_amount, 100_000);
    assert_eq!(
        Terms::parse(&String::from_utf8(t.canonical()).unwrap()).unwrap(),
        t
    );
    assert_eq!(t.id().len(), 64);
}

fn refused(t: &Terms, now: u64) -> String {
    t.check(&fx::policy(), &offer(&fx::terms_at(START)), now)
        .unwrap_err()
        .to_string()
}

#[test]
fn terms_with_other_times_are_refused() {
    let mut t = fx::terms_at(START);
    t.hns_refund_time += 1;
    assert!(refused(&t, START).contains("windows"));
    let mut t = fx::terms_at(START);
    t.btc_refund_time -= 60;
    assert!(refused(&t, START).contains("windows"));
    let mut t = fx::terms_at(START);
    t.reveal_deadline += 60;
    assert!(refused(&t, START).contains("windows"));
}

#[test]
fn terms_far_from_the_clock_are_refused() {
    let t = fx::terms_at(START);
    let skew = fx::policy().clock_skew;
    assert!(refused(&t, START + skew + 1).contains("clock"));
    assert!(refused(&t, START - skew - 1).contains("clock"));
    t.check(&fx::policy(), &offer(&t), START + skew).unwrap();
}

#[test]
fn terms_off_the_offers_price_or_amounts_are_refused() {
    let mut t = fx::terms_at(START);
    t.btc_amount -= 1;
    assert!(refused(&t, START).contains("BTC amount"));
    let mut t = fx::terms_at(START);
    t.price_msat_per_hns += 1;
    assert!(refused(&t, START).contains("price"));
    let mut t = fx::terms_at(START);
    t.hns_amount = 20_000 * 1_000_000;
    t.btc_amount = price::btc_for(t.hns_amount, t.price_msat_per_hns).unwrap();
    assert!(refused(&t, START).contains("outside the offer"));
}

#[test]
fn terms_outside_the_limits_are_refused() {
    let mut p = fx::policy();
    p.max_btc_sats = 50_000;
    let t = fx::terms_at(START);
    let e = t.check(&p, &offer(&t), START).unwrap_err().to_string();
    assert!(e.contains("largest"), "{e}");
    let mut p = fx::policy();
    p.min_btc_sats = 200_000;
    let e = t.check(&p, &offer(&t), START).unwrap_err().to_string();
    assert!(e.contains("smallest"), "{e}");
}

#[test]
fn terms_with_too_few_confirmations_or_a_repeated_key_are_refused() {
    let mut p = fx::policy();
    p.hns_confirmations_small = 3;
    let t = fx::terms_at(START);
    assert!(t
        .check(&p, &offer(&t), START)
        .unwrap_err()
        .to_string()
        .contains("confirmation"));
    let mut t = fx::terms_at(START);
    t.seller_btc_claim_pubkey = t.buyer_btc_refund_pubkey.clone();
    assert!(refused(&t, START).contains("same"));
    let mut t = fx::terms_at(START);
    t.hash = "00".repeat(31);
    assert!(refused(&t, START).contains("hex"));
}

#[test]
fn terms_with_unknown_fields_or_another_network_are_refused() {
    let t = fx::terms_at(START);
    let mut v: serde_json::Value = serde_json::from_slice(&t.canonical()).unwrap();
    v["extra"] = serde_json::json!(1);
    assert!(Terms::parse(&v.to_string()).is_err());
    let mut t2 = t.clone();
    t2.network = "main".into();
    assert!(refused(&t2, START).contains("network"));
}

#[test]
fn prices_convert_exactly_and_round_down() {
    assert_eq!(price::btc_for(1_000_000, 20_000).unwrap(), 20); // 1 HNS at 20 sats
    assert_eq!(price::btc_for(1, 1).unwrap(), 0);
    assert_eq!(price::btc_for(1_500_000, 1_000).unwrap(), 1); // 1.5 sat → 1
    assert!(price::btc_for(u64::MAX, u64::MAX).is_err());
    assert_eq!(price::parse_sats_per_hns("12").unwrap(), 12_000);
    assert_eq!(price::parse_sats_per_hns("12.5").unwrap(), 12_500);
    assert_eq!(price::parse_sats_per_hns(".125").unwrap(), 125);
    for bad in ["", ".", "0", "1.2345", "-1", "1e3", "1,5", "abc"] {
        assert!(price::parse_sats_per_hns(bad).is_err(), "{bad}");
    }
    assert_eq!(price::format_sats_per_hns(12_500), "12.5");
    assert_eq!(price::format_sats_per_hns(12_000), "12");
    assert_eq!(price::format_sats_per_hns(125), "0.125");
}

#[test]
fn terms_asking_for_more_confirmations_than_the_policy_are_refused() {
    // Review L6: more confirmations would keep the other side's coins
    // locked until the refund times.
    let mut t = fx::terms_at(START);
    t.btc_confirmations += 100;
    assert!(refused(&t, START).contains("confirmation"));
}

#[test]
fn a_watch_job_with_far_future_times_is_refused_by_the_watchtower() {
    // Review M2: junk terms must not occupy a watchtower forever.
    let f = fx::trade_at(START);
    let job = crate::noncustodial::exchange::watch::WatchJob::from_holdings(&f.buyer());
    assert!(job.check_fresh(START).is_ok());
    assert!(
        job.check_fresh(START - 3600).is_err(),
        "starts an hour ahead"
    );
    assert!(job.check_fresh(START + 10 * 86_400).is_err(), "long over");
}
