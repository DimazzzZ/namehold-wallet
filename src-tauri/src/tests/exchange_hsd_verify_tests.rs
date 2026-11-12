//! Transactions the Rust code signs, run through hsd's own script
//! interpreter (`scripts/exchange-hsd.mjs verify`, which calls
//! `tx.verifyInput` with `STANDARD_VERIFY_FLAGS`). Spec R1, R3, R6.
//!
//! Each case says what hsd must do: accept the honest claim and refund, and
//! refuse every tampered one (a claim of another value, a secret of another
//! length, a wrong secret, an early refund, a refund with a final sequence).
//!
//! Needs `node` and hsd (`npm i -g hsd`, or `HSD_LIB`); without them every
//! test prints "skip" and passes, like the live-node suite.

use std::io::Write;
use std::process::{Command, Stdio};

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::noncustodial::exchange::hns_htlc;
use crate::noncustodial::exchange::script::{self, HtlcParams};
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::{Outpoint, Transaction};

fn script_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/exchange-hsd.mjs")
}

/// hsd's verdicts on `items`, or `None` when node or hsd is not installed.
fn hsd_verify(items: &Value) -> Option<Vec<(bool, String)>> {
    let mut child = Command::new("node")
        .arg(script_path())
        .arg("verify")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()?
        .write_all(items.to_string().as_bytes())
        .ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        eprintln!(
            "skip: exchange-hsd.mjs failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(
        v.as_array()?
            .iter()
            .map(|r| {
                (
                    r["ok"].as_bool().unwrap_or(false),
                    r["error"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect(),
    )
}

fn key(label: &str) -> SecretKey {
    SecretKey::from_byte_array(Sha256::digest(label.as_bytes()).into()).unwrap()
}

fn pubkey(k: &SecretKey) -> [u8; 33] {
    PublicKey::from_secret_key(&Secp256k1::new(), k).serialize()
}

fn payout(label: &str) -> String {
    crate::noncustodial::address::address_from_pubkey(Network::Regtest, &pubkey(&key(label)))
        .unwrap()
}

const VALUE: u64 = 25_000_000;
const FEE: u64 = 10_000;

fn params_for(secret: &[u8]) -> HtlcParams {
    HtlcParams {
        hash: Sha256::digest(secret).into(),
        claim_pubkey: pubkey(&key("verify claim")),
        refund_pubkey: pubkey(&key("verify refund")),
        refund_lock_time: hns_htlc::encode_time_lock(1_790_000_000).unwrap(),
    }
}

fn lock_at() -> Outpoint {
    Outpoint {
        hash: [0x31; 32],
        index: 2,
    }
}

/// One verify item: `tx` spending the lock of `p` worth `value` at `lock_at`.
fn item(tx: &Transaction, p: &HtlcParams, value: u64) -> Value {
    json!({
        "txHex": tx.to_hex(),
        "coins": [{
            "hash": hex::encode(lock_at().hash),
            "index": lock_at().index,
            "value": value,
            "scriptHex": hex::encode(p.script()),
        }]
    })
}

fn secret() -> [u8; 32] {
    Sha256::digest(b"verify secret").into()
}

#[test]
fn hsd_accepts_the_claim_and_refund_and_refuses_every_tampered_spend() {
    let p = params_for(&secret());
    let claim = hns_htlc::sign_claim(
        Network::Regtest,
        &p,
        VALUE,
        &key("verify claim"),
        &payout("b"),
        FEE,
    )
    .unwrap();
    let good_claim = claim
        .finish(Network::Regtest, &p, lock_at(), &secret())
        .unwrap();
    let refund = hns_htlc::sign_refund(
        Network::Regtest,
        &p,
        lock_at(),
        VALUE,
        &key("verify refund"),
        &payout("s"),
        FEE,
    )
    .unwrap();

    // 1. A claim signed for VALUE, of a lock worth VALUE + 1.
    let wrong_value = item(&good_claim, &p, VALUE + 1);

    // 2. A wrong secret forced into the witness (finish refuses it, so the
    //    witness is set by hand).
    let mut wrong_secret = good_claim.clone();
    wrong_secret.inputs[0].witness[1] = vec![0x55; 32];

    // 3. A lock whose hash is of a 33-byte secret: OP_SIZE 32 refuses it
    //    even though SHA-256 matches.
    let long_secret = [0x66u8; 33];
    let p33 = params_for(&long_secret);
    let c33 = hns_htlc::sign_claim(
        Network::Regtest,
        &p33,
        VALUE,
        &key("verify claim"),
        &payout("b"),
        FEE,
    )
    .unwrap();
    // c33 signs the same output as `good_claim` (NOINPUT ignores the
    // outpoint), so its signature fits good_claim's transaction.
    let mut long = good_claim.clone();
    long.inputs[0].witness = vec![
        hex::decode(&c33.signature_hex).unwrap(),
        long_secret.to_vec(),
        vec![0x01],
        p33.script(),
    ];

    // 4. A refund whose transaction lock time is below the script's.
    let mut early = refund.clone();
    early.locktime = p.refund_lock_time - 1;
    let sig = early
        .sign_p2wsh_input(0, &key("verify refund"), &p.script(), VALUE, 1)
        .unwrap();
    early.inputs[0].witness = script::refund_witness(&sig, &p.script());

    // 5. A refund with a final sequence (CLTV is then refused).
    let mut final_seq = refund.clone();
    final_seq.inputs[0].sequence = 0xffff_ffff;
    let sig = final_seq
        .sign_p2wsh_input(0, &key("verify refund"), &p.script(), VALUE, 1)
        .unwrap();
    final_seq.inputs[0].witness = script::refund_witness(&sig, &p.script());

    // 6. Someone else's key on the refund path.
    let mut thief = refund.clone();
    let sig = thief
        .sign_p2wsh_input(0, &key("thief"), &p.script(), VALUE, 1)
        .unwrap();
    thief.inputs[0].witness = script::refund_witness(&sig, &p.script());

    let items = json!([
        item(&good_claim, &p, VALUE),
        item(&refund, &p, VALUE),
        wrong_value,
        item(&wrong_secret, &p, VALUE),
        item(&long, &p33, VALUE),
        item(&early, &p, VALUE),
        item(&final_seq, &p, VALUE),
        item(&thief, &p, VALUE),
    ]);
    let Some(verdicts) = hsd_verify(&items) else {
        eprintln!("skip: node or hsd not installed");
        return;
    };
    let names = [
        "honest claim",
        "honest refund",
        "claim of another value",
        "wrong secret",
        "33-byte secret",
        "early refund",
        "refund with final sequence",
        "refund signed by another key",
    ];
    assert_eq!(verdicts.len(), names.len());
    assert!(verdicts[0].0, "honest claim refused: {}", verdicts[0].1);
    assert!(verdicts[1].0, "honest refund refused: {}", verdicts[1].1);
    for (i, (ok, err)) in verdicts.iter().enumerate().skip(2) {
        assert!(!ok, "hsd accepted the {}", names[i]);
        eprintln!("{}: refused ({err})", names[i]);
    }
}

#[test]
fn the_same_presigned_claim_spends_a_lock_at_any_outpoint() {
    let p = params_for(&secret());
    let claim = hns_htlc::sign_claim(
        Network::Regtest,
        &p,
        VALUE,
        &key("verify claim"),
        &payout("b"),
        FEE,
    )
    .unwrap();
    let mut items = Vec::new();
    for (h, i) in [([0x01u8; 32], 0u32), ([0xab; 32], 7), ([0xfe; 32], 1)] {
        let at = Outpoint { hash: h, index: i };
        let tx = claim
            .finish(Network::Regtest, &p, at.clone(), &secret())
            .unwrap();
        items.push(json!({
            "txHex": tx.to_hex(),
            "coins": [{ "hash": hex::encode(h), "index": i, "value": VALUE, "scriptHex": hex::encode(p.script()) }]
        }));
    }
    let Some(verdicts) = hsd_verify(&Value::Array(items)) else {
        eprintln!("skip: node or hsd not installed");
        return;
    };
    for (ok, err) in verdicts {
        assert!(ok, "refused: {err}");
    }
}
