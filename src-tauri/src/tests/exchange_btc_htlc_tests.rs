//! The BTC lock's spends run through Bitcoin Core's own script interpreter
//! (libbitcoinconsensus, via rust-bitcoin's `bitcoinconsensus` feature,
//! enabled for tests only). Spec R1, R5, R6.

use bitcoin::consensus::validation::verify_transaction;
use bitcoin::hashes::Hash;
use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, TxOut, Txid, Witness};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use crate::noncustodial::exchange::btc_htlc;
use crate::noncustodial::exchange::script::{self, HtlcParams};
use crate::noncustodial::network::Network;

fn key(label: &str) -> SecretKey {
    SecretKey::from_byte_array(Sha256::digest(label.as_bytes()).into()).unwrap()
}

fn pubkey(k: &SecretKey) -> [u8; 33] {
    PublicKey::from_secret_key(&Secp256k1::new(), k).serialize()
}

fn secret() -> [u8; 32] {
    Sha256::digest(b"btc secret").into()
}

const REFUND_AT: u64 = 1_800_000_000;
const VALUE: u64 = 1_000_000;

fn params_for(secret: &[u8]) -> HtlcParams {
    HtlcParams {
        hash: Sha256::digest(secret).into(),
        claim_pubkey: pubkey(&key("seller btc claim")),
        refund_pubkey: pubkey(&key("buyer btc refund")),
        refund_lock_time: btc_htlc::encode_time_lock(REFUND_AT).unwrap(),
    }
}

fn params() -> HtlcParams {
    params_for(&secret())
}

fn lock_at() -> OutPoint {
    OutPoint {
        txid: Txid::from_byte_array([0x21; 32]),
        vout: 1,
    }
}

fn payout(label: &str) -> bitcoin::Address {
    crate::noncustodial::exchange::btc_account::p2wpkh(Network::Regtest, &pubkey(&key(label)))
        .unwrap()
}

/// Bitcoin Core's verdict on `tx` spending a lock of `p` worth `value`.
fn core_verifies(tx: &Transaction, p: &HtlcParams, value: u64) -> Result<(), String> {
    let spent = TxOut {
        value: Amount::from_sat(value),
        script_pubkey: btc_htlc::lock_script_pubkey(p),
    };
    verify_transaction(tx, |o| {
        (*o == tx.input[0].previous_output).then(|| spent.clone())
    })
    .map_err(|e| format!("{e:?}"))
}

fn claim(p: &HtlcParams, fee: u64) -> btc_htlc::PresignedSpend {
    btc_htlc::sign_claim(
        p,
        lock_at(),
        VALUE,
        &key("seller btc claim"),
        &payout("seller"),
        fee,
        1,
    )
    .unwrap()
}

fn refund(p: &HtlcParams, fee: u64) -> btc_htlc::PresignedSpend {
    btc_htlc::sign_refund(
        p,
        lock_at(),
        VALUE,
        &key("buyer btc refund"),
        &payout("buyer"),
        fee,
        1,
    )
    .unwrap()
}

#[test]
fn the_claim_with_the_secret_passes_consensus() {
    let p = params();
    let tx = btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap();
    core_verifies(&tx, &p, VALUE).unwrap();
}

#[test]
fn the_refund_passes_consensus() {
    let p = params();
    let tx = btc_htlc::refund_tx(&refund(&p, 1000), &p).unwrap();
    assert_eq!(u64::from(tx.lock_time.to_consensus_u32()), REFUND_AT);
    core_verifies(&tx, &p, VALUE).unwrap();
}

#[test]
fn the_presigned_claim_without_the_secret_fails_consensus() {
    let p = params();
    let tx = btc_htlc::decode(&claim(&p, 1000).tx_hex).unwrap();
    assert!(core_verifies(&tx, &p, VALUE).is_err());
}

#[test]
fn a_wrong_secret_fails_consensus_and_is_refused_by_finish() {
    let p = params();
    assert!(btc_htlc::finish_claim(&claim(&p, 1000), &p, &[3u8; 32]).is_err());
    let mut tx = btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap();
    let mut items = tx.input[0].witness.to_vec();
    items[1] = vec![3u8; 32];
    tx.input[0].witness = Witness::from_slice(&items);
    assert!(core_verifies(&tx, &p, VALUE).is_err());
}

#[test]
fn a_secret_of_33_bytes_fails_consensus_even_when_its_hash_matches() {
    let long = [0x44u8; 33];
    let p = params_for(&long);
    let mut tx = btc_htlc::decode(&claim(&p, 1000).tx_hex).unwrap();
    let mut items = tx.input[0].witness.to_vec();
    items[1] = long.to_vec();
    tx.input[0].witness = Witness::from_slice(&items);
    let err = core_verifies(&tx, &p, VALUE).unwrap_err();
    eprintln!("33-byte secret: {err}");
}

#[test]
fn a_claim_of_another_value_fails_consensus() {
    let p = params();
    let tx = btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap();
    assert!(core_verifies(&tx, &p, VALUE + 1).is_err());
}

