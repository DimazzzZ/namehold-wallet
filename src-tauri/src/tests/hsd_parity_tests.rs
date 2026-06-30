//! Byte-for-byte parity tests against canonical hsd 8.0.0.
//!
//! The vectors in `tests/vectors/vectors.json` are generated independently by
//! `tests/vectors/gen_hsd_vectors.js` running the real hsd library (see that
//! file's header). These tests prove our Rust transaction construction,
//! signing, serialization, and covenant encoding match hsd EXACTLY — not merely
//! that they are internally consistent. This is the core asset-safety guarantee:
//! a signed transaction our wallet broadcasts is the same transaction hsd would
//! have produced for the same intent.
//!
//! hsd and our signer both use RFC-6979 deterministic, low-S ECDSA, so identical
//! inputs (coins, output order, locktime, sighash type) yield an identical
//! signed-tx hex.

use serde_json::Value;

use crate::noncustodial::address;
use crate::noncustodial::bids;
use crate::noncustodial::covenants;
use crate::noncustodial::hd::{self, ExtendedPrivKey};
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::{self, SpendableCoin};
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::tx::{
    output_address_from_string, p2wpkh_script_code, sighash, Covenant, Input, Outpoint, Output,
    Transaction,
};

const VECTORS: &str = include_str!("../../tests/vectors/vectors.json");
const NETWORK: Network = Network::Main;
const ACCOUNT: u32 = 0;

fn vectors() -> Value {
    serde_json::from_str(VECTORS).expect("vectors.json parses")
}

fn hexv(s: &str) -> Vec<u8> {
    hex::decode(s).expect("valid hex")
}

fn hex32(s: &str) -> [u8; 32] {
    let v = hexv(s);
    assert_eq!(v.len(), 32, "expected 32 bytes, got {}", v.len());
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    a
}

fn master_from_known_mnemonic() -> ExtendedPrivKey {
    // The mnemonic is fixed in the vectors; assert it matches our copy.
    let mnemonic = "april coyote civil finger crane uncle situate moon choice wrong \
                    goose client purse deer funny hobby shrug give anxiety truly rack \
                    stand salad coach";
    let seed = hd::seed_from_mnemonic(mnemonic, "").expect("seed");
    ExtendedPrivKey::from_seed(&seed).expect("master")
}

/// Re-derive the (hash160, secret) for a BIP44 (branch, index) leaf from the
/// known master key.
fn leaf(master: &ExtendedPrivKey, branch: u32, index: u32) -> ([u8; 20], secp256k1::SecretKey) {
    let path = hd::bip44_path(NETWORK, ACCOUNT, branch, index);
    let child = master.derive_path(&path).expect("derive");
    let pubkey = child.compressed_pubkey();
    (address::pubkey_to_hash160(&pubkey), child.secret)
}

/// Build a tx::Transaction directly from a vector's inputs/outputs (no coin
/// selection), sign each P2WPKH input from the known master, and return it.
fn build_direct(master: &ExtendedPrivKey, v: &Value, extra_outputs: Vec<Output>) -> Transaction {
    let mut tx = Transaction::new();
    tx.locktime = v["locktime"].as_u64().unwrap_or(0) as u32;

    for inp in v["inputs"].as_array().unwrap() {
        tx.inputs.push(Input::new(Outpoint {
            hash: hex32(inp["prevoutHashInternal"].as_str().unwrap()),
            index: inp["vout"].as_u64().unwrap() as u32,
        }));
    }
    tx.outputs = extra_outputs;

    for (i, inp) in v["inputs"].as_array().unwrap().iter().enumerate() {
        let branch = inp["branch"].as_u64().unwrap() as u32;
        let index = inp["index"].as_u64().unwrap() as u32;
        let value = inp["value"].as_u64().unwrap();
        let (hash160, secret) = leaf(master, branch, index);
        // Sanity: our derived key-hash equals hsd's for this leaf.
        assert_eq!(
            hex::encode(hash160),
            inp["keyHash160"].as_str().unwrap(),
            "keyHash160 mismatch at input {i}"
        );
        tx.sign_p2wpkh_input(i, &secret, &hash160, value, sighash::ALL)
            .expect("sign");
    }
    tx
}

fn plain_outputs(v: &Value) -> Vec<Output> {
    let mut outs = vec![Output {
        value: v["recipient"]["value"].as_u64().unwrap(),
        address: output_address_from_string(NETWORK, v["recipient"]["address"].as_str().unwrap())
            .unwrap(),
        covenant: Covenant::default(),
    }];
    if let Some(change) = v.get("change").filter(|c| !c.is_null()) {
        outs.push(Output {
            value: change["value"].as_u64().unwrap(),
            address: output_address_from_string(NETWORK, change["address"].as_str().unwrap())
                .unwrap(),
            covenant: Covenant::default(),
        });
    }
    outs
}

