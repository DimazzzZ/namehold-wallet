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
    // Coin selection joins on the wallet's own addresses.
    conn.execute(
        "INSERT OR IGNORE INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, 'deadbeef', '00')",
        rusqlite::params![PROFILE, address],
    )
    .unwrap();
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
        profile_kind: "mnemonic_hot".into(),
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

/// An owner covenant without `items`, or whose target is not hex, names the
/// name it refuses, and no draft is persisted for any name of the batch.
#[test]
fn batch_finalize_refuses_an_owner_covenant_without_a_readable_target() {
    for (items, want) in [
        (None, "'bravo' owner coin has no covenant items"),
        (Some("zz"), "'bravo' bad transfer target"),
    ] {
        let (conn, ctx, mut per_name, rblock) = setup(&["alpha", "bravo"]);
        let nh_hex = hex::encode(names::hash_name("bravo").unwrap());
        let cov = match items {
            None => serde_json::json!({"type": sync::COV_TRANSFER}),
            Some(target) => serde_json::json!({
                "type": sync::COV_TRANSFER,
                "items": [nh_hex, "e8030000", "00", target]
            }),
        };
        per_name[1].3.covenant_json = Some(cov.to_string());
        let err = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 10).unwrap_err();
        assert!(
            matches!(&err, AppError::InvalidInput(m) if m.contains(want)),
            "{err:?}"
        );
        let drafts: i64 = conn
            .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(drafts, 0, "{want}");
    }
}

/// Each FINALIZE carries its own name's flags byte (`items[3]`): 1 for a weak
/// name, 0 otherwise. hsd checks it against the name state, so a weak name
/// finalized with 0 (or the reverse) is refused.
#[test]
fn batch_finalize_writes_each_names_weak_flag() {
    let (conn, ctx, mut per_name, rblock) = setup(&["alpha", "bravo"]);
    per_name[1].4.weak = true;
    build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 10).unwrap();

    let plan_json: String = conn
        .query_row(
            "SELECT signing_inputs_json FROM wallet_tx_drafts LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let plan: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    let mut flags = HashMap::new();
    for out in plan["outputs"].as_array().expect("plan has outputs") {
        if out["covenant_type"].as_u64() != Some(sync::COV_FINALIZE as u64) {
            continue;
        }
        let items = out["covenant_items_hex"].as_array().unwrap();
        flags.insert(
            items[0].as_str().unwrap().to_string(),
            items[3].as_str().unwrap().to_string(),
        );
    }
    let nh = |n: &str| hex::encode(names::hash_name(n).unwrap());
    assert_eq!(flags.len(), 2, "one FINALIZE per name");
    assert_eq!(flags[&nh("alpha")], "00");
    assert_eq!(flags[&nh("bravo")], "01");
}

// ---------------------------------------------------------------------------
// A series out of coins (2026-11-02, the 46 names left of 546): five batches
// took the five free coins, their change was not synced yet, and the sixth
// read "Insufficient HNS balance" with 5.57 HNS on the way back.
// ---------------------------------------------------------------------------

const WAITING: &str = "insufficient free coins";

/// The first batch built and put in `status`, its coin held. A second batch of
/// another name then runs with no coin left, as a fresh `load_ctx` would see.
fn second_batch_after_first_in(
    status: &str,
) -> Result<crate::noncustodial::types::TxDraftSummary, AppError> {
    let (conn, mut ctx, per_name, rblock) = setup(&["aaa", "bbb"]);
    let mut per_name = per_name.into_iter();
    let first = vec![per_name.next().unwrap()];
    let second = vec![per_name.next().unwrap()];
    let draft = build_batch_finalize_draft_inner(&conn, &ctx, first, &rblock, 1).unwrap();
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = ?2",
        rusqlite::params![status, draft.id],
    )
    .unwrap();
    ctx.funding =
        crate::noncustodial::send::load_spendable_coins(&conn, PROFILE, None, Network::Main)
            .unwrap();
    assert!(
        ctx.funding.is_empty(),
        "the first batch holds the only coin"
    );
    build_batch_finalize_draft_inner(&conn, &ctx, second, &rblock, 1)
}

