//! Tests for `build_open_draft_inner` — the pure inner logic of the
//! `build_open_draft` Tauri command, extracted so it can be exercised without
//! a `State<AppState>`.
//!
//! `build_open_draft` builds an OPEN covenant draft: it derives the next unused
//! receive address, funds a value-0 OPEN output from the wallet's spendable
//! coins, enforces the double-open guard (no second OPEN while one is pending),
//! and persists a `wallet_tx_drafts` row that reserves the funding coins.
//!
//! These tests seed an in-memory DB with a profile + spendable `tracked_utxos`,
//! construct a `Ctx` with a real (deterministic) xpub so address derivation
//! produces valid Handshake addresses, then call `build_open_draft_inner`
//! directly with the owned connection (no mutex — tests are single-threaded).

use std::collections::HashMap;

use crate::commands::names::{build_open_draft_inner, Ctx};
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
        conn, PROFILE, "Test", "mnemonic_hot", "mainnet", "xpubFAKE", 0, false,
    )
    .unwrap();
}

/// Insert a spendable `tracked_utxos` row at `(txid, vout)` so the funding
/// coin the plan spends can actually be reserved by the draft insert.
fn seed_tracked_coin(conn: &rusqlite::Connection, txid: &str, vout: i64, value: i64, address: &str) {
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
/// coin. Also seeds the matching `tracked_utxos` row for that coin so the
/// reservation step succeeds.
fn seed_ctx_with_funding(conn: &rusqlite::Connection, settings: HashMap<String, String>) -> Ctx {
    let network = Network::Main;
    let xpub = test_xpub();
    // Change address: same path load_ctx uses.
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    // Funding coin lives at the first receive address so the seeded
    // tracked_utxos address is plausible (not strictly required — the plan
    // reserves by txid/vout, not address).
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    let funding_txid = "aa".repeat(32);
    seed_tracked_coin(conn, &funding_txid, 0, 10_000_000, &recv0.address);

    Ctx {
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
        settings,
    }
}

#[test]
fn build_open_draft_succeeds_and_persists() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx_with_funding(&conn, HashMap::new());

    let summary = build_open_draft_inner(&conn, &ctx, NAME, Some(10)).unwrap();

    // The persisted draft is an OPEN for this name.
    assert_eq!(summary.action, "open");
    // A draft row now exists and reserves the funding coin.
    let reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 1, "the funding coin should be reserved by the draft");
}

#[test]
fn build_open_draft_uses_explicit_fee_rate() {
    let conn = test_db();
    seed_profile(&conn);
    // Settings has a fee_rate that would be used if no override is passed.
    let mut settings = HashMap::new();
    settings.insert("fee_rate_doos_per_kvb".to_string(), "5000".to_string());
    let ctx = seed_ctx_with_funding(&conn, settings);

    // Explicit override should win — just assert it builds successfully.
    let summary = build_open_draft_inner(&conn, &ctx, NAME, Some(42)).unwrap();
    assert_eq!(summary.action, "open");
}

#[test]
fn build_open_draft_rejects_when_open_coin_exists() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx_with_funding(&conn, HashMap::new());

    // Seed an unspent COV_OPEN coin for this name → double-open guard (a).
    // The detection query JOINs tracked_utxos → derived_addresses on address
    // and matches the name hash from covenant_json items[0], so we need both
    // a derived_addresses row and a covenant_json carrying the name hash.
    let nh = names::hash_name(NAME).unwrap();
    let nh_hex = hex::encode(nh);
    let open_addr = "hs1qopencoin";
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index, address,
             script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 99, ?2, 'deadbeef', 'deadbeef')",
        rusqlite::params![PROFILE, open_addr],
    )
    .unwrap();
    let cov_json = format!(r#"{{"type":{},"items":["{}"]}}"#, sync::COV_OPEN, nh_hex);
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, 'deadbeef', 0, ?4, ?5, 'name_control', NULL)",
        rusqlite::params![
            "bb".repeat(32),
            PROFILE,
            open_addr,
            sync::COV_OPEN as i64,
            cov_json
        ],
    )
    .unwrap();

    let err = build_open_draft_inner(&conn, &ctx, NAME, Some(10)).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("already being opened")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn build_open_draft_rejects_when_pending_open_draft_exists() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx_with_funding(&conn, HashMap::new());

    // First open succeeds and leaves a pending `open` draft.
    build_open_draft_inner(&conn, &ctx, NAME, Some(10)).unwrap();

    // A second open for the same name must be rejected by guard (b).
    let err = build_open_draft_inner(&conn, &ctx, NAME, Some(10)).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("already being opened")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn build_open_draft_allows_different_name_after_first() {
    let conn = test_db();
    seed_profile(&conn);
    // Two funding coins on separate names. The first draft reserves its
    // funding coin, so the second `Ctx` uses the OTHER coin — otherwise the
    // draft-insert reservation step rejects the (already-reserved) coin.
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    seed_tracked_coin(&conn, &"aa".repeat(32), 0, 10_000_000, &recv0.address);
    seed_tracked_coin(&conn, &"cc".repeat(32), 0, 10_000_000, &recv0.address);
    let ctx1 = Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub.clone(),
        change_address: change.address.clone(),
        funding: vec![SpendableCoin {
            txid: "aa".repeat(32),
            vout: 0,
            value: 10_000_000,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    };
    let ctx2 = Ctx {
        profile_id: PROFILE.into(),
        network,
        account: 0,
        account_xpub: xpub,
        change_address: change.address,
        funding: vec![SpendableCoin {
            txid: "cc".repeat(32),
            vout: 0,
            value: 10_000_000,
            branch: derivation::BRANCH_RECEIVE,
            child_index: 0,
        }],
        settings: HashMap::new(),
    };

    build_open_draft_inner(&conn, &ctx1, "example", Some(10)).unwrap();
    // A DIFFERENT name is unaffected by the first name's pending open.
    let summary = build_open_draft_inner(&conn, &ctx2, "different", Some(10)).unwrap();
    assert_eq!(summary.action, "open");
}

#[test]
fn build_open_draft_fails_with_insufficient_funds() {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    // A dust coin that can't cover the fee.
    seed_tracked_coin(&conn, &"aa".repeat(32), 0, 1, &recv0.address);
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

    let err = build_open_draft_inner(&conn, &ctx, NAME, Some(100)).unwrap_err();
    // Insufficient-funds surfaces as an error (exact variant is send-layer's).
    assert!(matches!(err, AppError::InvalidInput(_) | AppError::Other(_)));
}

#[test]
fn build_open_draft_rejects_invalid_name() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx_with_funding(&conn, HashMap::new());

    // Empty name is not a valid Handshake name.
    let err = build_open_draft_inner(&conn, &ctx, "", Some(10)).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}
