//! The HNS lock, its NOINPUT claim and its refund against reference vectors
//! hsd 8.0.0 produced on its own (`scripts/exchange-hsd.mjs vectors`, output
//! committed as `fixtures/exchange_hsd_vectors.json`). Spec R1–R4.
//!
//! The fixture was produced by hsd's Script, MTX and bcrypto from keys
//! derived from public labels, so it checks the Rust code against an
//! independent implementation byte for byte: script, address, time lock,
//! sighash (including "the same for another outpoint"), the deterministic
//! (RFC 6979) signature, and the whole signed transaction and its txid. hsd
//! also ran its interpreter over both spends (`claimVerifies`,
//! `refundVerifies`).

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::noncustodial::exchange::hns_htlc::{self, CLAIM_SIGHASH};
use crate::noncustodial::exchange::script::HtlcParams;
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::{Outpoint, Transaction};

const FIXTURE: &str = include_str!("fixtures/exchange_hsd_vectors.json");

fn v() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_else(|| panic!("fixture lacks {k}"))
}

fn key(label: &str) -> SecretKey {
    SecretKey::from_byte_array(Sha256::digest(label.as_bytes()).into()).unwrap()
}

fn pubkey(k: &SecretKey) -> [u8; 33] {
    PublicKey::from_secret_key(&Secp256k1::new(), k).serialize()
}

fn hns_address(k: &SecretKey) -> String {
    crate::noncustodial::address::address_from_pubkey(Network::Main, &pubkey(k)).unwrap()
}

struct Vector {
    v: Value,
    params: HtlcParams,
    claim_key: SecretKey,
    refund_key: SecretKey,
    secret: [u8; 32],
}

fn vector() -> Vector {
    let v = v();
    let labels = &v["labels"];
    let claim_key = key(s(labels, "claimKey"));
    let refund_key = key(s(labels, "refundKey"));
    let secret: [u8; 32] = Sha256::digest(s(labels, "secret").as_bytes()).into();
    let hash: [u8; 32] = Sha256::digest(secret).into();
    let refund_unix = v["refundUnix"].as_u64().unwrap();
    let params = HtlcParams {
        hash,
        claim_pubkey: pubkey(&claim_key),
        refund_pubkey: pubkey(&refund_key),
        refund_lock_time: hns_htlc::encode_time_lock(refund_unix).unwrap(),
    };
    Vector {
        v,
        params,
        claim_key,
        refund_key,
        secret,
    }
}

fn lock_outpoint(v: &Value) -> Outpoint {
    let mut hash = [0u8; 32];
    hex::decode_to_slice(s(v, "lockPrevHex"), &mut hash).unwrap();
    Outpoint {
        hash,
        index: v["lockPrevIndex"].as_u64().unwrap() as u32,
    }
}

#[test]
fn hsd_says_both_reference_spends_verify() {
    let v = v();
    assert_eq!(s(&v, "hsdVersion"), "8.0.0");
    assert_eq!(v["claimVerifies"], Value::Bool(true));
    assert_eq!(v["refundVerifies"], Value::Bool(true));
}

#[test]
fn the_lock_script_and_address_are_hsds() {
    let t = vector();
    assert_eq!(hex::encode(t.params.hash), s(&t.v, "hashHex"));
    assert_eq!(hex::encode(t.params.claim_pubkey), s(&t.v, "claimPubHex"));
    assert_eq!(hex::encode(t.params.refund_pubkey), s(&t.v, "refundPubHex"));
    assert_eq!(
        u64::from(t.params.refund_lock_time),
        t.v["lockTime"].as_u64().unwrap()
    );
    assert_eq!(hex::encode(t.params.script()), s(&t.v, "scriptHex"));
    assert_eq!(
        hns_htlc::lock_address(Network::Main, &t.params).unwrap(),
        s(&t.v, "lockAddressMain")
    );
    assert_eq!(
        hns_htlc::lock_address(Network::Regtest, &t.params).unwrap(),
        s(&t.v, "lockAddressRegtest")
    );
}

#[test]
fn the_noinput_claim_sighash_signature_and_transaction_are_hsds() {
    let t = vector();
    let value = t.v["lockValue"].as_u64().unwrap();
    let fee = t.v["fee"].as_u64().unwrap();
    let to = hns_address(&key(s(&t.v["labels"], "buyerPayout")));
    assert_eq!(to, s(&t.v, "buyerPayoutMain"));

    let claim =
        hns_htlc::sign_claim(Network::Main, &t.params, value, &t.claim_key, &to, fee).unwrap();
    assert_eq!(claim.signature_hex, s(&t.v, "claimSignatureHex"));
    assert!(claim.verify(Network::Main, &t.params));

    let tx = claim
        .finish(Network::Main, &t.params, lock_outpoint(&t.v), &t.secret)
        .unwrap();
    assert_eq!(
        hex::encode(
            tx.signature_hash(0, &t.params.script(), value, CLAIM_SIGHASH)
                .unwrap()
        ),
        s(&t.v, "claimSighashHex")
    );
    assert_eq!(tx.to_hex(), s(&t.v, "claimTxHex"));
    assert_eq!(tx.txid(), s(&t.v, "claimTxid"));
}

#[test]
fn the_claim_sighash_does_not_depend_on_the_outpoint() {
    let t = vector();
    let value = t.v["lockValue"].as_u64().unwrap();
    let fee = t.v["fee"].as_u64().unwrap();
    let to = hns_address(&key(s(&t.v["labels"], "buyerPayout")));
    let claim =
        hns_htlc::sign_claim(Network::Main, &t.params, value, &t.claim_key, &to, fee).unwrap();
    let other = claim
        .finish(
            Network::Main,
            &t.params,
            Outpoint {
                hash: [0x77; 32],
                index: 9,
            },
            &t.secret,
        )
        .unwrap();
    assert_eq!(
        hex::encode(
            other
                .signature_hash(0, &t.params.script(), value, CLAIM_SIGHASH)
                .unwrap()
        ),
        s(&t.v, "claimSighashOtherOutpointHex")
    );
    assert_eq!(
        s(&t.v, "claimSighashOtherOutpointHex"),
        s(&t.v, "claimSighashHex")
    );
}

#[test]
fn the_refund_is_hsds() {
    let t = vector();
    let value = t.v["lockValue"].as_u64().unwrap();
    let fee = t.v["fee"].as_u64().unwrap();
    let to = hns_address(&key(s(&t.v["labels"], "sellerPayout")));
    assert_eq!(to, s(&t.v, "sellerPayoutMain"));
    let tx: Transaction = hns_htlc::sign_refund(
        Network::Main,
        &t.params,
        lock_outpoint(&t.v),
        value,
        &t.refund_key,
        &to,
        fee,
    )
    .unwrap();
    assert_eq!(
        hex::encode(tx.signature_hash(0, &t.params.script(), value, 1).unwrap()),
        s(&t.v, "refundSighashHex")
    );
    assert_eq!(
        hex::encode(&tx.inputs[0].witness[0]),
        s(&t.v, "refundSignatureHex")
    );
    assert_eq!(tx.to_hex(), s(&t.v, "refundTxHex"));
    assert_eq!(tx.txid(), s(&t.v, "refundTxid"));
}
