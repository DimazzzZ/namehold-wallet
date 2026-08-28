//! Tests for `build_finalize_with_payment_draft_inner` — the pure inner logic
//! of the `build_finalize_with_payment_draft` Tauri command (atomic name
//! swap: finalize a TRANSFER + pay the seller in the same tx).
//!
//! The inner takes a pre-resolved owner `NameCoin` (in TRANSFER state), the
//! `NameState`, and the renewal-block hash (the wrapper's `owner_coin_and_state`
//! + `renewal_block` prefetch); it parses the TRANSFER target, validates the
//! payment address, funds the finalize + payment outputs, and persists the
//! draft recording the payment address as the recipient.

use std::collections::HashMap;

use crate::commands::names::{build_finalize_with_payment_draft_inner, Ctx, NameState};
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
/// reserve it.
fn seed_transfer_coin(
    conn: &rusqlite::Connection,
    txid: &str,
    value: i64,
    address: &str,
    target_h160: &[u8; 20],
) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
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

/// Build a TRANSFER-state `NameCoin` fixture holding the given target h160.
fn transfer_coin(txid: &str, addr: &str, value: u64, target_h160: &[u8; 20]) -> NameCoin {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [nh_hex, "e8030000", "00", hex::encode(target_h160)]
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

/// Returns (conn, ctx, owner_coin, rblock, payment_address).
fn setup(
    owner_value: u64,
    funding_value: i64,
) -> (rusqlite::Connection, Ctx, NameCoin, [u8; 32], String) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    // Finalize target: index 1.
    let target_addr =
        derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 1).unwrap();
    let (_v, program) = address::decode(network, &target_addr.address).unwrap();
    let mut target_h160 = [0u8; 20];
    target_h160.copy_from_slice(&program);
    // Seller payment address: index 2.
    let payment_addr =
        derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 2).unwrap();

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
    let coin = transfer_coin(&owner_txid, &recv0.address, owner_value, &target_h160);

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
    let rblock = [0x77u8; 32];
    (conn, ctx, coin, rblock, payment_addr.address)
}

#[test]
fn finalize_with_payment_happy_path_persists_draft() {
    let (conn, ctx, coin, rblock, pay) = setup(5_000_000, 20_000_000);
    let ns = closed_name_state(false);
    let summary = build_finalize_with_payment_draft_inner(
        &conn, &ctx, NAME, &pay, 3_000_000, 10, &coin, &ns, &rblock,
    )
    .unwrap();
    assert_eq!(summary.action, "finalize-with-payment");

    // Payment address recorded as recipient.
    let recipient = summary
        .summary
        .get("recipientAddress")
        .and_then(|v| v.as_str());
    assert_eq!(recipient, Some(pay.as_str()));

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 1);
}

#[test]
fn finalize_with_payment_weak_flag_ok() {
    let (conn, ctx, coin, rblock, pay) = setup(5_000_000, 20_000_000);
    let ns = closed_name_state(true);
    let summary = build_finalize_with_payment_draft_inner(
        &conn, &ctx, NAME, &pay, 3_000_000, 10, &coin, &ns, &rblock,
    )
    .unwrap();
    assert_eq!(summary.action, "finalize-with-payment");
}

#[test]
fn finalize_with_payment_rejects_non_transfer_coin() {
    let (conn, ctx, _coin, rblock, pay) = setup(5_000_000, 20_000_000);
    let ns = closed_name_state(false);
    // Owner coin not in TRANSFER (no covenant_json).
    let bad_coin = NameCoin {
        txid: "cc".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: ctx.change_address.clone(),
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_REVEAL as i64,
        covenant_json: None,
        name_height: Some(1000),
    };
    let err = build_finalize_with_payment_draft_inner(
        &conn, &ctx, NAME, &pay, 3_000_000, 10, &bad_coin, &ns, &rblock,
    )
    .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("not in transfer"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn finalize_with_payment_rejects_too_few_covenant_items() {
    let (conn, ctx, _coin, rblock, pay) = setup(5_000_000, 20_000_000);
    let ns = closed_name_state(false);
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [nh_hex, "e8030000", "00"]
    })
    .to_string();
    let bad_coin = NameCoin {
        txid: "dd".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: ctx.change_address.clone(),
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_TRANSFER as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    };
    let err = build_finalize_with_payment_draft_inner(
        &conn, &ctx, NAME, &pay, 3_000_000, 10, &bad_coin, &ns, &rblock,
    )
    .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("not a TRANSFER"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn finalize_with_payment_rejects_non_p2wpkh_target() {
    let (conn, ctx, _coin, rblock, pay) = setup(5_000_000, 20_000_000);
    let ns = closed_name_state(false);
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    // version 01 + 32-byte hash → not p2wpkh.
    let cov_json = serde_json::json!({
        "type": sync::COV_TRANSFER,
        "items": [nh_hex, "e8030000", "01", "bb".repeat(32)]
    })
    .to_string();
    let bad_coin = NameCoin {
        txid: "ee".repeat(32),
        vout: 0,
        value: 5_000_000,
        address: ctx.change_address.clone(),
        branch: 0,
        child_index: 0,
        covenant_type: sync::COV_TRANSFER as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    };
    let err = build_finalize_with_payment_draft_inner(
        &conn, &ctx, NAME, &pay, 3_000_000, 10, &bad_coin, &ns, &rblock,
    )
    .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("p2wpkh"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn finalize_with_payment_rejects_invalid_payment_address() {
    let (conn, ctx, coin, rblock, _pay) = setup(5_000_000, 20_000_000);
    let ns = closed_name_state(false);
    // A testnet-style / malformed address that won't decode to a program on
    // mainnet.
    let err = build_finalize_with_payment_draft_inner(
        &conn,
        &ctx,
        NAME,
        "not_a_valid_address",
        3_000_000,
        10,
        &coin,
        &ns,
        &rblock,
    )
    .unwrap_err();
    // A malformed address either fails to decode (Crypto/Other) or decodes to
    // an empty program (InvalidInput). Any error is acceptable; the key
    // invariant is that no draft is persisted.
    assert!(
        matches!(
            err,
            AppError::InvalidInput(_) | AppError::Crypto(_) | AppError::Other(_)
        ),
        "got {err:?}"
    );

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 0);
}

#[test]
fn finalize_with_payment_insufficient_funds() {
    let (conn, ctx, coin, rblock, pay) = setup(5_000_000, 1);
    let ns = closed_name_state(false);
    let err = build_finalize_with_payment_draft_inner(
        &conn, &ctx, NAME, &pay, 3_000_000, 100, &coin, &ns, &rblock,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        AppError::InvalidInput(_) | AppError::Other(_)
    ));

    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 0);
}
