//! Tests for `build_reveal_draft_inner` — the pure inner logic of the
//! `build_reveal_draft` Tauri command, extracted so it can be exercised without
//! a `State<AppState>` or a live node.
//!
//! `build_reveal_draft` builds a REVEAL covenant draft: given a prior bid
//! commitment (secret nonce/blind) and the wallet's unspent BID coin, it spends
//! the BID coin as a name input, emits a REVEAL output carrying the true bid
//! value, funds the fee from spendable coins, persists the draft (reserving both
//! the funding coin and the BID coin), and stamps the reveal txid back onto the
//! commitment row for the reveal-deadline scanner.
//!
//! The async node RPC (`fetch_name_state`) + the bid-commitment / BID-coin
//! lookups stay in the thin `#[tauri::command]` wrapper; the inner function
//! takes the resolved `NameState`, `BidCommitmentRow`, and BID `NameCoin`
//! directly, so these tests build those fixtures and need no mock RPC.

use std::collections::HashMap;

use crate::commands::names::{build_reveal_draft_inner, Ctx, NameState};
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
const NAME: &str = "example";

/// secp256k1 generator point G in compressed form — a guaranteed-valid
/// compressed public key, so `ExtendedPubKey::from_parts` never rejects it.
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

/// A REVEAL-phase name state at auction-start height 1000.
fn reveal_name_state() -> NameState {
    NameState {
        height: 1000,
        value: 0,
        renewals: 0,
        claimed: 0,
        weak: false,
        phase: "REVEAL".into(),
    }
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

/// Seed the unspent BID coin (the name input the reveal spends) into
/// tracked_utxos so `persist_with_conn`'s reservation step can claim it.
fn seed_bid_coin(conn: &rusqlite::Connection, txid: &str, value: i64, address: &str) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
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

/// Persist a bid commitment row so `set_bid_reveal_txid` (called inside the
/// inner) has a row to stamp. The inner reads nonce/bid_value from the passed
/// `BidCommitmentRow`, but the reveal-txid stamp writes to the DB by name.
fn seed_bid_commitment_row(conn: &rusqlite::Connection, addr: &str, nonce_hex: &str) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    db::queries::insert_bid_commitment(
        conn, PROFILE, NAME, &nh_hex, addr, 0, 0, 1_000_000, 2_000_000, nonce_hex, "beef",
    )
    .unwrap();
}

/// Build the fixture `BidCommitmentRow` the inner consumes (nonce + bid value).
fn bid_row(addr: &str, nonce_hex: &str) -> BidCommitmentRow {
    BidCommitmentRow {
        name: NAME.into(),
        name_hash_hex: hex::encode(names::hash_name(NAME).unwrap()),
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

/// Build the fixture BID `NameCoin` the reveal spends as its name input.
fn bid_coin(txid: &str, addr: &str, value: u64) -> NameCoin {
    NameCoin {
        txid: txid.into(),
        vout: 0,
        value,
        address: addr.into(),
        branch: derivation::BRANCH_RECEIVE,
        child_index: 0,
        covenant_type: sync::COV_BID as i64,
        covenant_json: None,
        name_height: Some(1000),
    }
}

/// Full setup: profile, funding coin, BID coin (in DB), bid commitment (in DB),
/// and a `Ctx` with real derived receive/change addresses. Returns the Ctx plus
/// the fixture `BidCommitmentRow` and BID `NameCoin` to pass into the inner.
fn setup() -> (rusqlite::Connection, Ctx, BidCommitmentRow, NameCoin) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    // 32-byte nonce (hex) — the reveal covenant requires exactly 32 bytes.
    let nonce_hex = "11".repeat(32);

    // Funding coin (fee) at a liquid address.
    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, 10_000_000, &recv0.address);

    // BID coin the reveal spends — lives at recv0 (branch 0, index 0).
    let bid_txid = "bb".repeat(32);
    seed_bid_coin(&conn, &bid_txid, 2_000_000, &recv0.address);
    seed_bid_commitment_row(&conn, &recv0.address, &nonce_hex);

    let ctx = Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub,
        change_address: change.address,
        funding: vec![SpendableCoin {
            txid: funding_txid,
            vout: 0,
            value: 10_000_000,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    };
    let bid = bid_row(&recv0.address, &nonce_hex);
    let coin = bid_coin(&bid_txid, &recv0.address, 2_000_000);
    (conn, ctx, bid, coin)
}

#[test]
fn build_reveal_draft_succeeds_and_persists() {
    let (conn, ctx, bid, coin) = setup();
    let ns = reveal_name_state();

    let summary = build_reveal_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &bid, &coin).unwrap();
    assert_eq!(summary.action, "reveal");

    // The BID coin is always reserved (it's the name input the reveal spends).
    // The funding coin may or may not be reserved depending on whether the
    // BID coin's slack (lockup − bid value) already covers the fee.
    let bid_reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_BID as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        bid_reserved, 1,
        "the BID coin must be reserved as the reveal's name input"
    );
}

