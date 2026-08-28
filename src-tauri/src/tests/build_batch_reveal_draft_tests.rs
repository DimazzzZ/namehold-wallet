//! Tests for `build_batch_reveal_draft_inner` — the pure inner logic of the
//! `build_batch_reveal_draft` Tauri command.
//!
//! The inner takes a pre-resolved `Vec<(name, name_hash, bid_commitment,
//! bid_coin, state)>` (the wrapper's per-name bid-commitment / BID-coin lookup
//! + `fetch_name_state` RPC), parses each stored 32-byte nonce, builds one
//! REVEAL covenant output per name, and persists a single batch draft.

use std::collections::HashMap;

use crate::commands::names::{build_batch_reveal_draft_inner, Ctx, NameState};
use crate::db;
use crate::db::queries::{BidCommitmentRow, NameCoin};
use crate::error::AppError;
use crate::noncustodial::derivation::{self, BRANCH_CHANGE};
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::SpendableCoin;
use crate::noncustodial::sync;

const PROFILE: &str = "test_profile";

const GENERATOR_PUBKEY: [u8; 33] = [
    0x02, 0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE, 0x87, 0x0B,
    0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81, 0x5B, 0x16, 0xF8, 0x17,
    0x98,
];

fn test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn test_xpub() -> ExtendedPubKey {
    ExtendedPubKey::from_parts(&GENERATOR_PUBKEY, &[7u8; 32]).unwrap()
}

fn seed_profile(conn: &rusqlite::Connection) {
    db::queries::insert_wallet_profile(
        conn,
        PROFILE,
        "Test",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
}

fn seed_liquid_coin(conn: &rusqlite::Connection, txid: &str, value: i64, address: &str) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, 'deadbeef', ?4, 0, NULL, 'liquid_hns', NULL)",
        rusqlite::params![txid, PROFILE, address, value],
    )
    .unwrap();
}

fn seed_bid_coin(conn: &rusqlite::Connection, name: &str, txid: &str, value: i64, address: &str) {
    let nh_hex = hex::encode(names::hash_name(name).unwrap());
    let cov_json = format!(r#"{{"type":{},"items":["{}"]}}"#, sync::COV_BID, nh_hex);
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, 'deadbeef', ?4, ?5, ?6, 'name_lockup', NULL)",
        rusqlite::params![
            txid,
            PROFILE,
            address,
            value,
            sync::COV_BID as i64,
            cov_json
        ],
    )
    .unwrap();
}

fn bid_row(name: &str, addr: &str, nonce_hex: &str) -> BidCommitmentRow {
    BidCommitmentRow {
        name: name.into(),
        name_hash_hex: hex::encode(names::hash_name(name).unwrap()),
        address: addr.into(),
        branch: 0,
        child_index: 0,
        bid_value_doos: 1_000_000,
        lockup_value_doos: 2_000_000,
        nonce_hex: nonce_hex.into(),
        blind_hex: "beef".into(),
        bid_txid: None,
        reveal_txid: None,
        reveal_end_height: None,
    }
}

fn bid_coin(name: &str, txid: &str, addr: &str, value: u64) -> NameCoin {
    let nh_hex = hex::encode(names::hash_name(name).unwrap());
    let cov_json = format!(r#"{{"type":{},"items":["{}"]}}"#, sync::COV_BID, nh_hex);
    NameCoin {
        txid: txid.into(),
        vout: 0,
        value,
        address: addr.into(),
        branch: derivation::BRANCH_RECEIVE,
        child_index: 0,
        covenant_type: sync::COV_BID as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    }
}

fn name_state() -> NameState {
    NameState {
        height: 1000,
        value: 0,
        renewals: 0,
        claimed: 0,
        weak: false,
        phase: "REVEAL".into(),
    }
}

type PerName = Vec<(String, [u8; 32], BidCommitmentRow, NameCoin, NameState)>;

/// Setup with the given (name, nonce_hex) pairs. Returns conn, ctx, per_name.
fn setup(names_in: &[(&str, String)]) -> (rusqlite::Connection, Ctx, PerName) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, 50_000_000, &recv0.address);

    let mut per_name: PerName = Vec::new();
    for (i, (name, nonce_hex)) in names_in.iter().enumerate() {
        let txid = format!("{:02x}", i + 0xc0).repeat(32);
        seed_bid_coin(&conn, name, &txid, 2_000_000, &recv0.address);
        let nh = names::hash_name(name).unwrap();
        per_name.push((
            name.to_string(),
            nh,
            bid_row(name, &recv0.address, nonce_hex),
            bid_coin(name, &txid, &recv0.address, 2_000_000),
            name_state(),
        ));
    }

    let ctx = Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub,
        change_address: change.address,
        funding: vec![SpendableCoin {
            txid: funding_txid,
            vout: 0,
            value: 50_000_000,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    };
    (conn, ctx, per_name)
}

#[test]
fn batch_reveal_happy_path_persists_draft() {
    let (conn, ctx, per_name) = setup(&[("alpha", "11".repeat(32)), ("bravo", "22".repeat(32))]);

    let summary = build_batch_reveal_draft_inner(&conn, &ctx, per_name, 10).unwrap();
    assert_eq!(summary.action, "batch-reveal");

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 1);

    let reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_BID as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 2, "both BID coins reserved as reveal name inputs");
}

#[test]
fn batch_reveal_empty_persists_zero_input_draft() {
    // Wrapper rejects empty `names` up front; inner delegates to
    // `build_batch_plan`, which refuses zero-output batches.
    let (conn, ctx, _per_name) = setup(&[]);
    let err = build_batch_reveal_draft_inner(&conn, &ctx, Vec::new(), 10).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("at least one output"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn batch_reveal_bad_nonce_length_errors() {
    // A nonce that decodes to fewer than 32 bytes trips the Crypto guard.
    let (conn, ctx, per_name) = setup(&[("alpha", "1122".to_string())]);

    let err = build_batch_reveal_draft_inner(&conn, &ctx, per_name, 10).unwrap_err();
    match err {
        AppError::Crypto(msg) => assert!(msg.contains("not 32 bytes")),
        other => panic!("expected Crypto, got {other:?}"),
    }

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 0);
}

#[test]
fn batch_reveal_non_hex_nonce_errors() {
    let (conn, ctx, per_name) = setup(&[("alpha", "zz".repeat(32))]);
    let err = build_batch_reveal_draft_inner(&conn, &ctx, per_name, 10).unwrap_err();
    assert!(matches!(err, AppError::Crypto(_)));
}

#[test]
fn batch_reveal_single_name() {
    let (conn, ctx, per_name) = setup(&[("solo", "33".repeat(32))]);
    let summary = build_batch_reveal_draft_inner(&conn, &ctx, per_name, 25).unwrap();
    assert_eq!(summary.action, "batch-reveal");
}
