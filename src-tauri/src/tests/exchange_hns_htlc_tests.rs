//! The HNS lock's pure rules (spec R1–R3, R6): the time lock's rounding and
//! boundary, exact-script recognition, the presigned claim's checks, and
//! secret extraction.

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use crate::noncustodial::exchange::hns_htlc::{self, LOCKTIME_FLAG};
use crate::noncustodial::exchange::script::{self, HtlcParams};
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::Outpoint;

fn key(label: &str) -> SecretKey {
    SecretKey::from_byte_array(Sha256::digest(label.as_bytes()).into()).unwrap()
}

fn pubkey(k: &SecretKey) -> [u8; 33] {
    PublicKey::from_secret_key(&Secp256k1::new(), k).serialize()
}

fn secret() -> [u8; 32] {
    Sha256::digest(b"test secret").into()
}

fn params() -> HtlcParams {
    HtlcParams {
        hash: Sha256::digest(secret()).into(),
        claim_pubkey: pubkey(&key("claim")),
        refund_pubkey: pubkey(&key("refund")),
        refund_lock_time: hns_htlc::encode_time_lock(1_800_000_000).unwrap(),
    }
}

fn payout() -> String {
    crate::noncustodial::address::address_from_pubkey(Network::Regtest, &pubkey(&key("payout")))
        .unwrap()
}

fn outpoint() -> Outpoint {
    Outpoint {
        hash: [9u8; 32],
        index: 0,
    }
}

#[test]
fn time_lock_rounds_up_to_the_next_512_seconds() {
    assert_eq!(hns_htlc::encode_time_lock(512).unwrap(), LOCKTIME_FLAG | 1);
    assert_eq!(hns_htlc::encode_time_lock(513).unwrap(), LOCKTIME_FLAG | 2);
    assert_eq!(hns_htlc::encode_time_lock(1024).unwrap(), LOCKTIME_FLAG | 2);
    // Never earlier than asked.
    for t in [1_700_000_000u64, 1_700_000_001, 1_799_999_999] {
        let lock = hns_htlc::encode_time_lock(t).unwrap();
        assert!(hns_htlc::time_lock_secs(lock).unwrap() >= t);
        assert!(hns_htlc::time_lock_secs(lock).unwrap() < t + 512);
    }
}

#[test]
fn time_lock_refuses_what_hsd_cannot_encode() {
    assert!(hns_htlc::encode_time_lock(0).is_err());
    assert!(hns_htlc::encode_time_lock(u64::from(hns_htlc::LOCKTIME_MASK) * 512 + 1).is_err());
}

#[test]
fn time_lock_opens_only_when_the_median_time_is_strictly_past() {
    let lock = hns_htlc::encode_time_lock(1_800_000_000).unwrap();
    let at = hns_htlc::time_lock_secs(lock).unwrap();
    // hsd isFinal: locktime * 512 < median time past.
    assert!(!hns_htlc::time_lock_open(lock, at - 1));
    assert!(!hns_htlc::time_lock_open(lock, at));
    assert!(hns_htlc::time_lock_open(lock, at + 1));
    // A height lock is never read as a time.
    assert!(!hns_htlc::time_lock_open(5000, u64::MAX));
    assert_eq!(hns_htlc::time_lock_secs(5000), None);
}

#[test]
fn script_num_is_minimal() {
    assert_eq!(script::script_num(0), Vec::<u8>::new());
    assert_eq!(script::script_num(0x7f), vec![0x7f]);
    assert_eq!(script::script_num(0x80), vec![0x80, 0x00]);
    assert_eq!(script::script_num(0xff), vec![0xff, 0x00]);
    assert_eq!(script::script_num(0x100), vec![0x00, 0x01]);
    assert_eq!(
        script::script_num(LOCKTIME_FLAG | 5),
        vec![0x05, 0x00, 0x00, 0x80, 0x00]
    );
}

#[test]
fn a_script_one_byte_off_is_not_the_lock() {
    let p = params();
    let good = p.script();
    assert!(p.matches(&good));
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0x01;
        assert!(!p.matches(&bad), "byte {i}");
    }
    let mut longer = good.clone();
    longer.push(0x00);
    assert!(!p.matches(&longer));
}

#[test]
fn params_refuse_equal_keys_a_zero_time_and_a_bad_point() {
    let mut p = params();
    p.refund_pubkey = p.claim_pubkey;
    assert!(p.check().is_err());
    let mut p = params();
    p.refund_lock_time = 0;
    assert!(p.check().is_err());
    let mut p = params();
    p.claim_pubkey = [2u8; 33];
    p.claim_pubkey[1..].copy_from_slice(&[0xff; 32]);
    assert!(p.check().is_err());
    assert!(params().check().is_ok());
}

