//! Tests for `build_batch_transfer_draft_inner` — the pure inner logic of the
//! `build_batch_transfer_draft` Tauri command, extracted so it can be exercised
//! without a `State<AppState>` or a live node.
//!
//! The inner takes a pre-resolved `Vec<(name, name_hash, owner_coin, state)>`
//! (the wrapper's per-name `owner_coin_and_state` prefetch) + a single shared
//! pre-decoded recipient `(version, program)`, builds one TRANSFER covenant
//! output per name spending the owner coin, and persists a single batch draft
//! reserving all inputs. The output value stays at the current owner address;
//! the recipient lives in each name's covenant items.

use std::collections::HashMap;

use crate::commands::names::{build_batch_transfer_draft_inner, Ctx, NameState};
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

/// Seed an owner coin (name control) so `persist_with_conn` can reserve it.
fn seed_owner_coin(conn: &rusqlite::Connection, name: &str, txid: &str, value: i64, address: &str) {
    let nh_hex = hex::encode(names::hash_name(name).unwrap());
    let cov_json = format!(
        r#"{{"type":{},"items":["{}"]}}"#,
        sync::COV_REGISTER,
        nh_hex
    );
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
            sync::COV_REGISTER as i64,
            cov_json
        ],
    )
    .unwrap();
}

fn owner_coin(name: &str, txid: &str, addr: &str, value: u64) -> NameCoin {
    let nh_hex = hex::encode(names::hash_name(name).unwrap());
    let cov_json = format!(
        r#"{{"type":{},"items":["{}"]}}"#,
        sync::COV_REGISTER,
        nh_hex
    );
    NameCoin {
        txid: txid.into(),
        vout: 0,
        value,
        address: addr.into(),
        branch: derivation::BRANCH_RECEIVE,
        child_index: 0,
        covenant_type: sync::COV_REGISTER as i64,
        covenant_json: Some(cov_json),
        name_height: Some(1000),
    }
}

fn name_state() -> NameState {
    NameState {
        height: 1000,
        value: 0,
        renewals: 3,
        claimed: 1,
        weak: false,
        phase: "CLOSED".into(),
    }
}

struct Fixture {
    conn: rusqlite::Connection,
    ctx: Ctx,
    per_name: Vec<(String, [u8; 32], NameCoin, NameState)>,
    recipient: String,
    version: u8,
    program: Vec<u8>,
    owner_addr: String,
}

/// Setup: profile, funding coin, N owner coins, a `Ctx`, and a shared
/// recipient address decoded to `(version, program)` (derived at receive
/// index 1, distinct from the owner address at index 0).
fn setup(names_in: &[&str]) -> Fixture {
    let conn = test_db();
    seed_profile(&conn);
    let network = Network::Main;
    let xpub = test_xpub();
    let change = derivation::derive_one(network, &xpub, BRANCH_CHANGE, 0).unwrap();
    let recv0 = derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 0).unwrap();
    let recipient_addr =
        derivation::derive_one(network, &xpub, derivation::BRANCH_RECEIVE, 1).unwrap();
    let (version, program) = address::decode(network, &recipient_addr.address).unwrap();

    let funding_txid = "aa".repeat(32);
    seed_liquid_coin(&conn, &funding_txid, 50_000_000, &recv0.address);

    let mut per_name = Vec::new();
    for (i, name) in names_in.iter().enumerate() {
        let txid = format!("{:02x}", i + 0xb0).repeat(32);
        seed_owner_coin(&conn, name, &txid, 5_000_000, &recv0.address);
        let nh = names::hash_name(name).unwrap();
        per_name.push((
            name.to_string(),
            nh,
            owner_coin(name, &txid, &recv0.address, 5_000_000),
            name_state(),
        ));
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
        node: crate::noncustodial::rpc::NodeRpcClient::new(
            "http://127.0.0.1:1",
            "",
            crate::noncustodial::rpc::ChainSource::LocalNode,
        ),
    };

    Fixture {
        conn,
        ctx,
        per_name,
        recipient: recipient_addr.address,
        version,
        program,
        owner_addr: recv0.address,
    }
}