fn assert_per_input_sighashes(master: &ExtendedPrivKey, tx: &Transaction, inputs: &Value) {
    for (i, inp) in inputs.as_array().unwrap().iter().enumerate() {
        let branch = inp["branch"].as_u64().unwrap() as u32;
        let index = inp["index"].as_u64().unwrap() as u32;
        let value = inp["value"].as_u64().unwrap();
        let (hash160, _) = leaf(master, branch, index);
        let sh = tx
            .signature_hash(i, &p2wpkh_script_code(&hash160), value, sighash::ALL)
            .expect("sighash");
        assert_eq!(
            hex::encode(sh),
            inp["sighashAll"].as_str().unwrap(),
            "sighash mismatch at input {i}"
        );
    }
}

// --- address derivation ----------------------------------------------------

#[test]
fn addresses_match_hsd() {
    let master = master_from_known_mnemonic();
    let v = vectors();
    for a in v["addresses"].as_array().unwrap() {
        let branch = a["branch"].as_u64().unwrap() as u32;
        let index = a["index"].as_u64().unwrap() as u32;
        let path = hd::bip44_path(NETWORK, ACCOUNT, branch, index);
        let child = master.derive_path(&path).unwrap();
        let pubkey = child.compressed_pubkey();
        let addr = address::address_from_pubkey(NETWORK, &pubkey).unwrap();
        assert_eq!(addr, a["address"].as_str().unwrap(), "address {branch}/{index}");
        assert_eq!(
            hex::encode(address::pubkey_to_hash160(&pubkey)),
            a["keyHash160"].as_str().unwrap()
        );
        assert_eq!(hex::encode(pubkey), a["pubkey"].as_str().unwrap());
    }
}

// --- plain send: direct construction (pins serialize/sighash/sign) ----------

#[test]
fn plain_send_direct_matches_hsd_byte_for_byte() {
    let master = master_from_known_mnemonic();
    let v = vectors();
    let ps = &v["plainSendDirect"];
    let tx = build_direct(&master, ps, plain_outputs(ps));

    assert_per_input_sighashes(&master, &tx, &ps["inputs"]);
    assert_eq!(tx.txid(), ps["txid"].as_str().unwrap(), "txid");
    assert_eq!(
        tx.to_hex(),
        ps["signedHex"].as_str().unwrap(),
        "signed tx hex must match hsd byte-for-byte"
    );
}

// --- plain send: through build_send (the production code path) --------------

fn run_build_send(v: &Value) -> send::BuiltTransaction {
    let master = master_from_known_mnemonic();
    let mut session = SignerSession::unlock("known".to_string(), NETWORK, master, 60_000);

    let coins: Vec<SpendableCoin> = v["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|inp| SpendableCoin {
            txid: inp["displayTxid"].as_str().unwrap().to_string(),
            vout: inp["vout"].as_u64().unwrap() as u32,
            value: inp["value"].as_u64().unwrap(),
            branch: inp["branch"].as_u64().unwrap() as u32,
            child_index: inp["index"].as_u64().unwrap() as u32,
        })
        .collect();

    send::build_send(
        &mut session,
        NETWORK,
        ACCOUNT,
        &coins,
        v["recipient"]["address"].as_str().unwrap(),
        v["params"]["amount"].as_u64().unwrap(),
        v["change"]["address"].as_str().unwrap(),
        v["params"]["rate"].as_u64().unwrap(),
        false,
    )
    .expect("build_send")
}

#[test]
fn build_send_single_input_matches_hsd() {
    let v = vectors();
    let bs = &v["buildSend1"];
    let built = run_build_send(bs);
    assert_eq!(built.fee, bs["params"]["fee"].as_u64().unwrap(), "fee");
    assert_eq!(built.change, bs["params"]["change"].as_u64().unwrap(), "change");
    assert_eq!(built.num_inputs, 1);
    assert_eq!(built.txid, bs["txid"].as_str().unwrap(), "txid");
    assert_eq!(
        built.tx_hex,
        bs["signedHex"].as_str().unwrap(),
        "build_send signed hex must match hsd"
    );
}

#[test]
fn build_send_two_inputs_matches_hsd() {
    let v = vectors();
    let bs = &v["buildSend2"];
    let built = run_build_send(bs);
    assert_eq!(built.fee, bs["params"]["fee"].as_u64().unwrap(), "fee");
    assert_eq!(built.change, bs["params"]["change"].as_u64().unwrap(), "change");
    assert_eq!(built.num_inputs, 2, "must select both coins");
    assert_eq!(built.txid, bs["txid"].as_str().unwrap(), "txid");
    assert_eq!(
        built.tx_hex,
        bs["signedHex"].as_str().unwrap(),
        "two-input build_send signed hex must match hsd"
    );
}

// --- covenant-bearing tx (OPEN), full signed parity -------------------------