#[test]
fn an_early_refund_or_one_with_a_final_sequence_fails_consensus() {
    let p = params();
    // Lock time one second short of the script's.
    let mut tx = btc_htlc::refund_tx(&refund(&p, 1000), &p).unwrap();
    tx.lock_time = bitcoin::absolute::LockTime::from_consensus(p.refund_lock_time - 1);
    assert!(core_verifies(&tx, &p, VALUE).is_err());
    // Final sequence disables the lock time, which CLTV refuses.
    let mut tx = btc_htlc::refund_tx(&refund(&p, 1000), &p).unwrap();
    tx.input[0].sequence = bitcoin::Sequence::MAX;
    assert!(core_verifies(&tx, &p, VALUE).is_err());
}

#[test]
fn the_wrong_key_cannot_sign_either_path() {
    let p = params();
    assert!(btc_htlc::sign_claim(
        &p,
        lock_at(),
        VALUE,
        &key("buyer btc refund"),
        &payout("x"),
        1000,
        1
    )
    .is_err());
    assert!(btc_htlc::sign_refund(
        &p,
        lock_at(),
        VALUE,
        &key("seller btc claim"),
        &payout("x"),
        1000,
        1
    )
    .is_err());
}

#[test]
fn the_destination_and_amount_are_signed() {
    let p = params();
    let mut tx = btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap();
    tx.output[0].script_pubkey = payout("thief").script_pubkey();
    assert!(core_verifies(&tx, &p, VALUE).is_err());
    let mut tx = btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap();
    tx.output[0].value = Amount::from_sat(VALUE - 10);
    assert!(core_verifies(&tx, &p, VALUE).is_err());
}

#[test]
fn every_spend_signals_replacement() {
    let p = params();
    for tx in [
        btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap(),
        btc_htlc::refund_tx(&refund(&p, 1000), &p).unwrap(),
    ] {
        assert!(tx.is_explicitly_rbf());
    }
}

#[test]
fn the_estimated_vsize_is_never_below_the_signed_one() {
    let p = params();
    for claim_path in [true, false] {
        let est = btc_htlc::spend_vsize(&p, &payout("seller"), claim_path);
        let tx = if claim_path {
            btc_htlc::finish_claim(&claim(&p, est), &p, &secret()).unwrap()
        } else {
            btc_htlc::refund_tx(&refund(&p, est), &p).unwrap()
        };
        let real = tx.vsize() as u64;
        assert!(
            est >= real && est - real <= 1,
            "estimate {est}, real {real}"
        );
    }
}

#[test]
fn a_spend_that_leaves_dust_is_refused() {
    let p = params();
    assert!(btc_htlc::sign_claim(
        &p,
        lock_at(),
        VALUE,
        &key("seller btc claim"),
        &payout("s"),
        VALUE - 100,
        1
    )
    .is_err());
    assert!(btc_htlc::sign_claim(
        &p,
        lock_at(),
        VALUE,
        &key("seller btc claim"),
        &payout("s"),
        VALUE + 1,
        1
    )
    .is_err());
}

#[test]
fn bitcoin_time_locks_are_times_and_open_strictly_after_the_median_time() {
    assert!(btc_htlc::encode_time_lock(499_999_999).is_err());
    assert!(btc_htlc::encode_time_lock(u64::from(u32::MAX) + 1).is_err());
    let lock = btc_htlc::encode_time_lock(REFUND_AT).unwrap();
    assert!(!btc_htlc::time_lock_open(lock, REFUND_AT));
    assert!(btc_htlc::time_lock_open(lock, REFUND_AT + 1));
    assert!(
        !btc_htlc::time_lock_open(400_000, u64::MAX),
        "a height is not a time"
    );
}

#[test]
fn the_lock_is_recognised_only_by_its_exact_output() {
    let p = params();
    let out = TxOut {
        value: Amount::from_sat(VALUE),
        script_pubkey: btc_htlc::lock_script_pubkey(&p),
    };
    assert!(btc_htlc::is_lock_output(&out, &p, VALUE));
    assert!(!btc_htlc::is_lock_output(&out, &p, VALUE - 1));
    let mut other = p.clone();
    other.refund_lock_time += 1;
    assert!(!btc_htlc::is_lock_output(&out, &other, VALUE));
    let p2pkh_like = TxOut {
        value: Amount::from_sat(VALUE),
        script_pubkey: ScriptBuf::from_bytes(p.script()),
    };
    assert!(!btc_htlc::is_lock_output(&p2pkh_like, &p, VALUE));
}

#[test]
fn a_witness_secret_is_read_only_from_a_claim_of_this_lock() {
    let p = params();
    let tx = btc_htlc::finish_claim(&claim(&p, 1000), &p, &secret()).unwrap();
    let items = tx.input[0].witness.to_vec();
    assert_eq!(
        script::secret_from_claim_witness(&p, &items),
        Some(secret())
    );
    let tx = btc_htlc::refund_tx(&refund(&p, 1000), &p).unwrap();
    assert_eq!(
        script::secret_from_claim_witness(&p, &tx.input[0].witness.to_vec()),
        None
    );
}

#[test]
fn addresses_are_of_the_paired_bitcoin_network() {
    let p = params();
    assert!(btc_htlc::lock_address(Network::Main, &p)
        .to_string()
        .starts_with("bc1q"));
    assert!(btc_htlc::lock_address(Network::Regtest, &p)
        .to_string()
        .starts_with("bcrt1q"));
    assert!(btc_htlc::parse_address(Network::Main, &payout("x").to_string()).is_err());
    assert!(btc_htlc::parse_address(Network::Regtest, &payout("x").to_string()).is_ok());
}