#[test]
fn batch_transfer_happy_path_persists_draft() {
    let f = setup(&["alpha", "bravo"]);
    let names: Vec<String> = vec!["alpha".into(), "bravo".into()];
    let summary = build_batch_transfer_draft_inner(
        &f.conn,
        &f.ctx,
        &names,
        f.per_name,
        &f.recipient,
        f.version,
        &f.program,
        10,
    )
    .unwrap();
    assert_eq!(summary.action, "batch-transfer");
    assert_eq!(
        summary
            .summary
            .get("recipientAddress")
            .and_then(|v| v.as_str()),
        Some(f.recipient.as_str()),
        "batch draft carries the shared recipient"
    );

    let drafts: i64 = f
        .conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drafts, 1);

    // Both owner coins reserved as name inputs.
    let reserved: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
             WHERE reserved_by_draft_id IS NOT NULL AND covenant_type = ?1",
            rusqlite::params![sync::COV_REGISTER as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 2);

    // The draft records the full name list.
    let name_list = summary.summary.get("nameList");
    assert!(name_list.is_some(), "batch draft records the name list");
}

#[test]
fn batch_transfer_uses_owner_address_for_output() {
    // TRANSFER semantics: the output value stays at the current owner address;
    // the recipient lives only in the covenant items. Guard both by parsing
    // the persisted plan.
    let f = setup(&["alpha", "bravo"]);
    let names: Vec<String> = vec!["alpha".into(), "bravo".into()];
    let owner_addr = f.owner_addr.clone();
    let program_hex = hex::encode(&f.program);
    let version = f.version;

    build_batch_transfer_draft_inner(
        &f.conn,
        &f.ctx,
        &names,
        f.per_name,
        &f.recipient,
        f.version,
        &f.program,
        10,
    )
    .unwrap();

    let plan_json: String = f
        .conn
        .query_row(
            "SELECT signing_inputs_json FROM wallet_tx_drafts LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let plan: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    let outputs = plan["outputs"].as_array().expect("plan has outputs");

    // Every TRANSFER covenant output stays at the owner address and carries the
    // shared recipient (version byte + program) in its covenant items.
    let mut transfer_outputs = 0;
    for out in outputs {
        let cov_type = out["covenant_type"].as_u64().unwrap_or(0);
        if cov_type != sync::COV_TRANSFER as u64 {
            continue;
        }
        transfer_outputs += 1;
        assert_eq!(
            out["address"].as_str().unwrap(),
            owner_addr,
            "transfer output must stay at the current owner address"
        );
        let items: Vec<String> = out["covenant_items_hex"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        // items: [name_hash, height, version, program]
        assert_eq!(
            items[2],
            format!("{:02x}", version),
            "version byte in covenant"
        );
        assert_eq!(items[3], program_hex, "recipient program in covenant");
    }
    assert_eq!(transfer_outputs, 2, "one TRANSFER output per name");
}

#[test]
fn batch_transfer_single_name() {
    let f = setup(&["solo"]);
    let names: Vec<String> = vec!["solo".into()];
    let summary = build_batch_transfer_draft_inner(
        &f.conn,
        &f.ctx,
        &names,
        f.per_name,
        &f.recipient,
        f.version,
        &f.program,
        20,
    )
    .unwrap();
    assert_eq!(summary.action, "batch-transfer");
}

#[test]
fn batch_transfer_empty_errors() {
    // The async wrapper rejects empty `names` up front; the inner also errors
    // because `build_batch_plan` requires at least one output.
    let f = setup(&[]);
    let err = build_batch_transfer_draft_inner(
        &f.conn,
        &f.ctx,
        &[],
        Vec::new(),
        &f.recipient,
        f.version,
        &f.program,
        10,
    )
    .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("at least one output"), "got {msg}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}
