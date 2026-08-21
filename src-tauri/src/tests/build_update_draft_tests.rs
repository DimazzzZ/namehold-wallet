//! Tests for `build_update_draft_inner` — the pure inner logic of the
//! `build_update_draft` Tauri command, extracted so it can be exercised
//! without a `State<AppState>` or a live node.
//!
//! `build_update_draft` rewrites the DNS resource records on a name we already
//! own: it spends the owner coin as a name input, emits an UPDATE covenant
//! output that keeps the full owner-coin value on the name (no price change)
//! but carries the new encoded resource, funds any fee from spendable coins,
//! and persists the draft (reserving the owner coin + any funding coin).
//!
//! The async node RPC (`fetch_name_state`) + the owner-coin lookup stay in the
//! thin `#[tauri::command]` wrapper; the inner function takes the resolved
//! `NameState` and owner `NameCoin` directly, so these tests build those
//! fixtures and need no mock RPC.

use std::collections::HashMap;

use crate::commands::names::{build_update_draft_inner, Ctx, NameState};
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

/// A CLOSED-phase name state at auction-start height 1000. `ns.value` is
/// ignored by UPDATE (unlike REGISTER); it's only referenced via `ns.height`.
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

/// Seed the owner coin (a REGISTER/UPDATE-shaped coin holding the name) into
/// tracked_utxos and tracked_name_states so it can be reserved as a name input.
fn seed_owner_coin(conn: &rusqlite::Connection, txid: &str, value: i64, address: &str) {
    let nh_hex = hex::encode(names::hash_name(NAME).unwrap());
    // The owner coin here is a REVEAL-shaped record; UPDATE only needs the row
    // to exist as unspent + reservable, not a specific covenant type.
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
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, ?3, 'CLOSED', ?4, 0, 1000)",
        rusqlite::params![PROFILE, NAME, nh_hex, txid],
    )
    .unwrap();
}

/// Build the fixture owner `NameCoin` the update spends as its name input.
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
/// and a `Ctx` with real derived receive/change addresses.
fn setup(owner_value: u64, funding_value: i64) -> (rusqlite::Connection, Ctx, NameCoin) {
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
    (conn, ctx, coin)
}

/// A valid Handshake TXT record — the simplest resource record type.
fn txt_records() -> Vec<serde_json::Value> {
    vec![serde_json::json!({ "type": "TXT", "txt": ["hello world"] })]
}

#[test]
fn build_update_draft_succeeds_and_persists() {
    let (conn, ctx, coin) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state();
    let records = txt_records();

    let summary =
        build_update_draft_inner(&conn, &ctx, NAME, &records, Some(10), &ns, &coin).unwrap();
    assert_eq!(summary.action, "update");

    // The owner coin is reserved as the update's name input.
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
        "the owner coin must be reserved as the update name input"
    );
}

#[test]
fn build_update_draft_keeps_full_owner_coin_value() {
    let (conn, ctx, coin) = setup(7_500_000, 10_000_000);
    let ns = closed_name_state();
    let records = txt_records();

    let summary =
        build_update_draft_inner(&conn, &ctx, NAME, &records, Some(10), &ns, &coin).unwrap();
    // UPDATE preserves the full owner-coin value on the name (no price change).
    let send_total = summary
        .summary
        .get("sendTotalDoos")
        .and_then(|v| v.as_i64());
    assert_eq!(
        send_total,
        Some(7_500_000),
        "update keeps the full owner-coin value on the name"
    );
}

#[test]
fn build_update_draft_encodes_multiple_records() {
    let (conn, ctx, coin) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state();
    let records = vec![
        serde_json::json!({ "type": "TXT", "txt": ["first"] }),
        serde_json::json!({ "type": "NS", "ns": "ns1.example." }),
    ];

    let summary =
        build_update_draft_inner(&conn, &ctx, NAME, &records, Some(10), &ns, &coin).unwrap();
    assert_eq!(summary.action, "update");
}

#[test]
fn build_update_draft_uses_explicit_fee_rate() {
    let (conn, ctx, coin) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state();
    let records = txt_records();

    let summary =
        build_update_draft_inner(&conn, &ctx, NAME, &records, Some(750), &ns, &coin).unwrap();
    assert_eq!(summary.action, "update");
}

#[test]
fn build_update_draft_rejects_invalid_records() {
    let (conn, ctx, coin) = setup(5_000_000, 10_000_000);
    let ns = closed_name_state();
    // Standard DNS `A` records are NOT valid Handshake resource types
    // (Handshake uses TXT/NS/GLUE/SYNTH/DS) — encoding must reject them.
    let bad = vec![serde_json::json!({ "type": "A", "address": "1.2.3.4" })];

    let err = build_update_draft_inner(&conn, &ctx, NAME, &bad, Some(10), &ns, &coin).unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("unsupported record type")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    // On failure, no draft should have been persisted.
    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 0);
}

#[test]
fn build_update_draft_fails_with_insufficient_funds() {
    // Owner coin can't grow (UPDATE preserves value), so the fee must come
    // from funding. A dust funding coin can't cover it → the plan fails.
    let (conn, ctx, coin) = setup(5_000_000, 1);
    let ns = closed_name_state();
    let records = txt_records();

    let err =
        build_update_draft_inner(&conn, &ctx, NAME, &records, Some(100), &ns, &coin).unwrap_err();
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
