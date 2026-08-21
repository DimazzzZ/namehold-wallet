//! Tests for `build_register_draft_inner` — the pure inner logic of the
//! `build_register_draft` Tauri command, extracted so it can be exercised
//! without a `State<AppState>` or a live node.
//!
//! `build_register_draft` finalizes a won auction: given the owner coin
//! (the winning REVEAL coin, tracked in `tracked_name_states`), it spends
//! that coin as a name input, emits a REGISTER covenant output locking the
//! auction's clearing price (name value), encodes optional DNS records into
//! the resource field, funds any fee from spendable coins, and persists the
//! draft (reserving the owner coin + any funding coin).
//!
//! The two async RPC calls (`fetch_name_state` for `ns` and `renewal_block`
//! for the renewal-block hash) + the owner-coin lookup stay in the thin
//! `#[tauri::command]` wrapper; the inner function takes the resolved
//! `NameState`, owner `NameCoin`, and renewal block hash directly, so these
//! tests build those fixtures and need no mock RPC.

use std::collections::HashMap;

use crate::commands::names::{build_register_draft_inner, Ctx, NameState};
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

/// A CLOSED-phase name state at auction-start height 1000 with a clearing
/// price of 5M doos (the amount REGISTER will lock).
fn closed_name_state_with_price(price: u64) -> NameState {
    NameState {
        height: 1000,
        value: price,
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

/// Seed the owner coin (the winning REVEAL coin) into tracked_utxos and
/// tracked_name_states so it can be looked up as the name's owner.
fn seed_owner_coin(conn: &rusqlite::Connection, txid: &str, value: i64, address: &str) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    // The owner coin is typically a COV_REVEAL (winning bid's reveal coin).
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

    // Register the name in tracked_name_states, pointing to this owner coin.
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, ?3, 'CLOSED', ?4, 0, 1000)",
        rusqlite::params![PROFILE, NAME, nh_hex, txid],
    )
    .unwrap();
}

/// Build the fixture owner `NameCoin` (the winning REVEAL coin the REGISTER spends).
fn owner_coin(txid: &str, addr: &str, value: u64) -> NameCoin {
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

/// Full setup: profile, funding coin, owner coin (in DB + tracked_name_states),
/// and a `Ctx` with real derived receive/change addresses. Returns the Ctx plus
/// the fixture owner `NameCoin` and renewal block hash to pass into the inner.
fn setup(owner_value: u64, funding_value: i64) -> (rusqlite::Connection, Ctx, NameCoin, [u8; 32]) {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();

    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, funding_value, &recv0.address);

    let owner_txid = "bb".repeat(32);
    seed_owner_coin(&conn, &owner_txid, owner_value as i64, &recv0.address);

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
    let coin = owner_coin(&owner_txid, &recv0.address, owner_value);
    let rblock = [0x55u8; 32]; // arbitrary renewal block hash
    (conn, ctx, coin, rblock)
}

#[test]
fn build_register_draft_succeeds_and_persists() {
    let (conn, ctx, coin, rblock) = setup(10_000_000, 10_000_000);
    let ns = closed_name_state_with_price(5_000_000);

    let summary =
        build_register_draft_inner(&conn, &ctx, NAME, None, Some(10), &ns, &coin, &rblock).unwrap();
    assert_eq!(summary.action, "register");

    // The owner coin is reserved as the register's name input.
    let owner_reserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_REVEAL as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        owner_reserved, 1,
        "the owner coin must be reserved as the register name input"
    );
}

#[test]
fn build_register_draft_locks_clearing_price() {
    let (conn, ctx, coin, rblock) = setup(10_000_000, 10_000_000);
    let ns = closed_name_state_with_price(5_000_000);

    let summary =
        build_register_draft_inner(&conn, &ctx, NAME, None, Some(10), &ns, &coin, &rblock).unwrap();
    // The register output value equals the clearing price (locked).
    let send_total = summary
        .summary
        .get("sendTotalDoos")
        .and_then(|v| v.as_i64());
    assert_eq!(
        send_total,
        Some(5_000_000),
        "register locks the clearing price"
    );
}

#[test]
fn build_register_draft_encodes_dns_records() {
    let (conn, ctx, coin, rblock) = setup(10_000_000, 10_000_000);
    let ns = closed_name_state_with_price(5_000_000);

    // Handshake resource records use HNS record types (TXT/NS/GLUE/DS/…),
    // not standard DNS A records. Use a TXT record here.
    let records = vec![serde_json::json!({
        "type": "TXT",
        "txt": ["hello world"]
    })];

    let summary = build_register_draft_inner(
        &conn,
        &ctx,
        NAME,
        Some(&records),
        Some(10),
        &ns,
        &coin,
        &rblock,
    )
    .unwrap();
    assert_eq!(summary.action, "register");
}

#[test]
fn build_register_draft_uses_explicit_fee_rate() {
    let (conn, ctx, coin, rblock) = setup(10_000_000, 10_000_000);
    let ns = closed_name_state_with_price(5_000_000);

    let summary =
        build_register_draft_inner(&conn, &ctx, NAME, None, Some(500), &ns, &coin, &rblock)
            .unwrap();
    assert_eq!(summary.action, "register");
}

#[test]
fn build_register_draft_fails_with_insufficient_funds() {
    // owner value == clearing price (no slack) + dust funding → the fee
    // can't be covered, so the plan fails and nothing is persisted.
    let (conn, ctx, coin, rblock) = setup(5_000_000, 1);
    let ns = closed_name_state_with_price(5_000_000);

    let err = build_register_draft_inner(&conn, &ctx, NAME, None, Some(100), &ns, &coin, &rblock)
        .unwrap_err();
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