#[test]
fn a_claim_is_signed_only_with_the_locks_claim_key() {
    let err = hns_htlc::sign_claim(
        Network::Regtest,
        &params(),
        1_000_000,
        &key("refund"),
        &payout(),
        1000,
    )
    .unwrap_err();
    assert!(err.to_string().contains("claim key"), "{err}");
}

#[test]
fn a_refund_is_signed_only_with_the_locks_refund_key() {
    let err = hns_htlc::sign_refund(
        Network::Regtest,
        &params(),
        outpoint(),
        1_000_000,
        &key("claim"),
        &payout(),
        1000,
    )
    .unwrap_err();
    assert!(err.to_string().contains("refund key"), "{err}");
}

#[test]
fn a_fee_of_the_whole_lock_is_refused() {
    assert!(hns_htlc::sign_claim(
        Network::Regtest,
        &params(),
        1000,
        &key("claim"),
        &payout(),
        1000
    )
    .is_err());
    assert!(hns_htlc::sign_refund(
        Network::Regtest,
        &params(),
        outpoint(),
        1000,
        &key("refund"),
        &payout(),
        5000
    )
    .is_err());
}

#[test]
fn a_presigned_claim_verifies_and_fails_once_tampered() {
    let p = params();
    let c = hns_htlc::sign_claim(
        Network::Regtest,
        &p,
        5_000_000,
        &key("claim"),
        &payout(),
        1000,
    )
    .unwrap();
    assert!(c.verify(Network::Regtest, &p));
    assert_eq!(c.to_value, 4_999_000);

    let mut more = c.clone();
    more.to_value += 1;
    assert!(!more.verify(Network::Regtest, &p), "output value is signed");

    let mut other_value = c.clone();
    other_value.lock_value += 1;
    assert!(
        !other_value.verify(Network::Regtest, &p),
        "lock value is signed"
    );

    let mut elsewhere = c.clone();
    elsewhere.to_address =
        crate::noncustodial::address::address_from_pubkey(Network::Regtest, &pubkey(&key("thief")))
            .unwrap();
    assert!(
        !elsewhere.verify(Network::Regtest, &p),
        "destination is signed"
    );

    let mut other_lock = p.clone();
    other_lock.refund_lock_time += 1;
    assert!(!c.verify(Network::Regtest, &other_lock), "script is signed");
}

#[test]
fn finish_puts_the_secret_in_and_refuses_a_wrong_one() {
    let p = params();
    let c = hns_htlc::sign_claim(
        Network::Regtest,
        &p,
        5_000_000,
        &key("claim"),
        &payout(),
        1000,
    )
    .unwrap();
    let tx = c
        .finish(Network::Regtest, &p, outpoint(), &secret())
        .unwrap();
    assert_eq!(tx.inputs[0].witness.len(), 4);
    assert_eq!(tx.inputs[0].witness[1], secret().to_vec());
    assert_eq!(hns_htlc::secret_from_spend(&tx, 0, &p), Some(secret()));

    let err = c
        .finish(Network::Regtest, &p, outpoint(), &[0u8; 32])
        .unwrap_err();
    assert!(err.to_string().contains("secret"), "{err}");
}

#[test]
fn a_refund_carries_no_secret() {
    let p = params();
    let tx = hns_htlc::sign_refund(
        Network::Regtest,
        &p,
        outpoint(),
        5_000_000,
        &key("refund"),
        &payout(),
        1000,
    )
    .unwrap();
    assert_eq!(tx.locktime, p.refund_lock_time);
    assert_eq!(tx.inputs[0].sequence, hns_htlc::REFUND_SEQUENCE);
    assert_eq!(hns_htlc::secret_from_spend(&tx, 0, &p), None);
}

#[test]
fn a_witness_for_another_script_or_hash_yields_no_secret() {
    let p = params();
    let mut other = p.clone();
    other.hash = [7u8; 32];
    let w = script::claim_witness(&[0u8; 65], &secret(), &other.script());
    assert_eq!(script::secret_from_claim_witness(&p, &w), None);
    // The right script but a secret that does not hash to H.
    let w = script::claim_witness(&[0u8; 65], &[1u8; 32], &p.script());
    assert_eq!(script::secret_from_claim_witness(&p, &w), None);
    let w = script::claim_witness(&[0u8; 65], &secret(), &p.script());
    assert_eq!(script::secret_from_claim_witness(&p, &w), Some(secret()));
}

