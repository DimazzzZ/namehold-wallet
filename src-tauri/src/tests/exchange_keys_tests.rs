//! Swap keys, the secret, the fee ladder and the Commits' refusals (spec
//! R12, R14).

use super::exchange_fixture::{self as fx, START};
use crate::noncustodial::exchange::commit;
use crate::noncustodial::exchange::fees;
use crate::noncustodial::exchange::keys;
use crate::noncustodial::hd::HARDENED_OFFSET;
use crate::noncustodial::network::Network;

#[test]
fn the_stored_branch_key_gives_the_same_public_keys_as_the_phrase() {
    let m = fx::buyer_master();
    let xpub = keys::branch_xpub(&m, Network::Main, 0).unwrap();
    for i in [0u32, 1, 7, 1_000_000] {
        let (h, b) = keys::trade_pubkeys(&xpub, i).unwrap();
        let k = keys::trade_keys(&m, Network::Main, 0, i).unwrap();
        assert_eq!(h, k.hns_pubkey);
        assert_eq!(b, k.btc_pubkey);
        assert_ne!(h, b);
    }
}

#[test]
fn swap_keys_are_on_their_own_branch() {
    assert_eq!(
        keys::swap_branch_path(Network::Main, 0).unwrap(),
        [
            HARDENED_OFFSET + 44,
            HARDENED_OFFSET + 5353,
            HARDENED_OFFSET,
            HARDENED_OFFSET + 3
        ]
    );
    assert!(keys::swap_branch_path(Network::Main, HARDENED_OFFSET).is_err());
    assert!(keys::trade_keys(&fx::buyer_master(), Network::Main, 0, HARDENED_OFFSET).is_err());
    // Not a receive, change or lock key.
    let m = fx::buyer_master();
    let k = keys::trade_keys(&m, Network::Main, 0, 0).unwrap();
    let recv = m
        .derive_path(&crate::noncustodial::hd::bip44_path(Network::Main, 0, 0, 0))
        .unwrap()
        .compressed_pubkey();
    assert_ne!(k.hns_pubkey, recv);
}

#[test]
fn the_secret_is_stable_per_trade_and_differs_between_trades_and_wallets() {
    let m = fx::buyer_master();
    let a = keys::trade_keys(&m, Network::Main, 0, 5).unwrap();
    let b = keys::trade_keys(&m, Network::Main, 0, 5).unwrap();
    let c = keys::trade_keys(&m, Network::Main, 0, 6).unwrap();
    let d = keys::trade_keys(&fx::seller_master(), Network::Main, 0, 5).unwrap();
    assert_eq!(a.secret(&[1; 32]), b.secret(&[1; 32]));
    assert_ne!(a.secret(&[1; 32]), c.secret(&[1; 32]));
    assert_ne!(a.secret(&[1; 32]), d.secret(&[1; 32]));
    use sha2::Digest;
    assert_eq!(
        a.hash(&[1; 32]),
        <[u8; 32]>::from(sha2::Sha256::digest(a.secret(&[1; 32])))
    );
    // The secret is not a public key's hash of the branch (an xpub holder
    // derives every public key, none of which gives the secret).
    let (h, btc) =
        keys::trade_pubkeys(&keys::branch_xpub(&m, Network::Main, 0).unwrap(), 5).unwrap();
    assert_ne!(&a.secret(&[1; 32])[..], &h[1..]);
    assert_ne!(&a.secret(&[1; 32])[..], &btc[1..]);
}

#[test]
fn the_ladder_rises_and_stops_at_the_fee_cap() {
    let p = fx::policy();
    let rungs = fees::btc_ladder(2, 150, 100_000, &p).unwrap();
    assert_eq!(rungs[0].rate, 2);
    assert!(rungs
        .windows(2)
        .all(|w| w[0].rate < w[1].rate && w[0].fee < w[1].fee));
    for r in &rungs {
        assert!(r.fee * 100 <= 100_000 * p.max_fee_percent, "{r:?}");
    }
    // 20 % of 100 000 = 20 000 sats; 150 vB × 2 × 64 = 19 200 fits, × 128 not.
    assert_eq!(rungs.len(), p.ladder_multipliers.len());
    let small = fees::btc_ladder(2, 150, 5_000, &p).unwrap();
    assert!(small.len() < rungs.len());
    assert!(
        fees::btc_ladder(100, 150, 5_000, &p).is_err(),
        "first rung over the cap"
    );
}