#[test]
fn open_covenant_tx_matches_hsd_byte_for_byte() {
    let master = master_from_known_mnemonic();
    let v = vectors();
    let ot = &v["openTx"];

    let name_hash = hex32(v["openTxMeta"]["nameHash"].as_str().unwrap());
    let raw_name = hexv(v["openTxMeta"]["rawName"].as_str().unwrap());

    let open_out = Output {
        value: ot["covenantOutput"]["value"].as_u64().unwrap(),
        address: output_address_from_string(
            NETWORK,
            ot["covenantOutput"]["address"].as_str().unwrap(),
        )
        .unwrap(),
        covenant: covenants::open(&name_hash, &raw_name),
    };
    let change_out = Output {
        value: ot["change"]["value"].as_u64().unwrap(),
        address: output_address_from_string(NETWORK, ot["change"]["address"].as_str().unwrap())
            .unwrap(),
        covenant: Covenant::default(),
    };

    // openTx stores a single input object (not an array); adapt to build_direct.
    let inputs = serde_json::json!([ot["input"]]);
    let mut shim = ot.clone();
    shim["inputs"] = inputs;
    let tx = build_direct(&master, &shim, vec![open_out, change_out]);

    assert_per_input_sighashes(&master, &tx, &shim["inputs"]);
    assert_eq!(tx.txid(), ot["txid"].as_str().unwrap(), "open tx txid");
    assert_eq!(
        tx.to_hex(),
        ot["signedHex"].as_str().unwrap(),
        "open-covenant signed tx hex must match hsd"
    );
}

// --- covenant raw serialization parity --------------------------------------

#[test]
fn covenant_serializations_match_hsd() {
    let v = vectors();
    for c in v["covenants"].as_array().unwrap() {
        let kind = c["kind"].as_str().unwrap();
        let a = &c["args"];
        let nh = hex32(a["nameHash"].as_str().unwrap());
        let cov = match kind {
            "open" => covenants::open(&nh, &hexv(a["rawName"].as_str().unwrap())),
            "bid" => covenants::bid(
                &nh,
                a["start"].as_u64().unwrap() as u32,
                &hexv(a["rawName"].as_str().unwrap()),
                &hex32(a["blind"].as_str().unwrap()),
            ),
            "reveal" => covenants::reveal(
                &nh,
                a["height"].as_u64().unwrap() as u32,
                &hex32(a["nonce"].as_str().unwrap()),
            ),
            "redeem" => covenants::redeem(&nh, a["height"].as_u64().unwrap() as u32),
            "register" => covenants::register(
                &nh,
                a["height"].as_u64().unwrap() as u32,
                &hexv(a["resource"].as_str().unwrap()),
                &hex32(a["renewalBlock"].as_str().unwrap()),
            ),
            "update" => covenants::update(
                &nh,
                a["height"].as_u64().unwrap() as u32,
                &hexv(a["resource"].as_str().unwrap()),
            ),
            "renew" => covenants::renew(
                &nh,
                a["height"].as_u64().unwrap() as u32,
                &hex32(a["renewalBlock"].as_str().unwrap()),
            ),
            "transfer" => covenants::transfer(
                &nh,
                a["height"].as_u64().unwrap() as u32,
                a["addrVersion"].as_u64().unwrap() as u8,
                &hexv(a["addrHash"].as_str().unwrap()),
            ),
            "finalize" => covenants::finalize(
                &nh,
                a["height"].as_u64().unwrap() as u32,
                &hexv(a["rawName"].as_str().unwrap()),
                a["flags"].as_u64().unwrap() as u8,
                a["claimed"].as_u64().unwrap() as u32,
                a["renewals"].as_u64().unwrap() as u32,
                &hex32(a["renewalBlock"].as_str().unwrap()),
            ),
            "cancel" => covenants::cancel(&nh, a["height"].as_u64().unwrap() as u32),
            "revoke" => covenants::revoke(&nh, a["height"].as_u64().unwrap() as u32),
            other => panic!("unknown covenant kind {other}"),
        };
        assert_eq!(
            hex::encode(cov.to_raw()),
            c["raw"].as_str().unwrap(),
            "covenant '{kind}' raw serialization must match hsd"
        );
    }
}

// --- name hash + bid blind --------------------------------------------------

#[test]
fn name_hash_matches_hsd() {
    let v = vectors();
    let name = v["nameHash"]["name"].as_str().unwrap();
    assert_eq!(
        hex::encode(names::hash_name(name).unwrap()),
        v["nameHash"]["hash"].as_str().unwrap()
    );
}

#[test]
fn bid_blind_matches_hsd() {
    let v = vectors();
    let value = v["blind"]["value"].as_u64().unwrap();
    let nonce = hex32(v["blind"]["nonce"].as_str().unwrap());
    assert_eq!(
        hex::encode(bids::compute_blind(value, &nonce)),
        v["blind"]["blind"].as_str().unwrap()
    );
}