#[test]
fn the_lock_output_matches_only_its_value_and_program() {
    let p = params();
    let out = hns_htlc::lock_output(&p, 777);
    assert!(hns_htlc::is_lock_output(&out, &p, 777));
    assert!(!hns_htlc::is_lock_output(&out, &p, 778));
    let mut other = p.clone();
    other.claim_pubkey = pubkey(&key("someone"));
    assert!(!hns_htlc::is_lock_output(&out, &other, 777));
}

#[test]
fn fees_are_sized_on_the_real_witness() {
    let p = params();
    let claim = hns_htlc::claim_fee(Network::Regtest, &p, &payout(), 1).unwrap();
    let refund = hns_htlc::refund_fee(Network::Regtest, &p, &payout(), 1).unwrap();
    let c = hns_htlc::sign_claim(
        Network::Regtest,
        &p,
        5_000_000,
        &key("claim"),
        &payout(),
        claim,
    )
    .unwrap();
    let tx = c
        .finish(Network::Regtest, &p, outpoint(), &secret())
        .unwrap();
    assert_eq!(tx.vsize(), claim);
    let r = hns_htlc::sign_refund(
        Network::Regtest,
        &p,
        outpoint(),
        5_000_000,
        &key("refund"),
        &payout(),
        refund,
    )
    .unwrap();
    assert_eq!(r.vsize(), refund);
}

// ------------------------------------------------ the node observer, C1

mod observer {
    use super::super::exchange_fixture as fx;
    use crate::exchange_io::hns_observe::{seen_from, NodeTx};
    use crate::noncustodial::exchange::engine::LockRef;
    use crate::noncustodial::exchange::hns_htlc;
    use crate::noncustodial::tx::{Input, Outpoint, Transaction};

    fn pay_lock(t: &crate::noncustodial::exchange::terms::Terms, tag: u8) -> Transaction {
        let mut tx = Transaction::new();
        tx.inputs.push(Input::new(Outpoint {
            hash: [tag; 32],
            index: 0,
        }));
        tx.outputs.push(hns_htlc::lock_output(
            &t.hns_params().unwrap(),
            t.hns_amount,
        ));
        tx
    }

    #[test]
    fn a_decoy_coin_cannot_blind_the_seller_to_the_claim_of_its_own() {
        let f = fx::trade();
        let t = &f.terms;
        let decoy = pay_lock(t, 1); // mined first, by the Buyer
        let own = pay_lock(t, 2); // the Seller's lock
        let own_ref = LockRef {
            txid: own.txid(),
            vout: 0,
        };
        let mut h = [0u8; 32];
        hex::decode_to_slice(own.txid(), &mut h).unwrap();
        let claim = f
            .buyer
            .hns_claim
            .finish(
                fx::NET,
                &t.hns_params().unwrap(),
                Outpoint { hash: h, index: 0 },
                &f.secret(),
            )
            .unwrap();
        let txs = vec![
            NodeTx {
                tx: decoy.clone(),
                height: Some(10),
            },
            NodeTx {
                tx: own.clone(),
                height: Some(12),
            },
            NodeTx {
                tx: claim.clone(),
                height: Some(13),
            },
        ];
        // The Seller watches its own coin and sees it claimed.
        let v = seen_from(t, &txs, 13, 0, Some(&own_ref), true).unwrap();
        let lock = v.lock.unwrap();
        assert_eq!(lock.outpoint, own_ref);
        assert_eq!(lock.spend.unwrap().secret, Some(f.secret()));
        assert_eq!(v.revealed, Some(f.secret()));
        // Without its own coin (the Buyer's view) the earliest is chosen,
        // and the secret is still found.
        let v = seen_from(t, &txs, 13, 0, None, false).unwrap();
        assert_eq!(v.lock.unwrap().outpoint.txid, decoy.txid());
        assert_eq!(v.revealed, Some(f.secret()));
    }

    #[test]
    fn the_sellers_own_coin_not_yet_seen_is_no_lock() {
        let f = fx::trade();
        let decoy = pay_lock(&f.terms, 1);
        let txs = vec![NodeTx {
            tx: decoy,
            height: Some(10),
        }];
        let own = LockRef {
            txid: "11".repeat(32),
            vout: 0,
        };
        let v = seen_from(&f.terms, &txs, 10, 0, Some(&own), true).unwrap();
        assert!(v.lock.is_none());
        assert!(v.revealed.is_none());
    }
}
