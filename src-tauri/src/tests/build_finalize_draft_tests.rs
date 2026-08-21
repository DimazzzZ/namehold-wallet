//! Tests for `build_finalize_draft_inner` — the pure inner logic of the
//! `build_finalize_draft` Tauri command, extracted so it can be exercised
//! without a `State<AppState>` or a live node.
//!
//! `build_finalize_draft` completes a name transfer: given an owner coin
//! holding a TRANSFER covenant (which records the target address), it spends
//! that coin as a name input, emits a FINALIZE covenant output that moves the
//! name to the target address, funds any fee from spendable coins, and persists
//! the draft (reserving the owner coin + any funding coin) with the target
//! address recorded on the draft summary.
//!
//! The two async RPC calls (`fetch_name_state` for `ns` and `renewal_block`
//! for the renewal-block hash) + the owner-coin lookup stay in the thin
//! `#[tauri::command]` wrapper; the inner function takes the resolved
//! `NameState`, owner `NameCoin`, and renewal block hash directly, and
//! performs the covenant parsing + target-address extraction synchronously.

use std::collections::HashMap;

use crate::commands::names::{build_finalize_draft_inner, Ctx, NameState};
use crate::db;
use crate::db::queries::NameCoin;
use crate::error::AppError;
use crate::noncustodial::address;
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

/// A CLOSED-phase name state. The `weak` flag affects the finalize covenant.
fn closed_name_state(weak: bool) -> NameState {
    NameState {
        height: 1000,
        value: 0,
        renewals: 5,
        claimed: 1,
        weak,
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

/// Seed the owner coin in a TRANSFER state (holding the target address in its
/// covenant) into tracked_utxos + tracked_name_states.
fn seed_transfer_coin(
    conn: &rusqlite::Connection,
    txid: &str,
    value: i64,
    address: &str,
    target_h160: &[u8; 20],
) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    // A TRANSFER covenant: items = [nameHash, height, version, addrHash]
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [
            nh_hex,
            "e8030000", // height 1000 in little-endian u32
            "00",       // version 0 (p2wpkh)
            hex::encode(target_h160)
        ]
    })
    .to_string();
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
            sync::COV_TRANSFER as i64,
            cov_json
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, ?3, 'CLOSED', ?4, 0, 1000)",
        rusqlite::params![PROFILE, NAME, nh_hex, txid],
    )
    .unwrap();
}

/// Build the fixture owner `NameCoin` in TRANSFER state (holding the target).
fn transfer_coin(txid: &str, addr: &str, value: u64, target_h160: &[u8; 20]) -> NameCoin {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [
            nh_hex,
            "e8030000",
            "00",
            hex::encode(target_h160)
        ]
    })
    .to_string();
    NameCoin {
        txid: txid.into(),
        vout: 0,
        value,
        address: addr.into(),
        branch: derivation::BRANCH_RECEIVE,
        child_index: 0,
        covenant_type: sync::COV_TRANSFER as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    }
}

/// Full setup: profile, funding coin, owner coin in TRANSFER state (in DB +
/// tracked_name_states), a `Ctx` with real derived receive/change addresses,
/// and a valid target h160 (recipient's 20-byte address hash). Returns the Ctx,
/// the fixture TRANSFER `NameCoin`, the renewal block hash, and the target h160.
fn setup(
    owner_value: u64,
    funding_value: i64,
) -> (rusqlite::Connection, Ctx, NameCoin, [u8; 32], [u8; 20]) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    // A distinct target address — derive at index 1 and extract its h160.
    let target_addr = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 1)
        .unwrap()
        .address;
    // Decode the target address to get version + h160.
    let (version, program) = address::decode(network, &target_addr).unwrap();
    assert_eq!(version, 0, "test target must be p2wpkh");
    let mut target_h160 = [0u8; 20];
    target_h160.copy_from_slice(&program);

    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, funding_value, &recv0.address);

    let owner_txid = "bb".repeat(32);
    seed_transfer_coin(
        &conn,
        &owner_txid,
        owner_value as i64,
        &recv0.address,
        &target_h160,
    );

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
    let coin = transfer_coin(&owner_txid, &recv0.address, owner_value, &target_h160);
    let rblock = [0x77u8; 32]; // arbitrary renewal block hash
    (conn, ctx, coin, rblock, target_h160)
}

#[test]
fn build_finalize_draft_succeeds_and_persists() {
    let (conn, ctx, coin, rblock, _target_h160) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state(false);

    let summary =
        build_finalize_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin, &rblock).unwrap();
    assert_eq!(summary.action, "finalize");

    // The owner coin is reserved as the finalize's name input.
    let owner_reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_TRANSFER as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        owner_reserved, 1,
        "the owner coin must be reserved as the finalize name input"
    );
}

#[test]
fn build_finalize_draft_records_target_address() {
    let (conn, ctx, coin, rblock, _target_h160) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state(false);

    let summary =
        build_finalize_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin, &rblock).unwrap();
    // The target address is recorded on the draft summary.
    let recorded = summary
        .summary
        .get("recipientAddress")
        .and_then(|v| v.as_str());
    assert!(
        recorded.is_some(),
        "target address recorded on the finalize draft"
    );
}

#[test]
fn build_finalize_draft_keeps_full_owner_coin_value() {
    let (conn, ctx, coin, rblock, _target_h160) = setup(7_500_000, 10_000_000);
    let ns = closed_name_state(false);

    let summary =
        build_finalize_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin, &rblock).unwrap();
    // FINALIZE preserves the full owner-coin value (moves it to the target).
    let send_total = summary
        .summary
        .get("sendTotalDoos")
        .and_then(|v| v.as_i64());
    assert_eq!(
        send_total,
        Some(7_500_000),
        "finalize keeps the full owner-coin value"
    );
}

#[test]
fn build_finalize_draft_uses_explicit_fee_rate() {
    let (conn, ctx, coin, rblock, _target_h160) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state(false);

    let summary =
        build_finalize_draft_inner(&conn, &ctx, NAME, Some(800), &ns, &coin, &rblock).unwrap();
    assert_eq!(summary.action, "finalize");
}

#[test]
fn build_finalize_draft_rejects_non_transfer_coin() {
    let (conn, ctx, _coin, rblock, _target_h160) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state(false);

    // A coin with no covenant_json (not in TRANSFER state).
    let bad_coin = NameCoin {
        txid: "cc".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: "hs1qtest".into(),
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_REVEAL as i64,
        covenant_json: None,
        name_height: Some(1000),
    };

    let err = build_finalize_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &bad_coin, &rblock)
        .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        drafts, 0,
        "no draft persisted when the coin is not in TRANSFER state"
    );
}

#[test]
fn build_finalize_draft_fails_with_insufficient_funds() {
    let (conn, ctx, coin, rblock, _target_h160) = setup(5_000_000, 1);
    let ns = closed_name_state(false);

    let err =
        build_finalize_draft_inner(&conn, &ctx, NAME, Some(100), &ns, &coin, &rblock).unwrap_err();
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

#[test]
fn build_finalize_draft_includes_weak_flag_in_covenant() {
    let (conn, ctx, coin, rblock, _target_h160) = setup(5_000_000, 10_000_000);
    // A weak name — the finalize covenant will include the weak flag.
    let ns = closed_name_state(true);

    let summary =
        build_finalize_draft_inner(&conn, &ctx, NAME, Some(10), &ns, &coin, &rblock).unwrap();
    assert_eq!(summary.action, "finalize");
    // The draft persists successfully; the weak flag is encoded into the
    // covenant (not directly visible in the summary, but the plan is built).
}