fn message(r: Result<crate::noncustodial::types::TxDraftSummary, AppError>) -> String {
    match r {
        Err(AppError::InvalidInput(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn batch_after_a_sent_batch_says_its_coins_are_held() {
    for status in ["broadcasted", "broadcast_pending", "confirmed"] {
        let m = message(second_batch_after_first_in(status));
        assert!(m.starts_with(WAITING), "{status}: {m}");
        assert!(m.contains("1 coin(s)"), "{status}: {m}");
    }
}

#[test]
fn batch_after_an_unsent_draft_is_not_told_to_wait() {
    // Nothing is on its way back: waiting would never help.
    for status in ["draft", "signed"] {
        let m = message(second_batch_after_first_in(status));
        assert!(m.starts_with("insufficient funds"), "{status}: {m}");
    }
}

#[test]
fn batch_with_no_coins_at_all_stays_plain_insufficient_funds() {
    let (conn, mut ctx, per_name, rblock) = setup(&["aaa"]);
    ctx.funding.clear();
    let m = message(build_batch_finalize_draft_inner(
        &conn, &ctx, per_name, &rblock, 1,
    ));
    assert!(m.starts_with("insufficient funds"), "{m}");
}

#[test]
fn coins_held_count_ignores_spent_coins_name_coins_and_other_wallets() {
    use crate::db::queries::count_coins_held_by_sent_drafts as held;
    let (conn, ctx, per_name, rblock) = setup(&["aaa"]);
    let draft = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 1).unwrap();
    assert_eq!(held(&conn, PROFILE).unwrap(), 0, "not sent yet");
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = 'broadcasted' WHERE id = ?1",
        rusqlite::params![draft.id],
    )
    .unwrap();
    // The draft also reserves the name's coin; only the liquid one pays fees.
    assert_eq!(held(&conn, PROFILE).unwrap(), 1);
    assert_eq!(held(&conn, "another_wallet").unwrap(), 0);
    conn.execute(
        "UPDATE tracked_utxos SET spent_by_txid = 'spent' WHERE spend_class = 'liquid_hns'",
        [],
    )
    .unwrap();
    assert_eq!(
        held(&conn, PROFILE).unwrap(),
        0,
        "synced: its change is back"
    );
}

#[test]
fn explain_waiting_coins_leaves_every_other_error_alone() {
    let (conn, ctx, per_name, rblock) = setup(&["aaa"]);
    let draft = build_batch_finalize_draft_inner(&conn, &ctx, per_name, &rblock, 1).unwrap();
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = 'broadcasted' WHERE id = ?1",
        rusqlite::params![draft.id],
    )
    .unwrap();
    let explain = crate::noncustodial::send::explain_waiting_coins;
    let e = explain(
        &conn,
        PROFILE,
        AppError::InvalidInput("too many coins".into()),
    );
    assert!(matches!(e, AppError::InvalidInput(m) if m == "too many coins"));
    let e = explain(&conn, PROFILE, AppError::Rpc("insufficient funds".into()));
    assert!(
        matches!(e, AppError::Rpc(_)),
        "only coin selection's own refusal"
    );
}

/// A whole series, coin by coin: three free coins carry three batches, the
/// fourth is told to wait, and once a sync brings the first one's change the
/// fourth builds on that change.
#[test]
fn a_series_runs_until_the_coins_run_out_and_resumes_on_the_change() {
    let (conn, mut ctx, per_name, rblock) = setup(&["aaa", "bbb", "ccc", "ddd"]);
    let addr: String = conn
        .query_row("SELECT address FROM tracked_utxos LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    seed_liquid_coin(&conn, &"ab".repeat(32), 40_000_000, &addr);
    seed_liquid_coin(&conn, &"ac".repeat(32), 30_000_000, &addr);
    let load = |conn: &rusqlite::Connection| {
        crate::noncustodial::send::load_spendable_coins(conn, PROFILE, None, Network::Main).unwrap()
    };
    let mut names = per_name.into_iter();
    let mut first_id = String::new();
    for i in 0..3 {
        ctx.funding = load(&conn);
        assert_eq!(
            ctx.funding.len(),
            3 - i,
            "one coin left per batch not yet built"
        );
        let d =
            build_batch_finalize_draft_inner(&conn, &ctx, vec![names.next().unwrap()], &rblock, 1)
                .unwrap();
        conn.execute(
            "UPDATE wallet_tx_drafts SET status = 'broadcasted' WHERE id = ?1",
            rusqlite::params![d.id],
        )
        .unwrap();
        if i == 0 {
            first_id = d.id;
        }
    }
    ctx.funding = load(&conn);
    let fourth = names.next().unwrap();
    let m = message(build_batch_finalize_draft_inner(
        &conn,
        &ctx,
        vec![fourth.clone()],
        &rblock,
        1,
    ));
    assert!(m.starts_with(WAITING) && m.contains("3 coin(s)"), "{m}");

    // A sync: the first batch's coin reads spent, its change arrives.
    conn.execute(
        "UPDATE tracked_utxos SET spent_by_txid = 'spent' WHERE reserved_by_draft_id = ?1",
        rusqlite::params![first_id],
    )
    .unwrap();
    seed_liquid_coin(&conn, &"cd".repeat(32), 49_000_000, &addr);
    ctx.funding = load(&conn);
    assert_eq!(ctx.funding.len(), 1);
    build_batch_finalize_draft_inner(&conn, &ctx, vec![fourth], &rblock, 1)
        .expect("the change pays for the batch that had to wait");
}