#[test]
fn the_ladder_starts_at_the_floor_rate() {
    let p = crate::noncustodial::exchange::policy::Policy::for_network(Network::Main);
    let rungs = fees::btc_ladder(0, 150, 10_000_000, &p).unwrap();
    assert_eq!(rungs[0].rate, p.btc_min_rate);
}

#[test]
fn the_rung_follows_time_and_the_deadline() {
    let p = fx::policy();
    let e = p.ladder_escalate_after;
    assert_eq!(fees::rung_now(6, None, 1000, None, &p), 0);
    assert_eq!(fees::rung_now(6, Some(1000), 1000 + e - 1, None, &p), 0);
    assert_eq!(fees::rung_now(6, Some(1000), 1000 + e, None, &p), 1);
    assert_eq!(fees::rung_now(6, Some(1000), 1000 + 100 * e, None, &p), 5);
    assert_eq!(
        fees::rung_now(6, Some(1000), 1001, Some(1001 + p.ladder_top_before), &p),
        5
    );
    assert_eq!(
        fees::rung_now(6, Some(1000), 1001, Some(1002 + p.ladder_top_before), &p),
        0
    );
    assert_eq!(fees::rung_now(0, None, 0, None, &p), 0);
}

#[test]
fn hns_presigned_spends_pay_well_above_the_node_rate() {
    let p = fx::policy();
    assert_eq!(fees::hns_presign_rate(0, &p), p.hns_min_rate);
    assert_eq!(fees::hns_presign_rate(5, &p), 5 * p.hns_rate_multiplier);
    assert_eq!(fees::hns_presign_rate(u64::MAX, &p), u64::MAX);
}

#[test]
fn a_commit_refuses_terms_that_are_not_this_wallets() {
    let t = fx::trade_at(START);
    let p = fx::policy();
    let sm = fx::seller_master();
    // The Seller's wallet cannot sign the Buyer's Commit.
    let coin = fx::btc_coin(1_000_000);
    let err = commit::buyer_commit(&commit::BuyerCommitInput {
        master: &sm,
        account: 0,
        trade_index: 0,
        secret_nonce: &fx::NONCE,
        terms: &t.terms,
        policy: &p,
        btc_coins: &[coin.0],
        btc_change_to: &fx::btc_address(&sm, 1, 0),
        btc_refund_to: &fx::btc_address(&sm, 0, 1),
        btc_rate: 2,
        hns_rate: 1,
        hns_to: &fx::hns_address(&sm, 0),
    })
    .unwrap_err();
    assert!(err.to_string().contains("not this wallet"), "{err}");
    // Another trade index of the right wallet.
    let bm = fx::buyer_master();
    let coin = fx::btc_coin(1_000_000);
    let err = commit::buyer_commit(&commit::BuyerCommitInput {
        master: &bm,
        account: 0,
        trade_index: 1,
        secret_nonce: &fx::NONCE,
        terms: &t.terms,
        policy: &p,
        btc_coins: &[coin.0],
        btc_change_to: &fx::btc_address(&bm, 1, 0),
        btc_refund_to: &fx::btc_address(&bm, 0, 1),
        btc_rate: 2,
        hns_rate: 1,
        hns_to: &fx::hns_address(&bm, 0),
    })
    .unwrap_err();
    assert!(err.to_string().contains("not this wallet"), "{err}");
}

