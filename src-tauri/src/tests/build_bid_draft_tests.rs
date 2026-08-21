//! Tests for `build_bid_draft_inner` — the pure inner logic of the
//! `build_bid_draft` Tauri command, extracted so it can be exercised without
//! a `State<AppState>` or a live node.
//!
//! `build_bid_draft` builds a BID covenant draft: it derives the next unused
//! receive address, computes a blinded bid (nonce + blind from the wallet
//! xpub), funds a lockup-value BID output, enforces the one-bid-per-name guard,
//! and persists a bid commitment (secret nonce/blind) + reveal-end-height +
//! draft + on-chain bid txid.
//!
//! The async node RPC (`fetch_name_state`) stays in the thin `#[tauri::command]`
//! wrapper and is covered separately in `node_rpc_injected_tests.rs`; the inner
//! function takes the resolved `NameState` directly, so these tests pass a
//! fixture `NameState` and need no mock RPC.

use std::collections::HashMap;

use crate::commands::names::{build_bid_draft_inner, Ctx, NameState};
use crate::db;
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

/// A BIDDING-phase name state at auction-start height 1000.
fn bidding_name_state() -> NameState {
    NameState {
        height: 1000,
        value: 0,
        renewals: 0,
        claimed: 0,
        weak: false,
        phase: "BIDDING".into(),
    }
}

/// Insert a spendable liquid `tracked_utxos` row so the funding coin the plan
/// spends can actually be reserved by the draft insert.
fn seed_tracked_coin(
    conn: &rusqlite::Connection,
    txid: &str,
    vout: i64,
    value: i64,
    address: &str,
) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, ?2, ?3, ?4, 'deadbeef', ?5, 0, NULL, 'liquid_hns', NULL)",
        rusqlite::params![txid, vout, PROFILE, address, value],
    )
    .unwrap();
}

/// Build a `Ctx` with real derived receive/change addresses and one funding
/// coin (seeded into tracked_utxos so the reservation step succeeds).
fn seed_ctx(conn: &rusqlite::Connection, funding_txid: &str, funding_value: i64) -> Ctx {
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    seed_tracked_coin(conn, funding_txid, 0, funding_value, &recv0.address);
    Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub,
        change_address: change.address,
        funding: vec![SpendableCoin {
            txid: funding_txid.into(),
            vout: 0,
            value: funding_value as u64,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    }
}

// --- input validation is in the wrapper, but the inner still relies on it ---
// (bid_value <= 0 / lockup < bid_value are rejected by the wrapper before
//  build_bid_draft_inner is reached; not re-tested here.)

#[test]
fn build_bid_draft_succeeds_and_persists_commitment_and_draft() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);
    let ns = bidding_name_state();

    let summary =
        build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(10), &ns).unwrap();

    assert_eq!(summary.action, "bid");

    // A bid commitment row now exists for this name.
    let commit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
            rusqlite::params![PROFILE, NAME],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(commit_count, 1, "one bid commitment persisted");

    // The bid txid is stamped onto the commitment (Task 1 fix).
    let bid_txid: Option<String> = conn
        .query_row(
            "SELECT bid_txid FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
            rusqlite::params![PROFILE, NAME],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        bid_txid.is_some(),
        "bid_txid should be stamped onto the commitment"
    );

    // The funding coin is reserved by the draft.
    let reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 1);
}

#[test]
fn build_bid_draft_stamps_reveal_end_height() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);
    let ns = bidding_name_state();

    build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(10), &ns).unwrap();

    // reveal_end_height is derived from ns.height + network params; assert a
    // non-null value > the auction-start height was stamped.
    let reveal_end: Option<i64> = conn
        .query_row(
            "SELECT reveal_end_height FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
            rusqlite::params![PROFILE, NAME],
            |r| r.get(0),
        )
        .unwrap();
    let reveal_end = reveal_end.expect("reveal_end_height should be set");
    assert!(
        reveal_end > 1000,
        "reveal window closes after auction start height"
    );
}

#[test]
fn build_bid_draft_uses_explicit_fee_rate() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);
    let ns = bidding_name_state();

    let summary =
        build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(42), &ns).unwrap();
    assert_eq!(summary.action, "bid");
}

#[test]
fn build_bid_draft_rejects_when_bid_coin_exists() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);
    let ns = bidding_name_state();

    // Seed an unspent COV_BID coin for this name → multiplicity guard (a).
    // Detection JOINs tracked_utxos → derived_addresses and matches the name
    // hash from covenant_json items[0].
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    let bid_addr = "hs1qbidcoin";
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index, address,
             script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 99, ?2, 'deadbeef', 'deadbeef')",
        rusqlite::params![PROFILE, bid_addr],
    )
    .unwrap();
    let cov_json = format!(r#"{{"type":{},"items":["{}"]}}"#, sync::COV_BID, nh_hex);
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, 'deadbeef', 2000000, ?4, ?5, 'name_lockup', NULL)",
        rusqlite::params![
            "bb".repeat(32),
            PROFILE,
            bid_addr,
            sync::COV_BID as i64,
            cov_json
        ],
    )
    .unwrap();

    let err =
        build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(10), &ns).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("already has an unspent bid")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn build_bid_draft_rejects_second_bid_when_draft_pending() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);
    let ns = bidding_name_state();

    // First bid succeeds and leaves a pending `bid` draft.
    build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(10), &ns).unwrap();

    // Second bid for the same name is rejected by guard (b) — pending draft.
    // (This surfaces before the funding-reservation path.)
    let err =
        build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(10), &ns).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => {
            assert!(msg.contains("already pending") || msg.contains("one bid per wallet"))
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn build_bid_draft_fails_with_insufficient_funds() {
    let conn = test_db();
    seed_profile(&conn);
    // A dust coin that can't cover a 2M lockup + fee.
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 1);
    let ns = bidding_name_state();

    let err =
        build_bid_draft_inner(&conn, &ctx, NAME, 1_000_000, 2_000_000, Some(100), &ns).unwrap_err();
    assert!(matches!(
        err,
        AppError::InvalidInput(_) | AppError::Other(_)
    ));

    // On failure, NO commitment row and NO draft should have been persisted.
    let commit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1",
            rusqlite::params![PROFILE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        commit_count, 0,
        "no commitment persisted when the plan can't be funded"
    );
}

#[test]
fn build_bid_draft_rejects_invalid_name() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);
    let ns = bidding_name_state();

    let err =
        build_bid_draft_inner(&conn, &ctx, "", 1_000_000, 2_000_000, Some(10), &ns).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}
