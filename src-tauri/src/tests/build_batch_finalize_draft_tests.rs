//! Tests for `build_batch_finalize_draft_inner` — the pure inner logic of the
//! `build_batch_finalize_draft` Tauri command, extracted so it can be exercised
//! without a `State<AppState>` or a live node.
//!
//! The inner takes a pre-resolved `Vec<(name, name_hash, raw_name, owner_coin,
//! name_state)>` (the wrapper's per-name `owner_coin_and_state` prefetch) +
//! the renewal-block hash, extracts the TRANSFER target from each coin's
//! covenant, builds one FINALIZE output per name, and persists a single batch
//! draft reserving all inputs.

use std::collections::HashMap;

use crate::commands::names::{build_batch_finalize_draft_inner, Ctx, NameState};
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

/// Seed a TRANSFER-state owner coin into tracked_utxos so the draft can
/// reserve it (matches `build_finalize_draft_tests::seed_transfer_coin`).
fn seed_transfer_coin(
    conn: &rusqlite::Connection,
    name: &str,
    txid: &str,
    value: i64,
    address: &str,
    target_h160: &[u8; 20],
) {
    let nh_hex = hex::encode(names::hash_name(name).unwrap());
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [nh_hex, "e8030000", "00", hex::encode(target_h160)]
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
}

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

/// Build a TRANSFER-state `NameCoin` fixture with the given target h160.
fn transfer_coin(
    name: &str,
    txid: &str,
    addr: &str,
    value: u64,
    target_h160: &[u8; 20],
) -> NameCoin {
    let nh_hex = hex::encode(names::hash_name(name).unwrap());
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

type PerName = Vec<(String, [u8; 32], Vec<u8>, NameCoin, NameState)>;

fn setup(name_list: &[&str]) -> (rusqlite::Connection, Ctx, PerName, [u8; 32]) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    // Target: derive at index 1.
    let target_addr =
        derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 1).unwrap();
    let (_v, program) = address::decode(network, &target_addr.address).unwrap();
    let mut target_h160 = [0u8; 20];
    target_h160.copy_from_slice(&program);

    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, 50_000_000, &recv0.address);

    let mut per_name: PerName = Vec::new();
    for (i, name) in name_list.iter().enumerate() {
        let txid = format!("{:02x}", i + 0xd0).repeat(32);
        let nh = names::hash_name(name).unwrap();
        let raw = names::raw_name(name).unwrap();
        let coin = transfer_coin(name, &txid, &recv0.address, 5_000_000, &target_h160);
        seed_transfer_coin(&conn, name, &txid, 5_000_000, &recv0.address, &target_h160);
        per_name.push((name.to_string(), nh, raw, coin, closed_name_state(false)));
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
    let rblock = [0x77u8; 32];
    (conn, ctx, per_name, rblock)
}

#[test]
fn batch_finalize_happy_path_persists_draft() {
    let (conn, ctx, per_name, rblock) = setup(&["alpha", "bravo"]);
    let summary = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 10).unwrap();
    assert_eq!(summary.action, "batch-finalize");

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 1);

    // Both owner coins are reserved.
    let reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_TRANSFER as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 2);
}

#[test]
fn batch_finalize_single_name() {
    let (conn, ctx, per_name, rblock) = setup(&["solo"]);
    let summary = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 20).unwrap();
    assert_eq!(summary.action, "batch-finalize");
}

#[test]
fn batch_finalize_empty_errors() {
    // The async wrapper rejects empty `names` up front; the inner also
    // errors because `build_batch_plan` requires at least one output.
    let (conn, ctx, _per_name, rblock) = setup(&[]);
    let err = build_batch_finalize_draft_inner(&conn, &ctx, Vec::new(), &rblock, 10).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("at least one output"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn batch_finalize_rejects_non_transfer_coin() {
    let (conn, ctx, _per_name, rblock) = setup(&[]);
    let network = Network::Main;
    let xpub = test_xpub();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    // A coin with no covenant_json (not in TRANSFER state).
    let bad_coin = NameCoin {
        txid: "cc".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: recv0.address,
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_REVEAL as i64,
        covenant_json: None,
        name_height: Some(1000),
    };
    let nh = names::hash_name("badname").unwrap();
    let raw = names::raw_name("badname").unwrap();
    let per_name = vec![(
        "badname".to_string(),
        nh,
        raw,
        bad_coin,
        closed_name_state(false),
    )];

    let err = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 10).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}

#[test]
fn batch_finalize_rejects_too_few_covenant_items() {
    let (conn, ctx, _per_name, rblock) = setup(&[]);
    let network = Network::Main;
    let xpub = test_xpub();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    // Covenant with only 3 items (needs 4).
    let nh_hex = hex::encode(names::hash_name("short").unwrap());
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [nh_hex, "e8030000", "00"]
    })
    .to_string();
    let bad_coin = NameCoin {
        txid: "dd".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: recv0.address,
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_TRANSFER as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    };
    let nh = names::hash_name("short").unwrap();
    let raw = names::raw_name("short").unwrap();
    let per_name = vec![(
        "short".to_string(),
        nh,
        raw,
        bad_coin,
        closed_name_state(false),
    )];

    let err = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 10).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("not a TRANSFER"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn batch_finalize_rejects_non_p2wpkh_target() {
    let (conn, ctx, _per_name, rblock) = setup(&[]);
    let network = Network::Main;
    let xpub = test_xpub();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    // version = 01 (not p2wpkh) with a 32-byte hash (p2wsh length).
    let nh_hex = hex::encode(names::hash_name("badtarget").unwrap());
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [nh_hex, "e8030000", "01", "aa".repeat(32)]
    })
    .to_string();
    let bad_coin = NameCoin {
        txid: "ee".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: recv0.address,
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_TRANSFER as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    };
    let nh = names::hash_name("badtarget").unwrap();
    let raw = names::raw_name("badtarget").unwrap();
    let per_name = vec![(
        "badtarget".to_string(),
        nh,
        raw,
        bad_coin,
        closed_name_state(false),
    )];

    let err = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 10).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("p2wpkh"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}
