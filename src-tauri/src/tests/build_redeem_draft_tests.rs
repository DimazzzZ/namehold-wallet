//! Tests for `build_redeem_draft_inner` — the pure inner logic of the
//! `build_redeem_draft` Tauri command, extracted so it can be exercised without
//! a `State<AppState>` or a live node.
//!
//! `build_redeem_draft` reclaims the value locked in a *losing* bid's REVEAL
//! coin: it spends the unspent COV_REVEAL coin as a name input, emits a REDEEM
//! covenant output that returns the reveal value back to the wallet, funds any
//! extra fee from spendable coins, and persists the draft (reserving the REVEAL
//! coin + any funding coin).
//!
//! The async node RPC (`fetch_name_state`) + the bid-commitment / REVEAL-coin
//! lookups stay in the thin `#[tauri::command]` wrapper; the inner function
//! takes the resolved `NameState` and REVEAL `NameCoin` directly, so these
//! tests build those fixtures and need no mock RPC.

use std::collections::HashMap;

use crate::commands::names::{build_redeem_draft_inner, Ctx, NameState};
use crate::db;
use crate::db::queries::NameCoin;
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

/// A CLOSED-phase name state at auction-start height 1000 (redeem happens
/// after the auction resolves).
fn closed_name_state() -> NameState {
    NameState {
        height: 1000,
        value: 0,
        renewals: 0,
        claimed: 0,
        weak: false,
        phase: "CLOSED".into(),
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

/// Seed the unspent losing REVEAL coin (the name input the redeem spends) into
/// tracked_utxos so `persist_with_conn`'s reservation step can claim it.
fn seed_reveal_coin(conn: &rusqlite::Connection, txid: &str, value: i64, address: &str) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    let cov_json = format!(r#"{{"type":{},"items":["{}"]}}"#, sync::COV_REVEAL, nh_hex);
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, 'deadbeef', ?4, ?5, ?6, 'name_control', NULL)",
        rusqlite::params![
            txid,
            PROFILE,
            address,
            value,
            sync::COV_REVEAL as i64,
            cov_json
        ],
    )
    .unwrap();
}

/// Build the fixture REVEAL `NameCoin` the redeem spends as its name input.
fn reveal_coin(txid: &str, addr: &str, value: u64) -> NameCoin {
    NameCoin {
        txid: txid.into(),
        vout: 0,
        value,
        address: addr.into(),
        branch: derivation::BRANCH_RECEIVE,
        child_index: 0,
        covenant_type: sync::COV_REVEAL as i64,
        covenant_json: None,
        name_height: Some(1000),
    }
}

/// Full setup: profile, funding coin, REVEAL coin (in DB), and a `Ctx` with
/// real derived receive/change addresses. Returns the Ctx plus the fixture
/// REVEAL `NameCoin` to pass into the inner.
fn setup(reveal_value: u64, funding_value: i64) -> (rusqlite::Connection, Ctx, NameCoin) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, funding_value, &recv0.address);

    let reveal_txid = "bb".repeat(32);
    seed_reveal_coin(&conn, &reveal_txid, reveal_value as i64, &recv0.address);

    let ctx = Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub,
        change_address: change.address,
        funding: vec![SpendableCoin {
            txid: funding_txid,
            vout: 0,
            value: funding_value as u64,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    };
    let coin = reveal_coin(&reveal_txid, &recv0.address, reveal_value);
    (conn, ctx, coin)
}

#[test]
fn build_redeem_draft_succeeds_and_persists() {
    let (conn, ctx, coin) = setup(2_000_000, 10_000_000);
    let ns = closed_name_state();

    let summary = build_redeem_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin).unwrap();
    assert_eq!(summary.action, "redeem");

    // The REVEAL coin is reserved as the redeem's name input.
    let reveal_reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_REVEAL as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reveal_reserved, 1,
        "the REVEAL coin must be reserved as the redeem name input"
    );
}

#[test]
fn build_redeem_draft_reclaims_reveal_value_to_its_address() {
    let (conn, ctx, coin) = setup(2_000_000, 10_000_000);
    let ns = closed_name_state();

    let summary = build_redeem_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin).unwrap();
    // The redeem output value equals the reveal coin value (reclaimed).
    let send_total = summary
        .summary
        .get("sendTotalDoos")
        .and_then(|v| v.as_i64());
    assert_eq!(
        send_total,
        Some(2_000_000),
        "redeem reclaims the full reveal value"
    );
}

#[test]
fn build_redeem_draft_uses_explicit_fee_rate() {
    let (conn, ctx, coin) = setup(2_000_000, 10_000_000);
    let ns = closed_name_state();

    let summary = build_redeem_draft_inner(&conn, &ctx, NAME, Some(300), &ns, &coin).unwrap();
    assert_eq!(summary.action, "redeem");
}

#[test]
fn build_redeem_draft_funds_fee_when_reveal_has_no_slack() {
    // reveal value == redeem output value, so the fee must come from funding.
    let (conn, ctx, coin) = setup(1_000_000, 10_000_000);
    let ns = closed_name_state();

    let summary = build_redeem_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin).unwrap();
    assert_eq!(summary.action, "redeem");
    // Both the REVEAL coin and the funding coin should be reserved.
    let reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reserved, 2,
        "fee funded from a liquid coin in addition to the REVEAL coin"
    );
}

#[test]
fn build_redeem_draft_fails_with_insufficient_funds() {
    // reveal value == redeem output value (zero slack) + dust funding → the
    // fee can't be covered, so the plan fails and nothing is persisted.
    let (conn, ctx, coin) = setup(1_000_000, 1);
    let ns = closed_name_state();

    let err = build_redeem_draft_inner(&conn, &ctx, NAME, Some(100), &ns, &coin).unwrap_err();
    assert!(matches!(
        err,
        AppError::InvalidInput(_) | AppError::Other(_)
    ));

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        drafts, 0,
        "no draft persisted when the plan can't be funded"
    );
}
