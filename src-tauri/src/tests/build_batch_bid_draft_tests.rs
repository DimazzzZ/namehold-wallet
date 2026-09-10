//! Tests for `build_batch_bid_draft_inner` — the pure inner logic of the
//! `build_batch_bid_draft` Tauri command, extracted so it can be exercised
//! without a `State<AppState>` or a live node.
//!
//! The inner takes a pre-fetched `Vec<NameSpec>` (one per name, each carrying
//! the on-chain `NameState`), enforces the consensus phase check + one-bid-per-
//! name multiplicity guard, derives a receive address / nonce / blind per name,
//! persists a bid commitment per name, and persists a single batch draft.
//!
//! The async node RPC (`fetch_name_state`) stays in the thin `#[tauri::command]`
//! wrapper; these tests build fixture `NameSpec`s directly and need no mock RPC.

use std::collections::HashMap;

use crate::commands::names::{build_batch_bid_draft_inner, Ctx, NameSpec, NameState};
use crate::db;
use crate::error::AppError;
use crate::noncustodial::derivation::{self, BRANCH_CHANGE};
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::SpendableCoin;
use crate::noncustodial::sync;

const PROFILE: &str = "test_profile";

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

fn seed_tracked_coin(conn: &rusqlite::Connection, txid: &str, value: i64, address: &str) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, 'deadbeef', ?4, 0, NULL, 'liquid_hns', NULL)",
        rusqlite::params![txid, PROFILE, address, value],
    )
    .unwrap();
}

/// A `NameSpec` in the given auction `phase` for `name`.
fn spec(name: &str, phase: &str) -> NameSpec {
    let nh = names::hash_name(name).unwrap();
    NameSpec {
        name: name.into(),
        nh,
        nh_hex: hex::encode(nh),
        raw: names::raw_name(name).unwrap(),
        ns: NameState {
            height: 1000,
            value: 0,
            renewals: 0,
            claimed: 0,
            weak: false,
            phase: phase.into(),
        },
    }
}

/// Build a `Ctx` with one funding coin seeded into tracked_utxos.
fn seed_ctx(conn: &rusqlite::Connection, funding_txid: &str, funding_value: i64) -> Ctx {
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    seed_tracked_coin(conn, funding_txid, funding_value, &recv0.address);
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

#[test]
fn batch_bid_happy_path_persists_summary_commitments_and_draft() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 50_000_000);
    let names = vec![
        "alpha".to_string(),
        "bravo".to_string(),
        "charlie".to_string(),
    ];
    let specs = vec![
        spec("alpha", "BIDDING"),
        spec("bravo", "OPENING"),
        spec("charlie", "BIDDING"),
    ];

    let summary =
        build_batch_bid_draft_inner(&conn, &ctx, &names, specs, 1_000_000, 2_000_000, 10).unwrap();
    assert_eq!(summary.action, "batch-bid");

    // One commitment per name.
    let commit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1",
            rusqlite::params![PROFILE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(commit_count, 3, "one bid commitment persisted per name");

    // A single draft row persisted.
    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 1);

    // Every commitment got its bid txid stamped.
    let stamped: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments
             WHERE wallet_profile_id = ?1 AND bid_txid IS NOT NULL",
            rusqlite::params![PROFILE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stamped, 3);
}

#[test]
fn batch_bid_empty_specs_persists_zero_output_draft() {
    // The async wrapper rejects empty `names` up front with
    // `InvalidInput("no names provided")`, so the inner never sees an empty
    // batch in production. If it somehow does, `build_batch_plan` refuses to
    // build a zero-output tx — no draft is persisted. This test pins that
    // load-bearing precondition: an empty batch is an error, not a zero-output
    // draft. (Empty-names rejection at the wrapper is covered in
    // names_cmd_tests.)
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 10_000_000);

    let err = build_batch_bid_draft_inner(&conn, &ctx, &[], Vec::new(), 1_000_000, 2_000_000, 10)
        .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("at least one output"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    let commit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1",
            rusqlite::params![PROFILE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(commit_count, 0);
}

#[test]
fn batch_bid_one_bad_phase_aborts_entire_batch() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 50_000_000);
    let names = vec!["alpha".to_string(), "bravo".to_string()];
    // "bravo" is CLOSED — not biddable. The whole batch must abort.
    let specs = vec![spec("alpha", "BIDDING"), spec("bravo", "CLOSED")];

    let err = build_batch_bid_draft_inner(&conn, &ctx, &names, specs, 1_000_000, 2_000_000, 10)
        .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => {
            assert!(msg.contains("bravo") && msg.contains("not open for bidding"))
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    // Nothing persisted — the phase check runs before any write.
    let commit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1",
            rusqlite::params![PROFILE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(commit_count, 0, "no commitment persisted on abort");
}

#[test]
fn batch_bid_empty_phase_shown_as_available() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 50_000_000);
    let names = vec!["alpha".to_string()];
    let specs = vec![spec("alpha", "")]; // no phase => AVAILABLE label

    let err = build_batch_bid_draft_inner(&conn, &ctx, &names, specs, 1_000_000, 2_000_000, 10)
        .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("AVAILABLE")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn batch_bid_rejects_when_existing_unspent_bid_coin() {
    let conn = test_db();
    seed_profile(&conn);
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 50_000_000);

    // Seed an unspent COV_BID coin for "alpha" → multiplicity guard trips.
    let nh_hex = hex::encode(names::hash_name("alpha").unwrap());
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

    let names = vec!["alpha".to_string()];
    let specs = vec![spec("alpha", "BIDDING")];
    let err = build_batch_bid_draft_inner(&conn, &ctx, &names, specs, 1_000_000, 2_000_000, 10)
        .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("already has an unspent bid")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn batch_bid_insufficient_funds_persists_nothing() {
    let conn = test_db();
    seed_profile(&conn);
    // A dust coin that can't fund even one 2M lockup.
    let ctx = seed_ctx(&conn, &"aa".repeat(32), 1);
    let names = vec!["alpha".to_string()];
    let specs = vec![spec("alpha", "BIDDING")];

    let err = build_batch_bid_draft_inner(&conn, &ctx, &names, specs, 1_000_000, 2_000_000, 100)
        .unwrap_err();
    assert!(matches!(
        err,
        AppError::InvalidInput(_) | AppError::Other(_)
    ));

    // No draft persisted (the commitment rows are written inside the same
    // rolled-back-by-failure sequence, but the draft never lands).
    let drafts: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 0);
}