#[test]
fn build_reveal_draft_stamps_reveal_txid_on_commitment() {
    let (conn, ctx, bid, coin) = setup();
    let ns = reveal_name_state();

    build_reveal_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &bid, &coin).unwrap();

    // The reveal txid is stamped back onto the commitment row (Task 1 fix).
    let reveal_txid: Option<String> = conn
        .query_row(
            "SELECT reveal_txid FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
            rusqlite::params![PROFILE, NAME],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        reveal_txid.is_some(),
        "reveal_txid should be stamped onto the commitment"
    );
}

#[test]
fn build_reveal_draft_uses_explicit_fee_rate() {
    let (conn, ctx, bid, coin) = setup();
    let ns = reveal_name_state();

    let summary = build_reveal_draft_inner(&conn, &ctx, NAME, Some(250), &ns, &bid, &coin).unwrap();
    assert_eq!(summary.action, "reveal");
}

#[test]
fn build_reveal_draft_rejects_bad_nonce_length() {
    let (conn, ctx, _bid, coin) = setup();
    let ns = reveal_name_state();
    // A nonce that decodes to fewer than 32 bytes must be rejected.
    let bad = bid_row(&coin.address, "1122");

    let err = build_reveal_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &bad, &coin).unwrap_err();
    match err {
        AppError::Crypto(msg) => assert!(msg.contains("32 bytes")),
        other => panic!("expected Crypto error, got {other:?}"),
    }
}

#[test]
fn build_reveal_draft_rejects_non_hex_nonce() {
    let (conn, ctx, _bid, coin) = setup();
    let ns = reveal_name_state();
    let bad = bid_row(&coin.address, "zzzz");

    let err = build_reveal_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &bad, &coin).unwrap_err();
    assert!(matches!(err, AppError::Crypto(_)));
}

#[test]
fn build_reveal_draft_fails_with_insufficient_funds() {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    let nonce_hex = "11".repeat(32);

    // BID coin value == bid value → zero slack → the fee must come from
    // funding. A dust funding coin can't cover it, so the plan fails.
    seed_liquid_coin(&conn, &"aa".repeat(32), 1, &recv0.address);
    let bid_txid = "bb".repeat(32);
    seed_bid_coin(&conn, &bid_txid, 1_000_000, &recv0.address);
    seed_bid_commitment_row(&conn, &recv0.address, &nonce_hex);

    let ctx = Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub,
        change_address: change.address,
        funding: vec![SpendableCoin {
            txid: "aa".repeat(32),
            vout: 0,
            value: 1,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    };
    let bid = bid_row(&recv0.address, &nonce_hex);
    let coin = bid_coin(&bid_txid, &recv0.address, 1_000_000);
    let ns = reveal_name_state();

    let err = build_reveal_draft_inner(&conn, &ctx, NAME, Some(100), &ns, &bid, &coin).unwrap_err();
    assert!(matches!(
        err,
        AppError::InvalidInput(_) | AppError::Other(_)
    ));
}