#[test]
fn the_seller_commit_refuses_a_funding_that_does_not_pay_the_lock_exactly() {
    let t = fx::trade_at(START);
    let p = fx::policy();
    let sm = fx::seller_master();
    let mut wrong = t.hns_funding.clone();
    wrong.outputs[0].value += 1;
    let input = |f: &crate::noncustodial::tx::Transaction| {
        commit::seller_commit(&commit::SellerCommitInput {
            master: &sm,
            account: 0,
            trade_index: 0,
            terms: &t.terms,
            policy: &p,
            hns_funding: f,
            btc_lock: &t.buyer.btc_lock,
            hns_rate: 1,
            hns_refund_to: &fx::hns_address(&sm, 2),
            btc_rate: 2,
            btc_claim_to: &fx::btc_address(&sm, 0, 0),
        })
        .map(|_| ())
        .unwrap_err()
        .to_string()
    };
    assert!(input(&wrong).contains("another amount"));
    let mut twice = t.hns_funding.clone();
    twice.outputs.push(twice.outputs[0].clone());
    assert!(input(&twice).contains("twice"));
    let mut none = t.hns_funding.clone();
    none.outputs.remove(0);
    assert!(input(&none).contains("does not pay"));
}

#[test]
fn the_buyers_commit_signs_a_funding_of_exactly_the_lock() {
    let t = fx::trade_at(START);
    let bp = t.terms.btc_params().unwrap();
    assert!(crate::noncustodial::exchange::btc_htlc::is_lock_output(
        &t.buyer.funding.output[0],
        &bp,
        t.terms.btc_amount
    ));
    use bitcoin::consensus::validation::verify_transaction;
    let coin = t.btc_coin.1.clone();
    verify_transaction(&t.buyer.funding, |_| Some(coin.clone())).unwrap();
    assert!(t
        .buyer
        .hns_claim
        .verify(fx::NET, &t.terms.hns_params().unwrap()));
}

#[test]
fn the_same_trade_index_with_another_nonce_has_another_secret() {
    // A restore into an empty database hands trade indexes out again: the
    // nonce keeps the secret from repeating (a repeated secret would let
    // the later Seller open the BTC lock with the earlier public secret).
    let k = keys::trade_keys(&fx::buyer_master(), Network::Main, 0, 0).unwrap();
    assert_ne!(k.secret(&[1; 32]), k.secret(&[2; 32]));
    assert_ne!(keys::new_secret_nonce(), keys::new_secret_nonce());
}

#[test]
fn a_commit_refuses_the_wrong_nonce() {
    let t = fx::trade_at(START);
    let bm = fx::buyer_master();
    let coin = fx::btc_coin(1_000_000);
    let err = commit::buyer_commit(&commit::BuyerCommitInput {
        master: &bm,
        account: 0,
        trade_index: 0,
        secret_nonce: &[9; 32],
        terms: &t.terms,
        policy: &fx::policy(),
        btc_coins: &[coin.0],
        btc_change_to: &fx::btc_address(&bm, 1, 0),
        btc_refund_to: &fx::btc_address(&bm, 0, 1),
        btc_rate: 2,
        hns_rate: 1,
        hns_to: &fx::hns_address(&bm, 0),
    })
    .unwrap_err();
    assert!(err.to_string().contains("hash"), "{err}");
}

/// The BTC account is account 0 of the phrase whatever the profile's HNS
/// account index is (`exchange_enable` stores that account's public key and
/// the coins are at its addresses), so the Buyer's funding is signed with
/// its keys, not with the keys of a BTC account numbered like the profile.
#[test]
fn the_buyers_funding_is_signed_by_the_btc_account_whatever_the_profiles_account() {
    let bm = fx::buyer_master();
    let terms = fx::terms_of_buyer_account(START, 1);
    let coin = fx::btc_coin(1_000_000);
    let c = commit::buyer_commit(&commit::BuyerCommitInput {
        master: &bm,
        account: 1,
        trade_index: 0,
        secret_nonce: &fx::NONCE,
        terms: &terms,
        policy: &fx::policy(),
        btc_coins: std::slice::from_ref(&coin.0),
        btc_change_to: &fx::btc_address(&bm, 1, 0),
        btc_refund_to: &fx::btc_address(&bm, 0, 1),
        btc_rate: 2,
        hns_rate: 1,
        hns_to: &fx::hns_address(&bm, 0),
    })
    .unwrap();
    bitcoin::consensus::validation::verify_transaction(&c.funding, |_| Some(coin.1.clone()))
        .expect("the funding spends the BTC account's coin");
}
