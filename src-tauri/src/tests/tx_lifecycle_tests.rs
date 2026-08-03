//! Full transaction lifecycle integration tests against a mock hsrd node.
//!
//! These drive the REAL `#[tauri::command]` functions (`build_send_hns_draft`,
//! `sign_tx_draft`, `broadcast_tx_draft`) with a managed `AppState` over a fully
//! -migrated in-memory DB and strict `mockito` wallet-RPC v1 envelopes. They prove the
//! end-to-end orchestration — coin selection → draft persistence → signing →
//! broadcast/status — behaves correctly, and that every guard that protects
//! funds (watch-only, non-positive amount, profile mismatch, broadcasting an
//! unsigned draft, recording a failed broadcast as failed) actually fires.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::tx::{
    broadcast_tx_draft, build_send_hns_draft, delete_tx_draft, get_write_capability,
    refresh_tx_confirmations, release_tx_draft_reservation, sign_tx_draft_confirmed,
    sign_tx_draft_inner, sync_wallet_state,
};
use crate::db;
use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::types::TxSummary;
use crate::tests::names_cmd_tests::{
    mock_chain_binding, mock_chain_tip, test_tip, wallet_error, wallet_result, TEST_CHAIN_EPOCH,
    TEST_HASH,
};
use crate::AppState;

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
const PROFILE: &str = "life1";
const COIN_TXID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn seed() -> [u8; 64] {
    hd::seed_from_mnemonic(MNEMONIC, "").unwrap()
}

fn master() -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed()).unwrap()
}

/// Account-level xpub string (m/44'/5353'/0') for the known mnemonic.
fn account_xpub() -> String {
    let path = hd::bip44_path(Network::Main, 0, 0, 0);
    let account = master().derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account).to_base58check(Network::Main)
}

/// Receive address + its script/pubkey hex for leaf 0/0.
fn leaf00() -> (String, String, String) {
    let s = seed();
    let (_sk, pk, addr) = derivation_derive(&s);
    let spk = hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

fn derivation_derive(s: &[u8]) -> (secp256k1::SecretKey, [u8; 33], String) {
    // derive_address(network, seed, account, branch, index) -> (sk, pubkey, addr)
    let (sk, pk, addr) =
        crate::noncustodial::hd::derive_address(Network::Main, s, 0, 0, 0).unwrap();
    (sk, pk, addr)
}

/// Build a fully-migrated, seeded in-memory DB with one spendable coin under a
/// non-watch-only profile owning leaf 0/0. `node_url` is stored for broadcast.
fn seeded_conn(node_url: &str, value: u64) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();

    let (addr, spk, pubkey) = leaf00();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Life",
        "mnemonic_hot",
        "mainnet",
        &account_xpub(),
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "hsrd_rpc_url", node_url).unwrap();

    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, ?3, ?4)",
        params![PROFILE, addr, spk, pubkey],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, 0, 'liquid_hns', NULL)",
        params![COIN_TXID, PROFILE, addr, spk, value as i64],
    )
    .unwrap();
    conn
}

/// Add a second spendable coin (same address) to a DB built by `seeded_conn`,
/// so coin-reservation tests can prove two drafts pick disjoint inputs
/// instead of merely running out of funds.
const COIN_TXID_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn add_second_coin(conn: &rusqlite::Connection, value: u64) {
    let (addr, spk, _pubkey) = leaf00();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, 0, 'liquid_hns', NULL)",
        params![COIN_TXID_2, PROFILE, addr, spk, value as i64],
    )
    .unwrap();
}

/// Every txid+vout reserved for a draft, read straight from `tracked_utxos`
/// (order-independent — returned as a sorted set of txids).
fn reserved_txids_for(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) -> Vec<String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT txid FROM tracked_utxos WHERE reserved_by_draft_id = ?1 ORDER BY txid")
        .unwrap();
    stmt.query_map(params![draft_id], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsrd_child: std::sync::Mutex::new(None),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn unlock(app: &tauri::App<tauri::test::MockRuntime>, profile_id: &str) {
    let state = app.state::<AppState>();
    *state.signer.lock().unwrap() = Some(SignerSession::unlock(
        profile_id.to_string(),
        Network::Main,
        master(),
        600_000,
    ));
}

fn recv_addr() -> String {
    leaf00().0
}

/// Fetch the persisted draft row for assertions about signed hex / status.
fn draft_row(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> db::queries::TxDraftRow {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    db::queries::get_tx_draft(&c, id).unwrap().unwrap()
}

fn summary_of(row: &db::queries::TxDraftRow) -> TxSummary {
    serde_json::from_str(&row.summary_json).expect("summary parses")
}

fn evidence_transaction() -> &'static (String, String) {
    static TX: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    TX.get_or_init(|| {
        let tx = crate::noncustodial::tx::Transaction::new();
        (tx.txid(), hex::encode(tx.serialize()))
    })
}

async fn mock_mempool_snapshot(server: &mut mockito::Server, tip: u32) -> mockito::Mock {
    server
        .mock("POST", "/api/v1/wallet")
        .match_body(mockito::Matcher::Regex("mempool_scripts_page".into()))
        .with_header("content-type", "application/json")
        .with_body(wallet_result(serde_json::json!({
            "chain_epoch": TEST_CHAIN_EPOCH,
            "tip": test_tip(tip),
            "instance_nonce": TEST_HASH,
            "generation": 11,
            "entries": [],
            "continuation": null
        })))
        .expect_at_least(1)
        .create_async()
        .await
}

async fn mock_fee_quote(server: &mut mockito::Server, tip: u32) -> Vec<mockito::Mock> {
    let mut mocks = vec![mock_chain_binding(server, tip).await];
    mocks.push(mock_mempool_snapshot(server, tip).await);
    mocks.push(
        server
            .mock("POST", "/api/v1/wallet")
            .match_body(mockito::Matcher::Regex("quote_transaction_fee".into()))
            .with_header("content-type", "application/json")
            .with_body(wallet_result(serde_json::json!({
                "txid": evidence_transaction().0.clone(),
                "chain_epoch": TEST_CHAIN_EPOCH,
                "tip": test_tip(tip),
                "mempool_instance_nonce": TEST_HASH,
                "mempool_generation": 11,
                "target_blocks": 6,
                "rate_atomic_units_per_1000_policy_vbytes": 1_000,
                "rate_sample_count": 1,
                "rate_source": "mempool",
                "transaction_weight": 400,
                "transaction_sigops": 1,
                "sigop_adjusted_policy_vbytes": 100,
                "minimum_policy_fee_atomic_units": 100,
                "actual_fee_atomic_units": 1_000,
                "meets_minimum_policy_fee": true,
                "minimum_policy_fee_shortfall_atomic_units": 0
            })))
            .create_async()
            .await,
    );
    mocks
}

async fn mock_broadcast_success(
    server: &mut mockito::Server,
    tip: u32,
    txid: &str,
) -> Vec<mockito::Mock> {
    let mut mocks = mock_fee_quote(server, tip).await;
    mocks.push(
        server
            .mock("POST", "/api/v1/wallet")
            .match_body(mockito::Matcher::Regex("broadcast_transaction".into()))
            .with_header("content-type", "application/json")
            .with_body(wallet_result(serde_json::json!({
                "txid": txid,
                "newly_admitted": true,
                "attempted_peers": 2,
                "queued_peers": 2,
                "failed_peers": 0
            })))
            .create_async()
            .await,
    );
    mocks
}

async fn mock_broadcast_rejection(server: &mut mockito::Server, tip: u32) -> Vec<mockito::Mock> {
    let mut mocks = mock_fee_quote(server, tip).await;
    mocks.push(
        server
            .mock("POST", "/api/v1/wallet")
            .match_body(mockito::Matcher::Regex("broadcast_transaction".into()))
            .with_header("content-type", "application/json")
            .with_body(wallet_error(
                "transaction_rejected",
                "transaction inputs are missing or spent",
            ))
            .create_async()
            .await,
    );
    mocks
}

#[derive(Clone, Copy)]
enum TestTxState {
    Unknown,
    Mempool,
    Confirmed { height: u32, confirmations: u32 },
}

async fn mock_transaction_evidence(
    server: &mut mockito::Server,
    tip: u32,
    state: TestTxState,
) -> mockito::Mock {
    let inclusion = match state {
        TestTxState::Confirmed {
            height,
            confirmations,
        } => Some(serde_json::json!({
            "block_hash": TEST_HASH,
            "height": height,
            "transaction_index": 0,
            "confirmations": confirmations
        })),
        _ => None,
    };
    let known = !matches!(state, TestTxState::Unknown);
    server
        .mock("POST", "/api/v1/wallet")
        .match_body(mockito::Matcher::Regex("transaction_evidence".into()))
        .with_header("content-type", "application/json")
        .with_body(wallet_result(serde_json::json!({
            "chain_epoch": TEST_CHAIN_EPOCH,
            "mempool_instance_nonce": TEST_HASH,
            "mempool_generation": 11,
            "tip": test_tip(tip),
            "status": match state {
                TestTxState::Unknown => "unknown",
                TestTxState::Mempool => "mempool",
                TestTxState::Confirmed { .. } => "confirmed"
            },
            "inclusion": inclusion,
            "payload": if known { "retained" } else { "absent" },
            "transaction_hex": known.then(|| evidence_transaction().1.clone())
        })))
        .create_async()
        .await
}

// --- happy path: build -> sign -> broadcast --------------------------------

#[tokio::test]
async fn full_lifecycle_build_sign_broadcast_succeeds() {
    let mut server = mockito::Server::new_async().await;
    let node_txid = "abc0000000000000000000000000000000000000000000000000000000000def";
    let _mocks = mock_broadcast_success(&mut server, 1_000, node_txid).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    // 1. Build — no unlock required; accurate fee/change preview persisted.
    let draft = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .expect("build draft");
    let draft_id = draft.id.clone();
    {
        let row = draft_row(&app, &draft_id);
        let s = summary_of(&row);
        assert_eq!(s.send_total_doos, 500_000);
        assert_eq!(s.num_inputs, 1);
        assert!(s.fee_doos > 0);
        assert_eq!(s.recipient_address.as_deref(), Some(to.as_str()));
        // Unsigned at this stage.
        assert!(row.signed_tx_hex.is_none(), "must be unsigned before sign");
    }

    // 2. Sign — requires unlock; materializes the signed hex + a real txid.
    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft_id)
        .await
        .expect("sign");
    {
        let row = draft_row(&app, &draft_id);
        assert!(row.signed_tx_hex.is_some(), "draft must be signed");
        assert!(
            summary_of(&row).txid.is_some(),
            "signed summary carries a txid"
        );
    }

    // 3. Broadcast — sends to the mock node; status + node txid recorded.
    let result = broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .expect("broadcast");
    assert_eq!(result.status, "broadcasted");
    assert_eq!(result.txid, node_txid);

    // The draft row reflects the broadcast.
    let row = draft_row(&app, &draft_id);
    assert_eq!(row.status, "broadcasted");
    assert_eq!(row.txid.as_deref(), Some(node_txid));
}

// --- failure path: node rejects -> recorded as failed, NOT sent ------------

#[tokio::test]
async fn broadcast_failure_marks_draft_failed_and_errors() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_broadcast_rejection(&mut server, 1_000).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .unwrap();
    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft.id).await.unwrap();

    let err = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect_err("broadcast must surface the node rejection");
    assert!(matches!(err, AppError::Rpc(_)), "got {err:?}");

    // Critically: the draft is marked failed, never "broadcasted".
    let stored = draft_row(&app, &draft.id);
    assert_eq!(stored.status, "failed");
    assert!(
        stored.signed_tx_hex.is_some(),
        "signed hex retained for inspection"
    );
}

// --- confirmation tracking (broadcasted -> confirmed / dropped) ------------

/// Seed a `broadcasted` draft (with a txid) for the active profile.
fn seed_broadcasted_draft(conn: &rusqlite::Connection, id: &str) {
    db::queries::insert_tx_draft(conn, id, PROFILE, "send_hns", "00", "{}", "{}").unwrap();
    db::queries::update_tx_draft_status(
        conn,
        id,
        "broadcasted",
        None,
        Some(&evidence_transaction().0),
    )
    .unwrap();
}

/// Chain readiness plus snapshot-bound transaction evidence.
async fn mock_node(
    server: &mut mockito::Server,
    tip: u32,
    state: TestTxState,
) -> Vec<mockito::Mock> {
    let mut mocks = mock_chain_tip(server, tip, 1.0).await;
    mocks.push(mock_chain_binding(server, tip).await);
    mocks.push(mock_transaction_evidence(server, tip, state).await);
    mocks
}

#[tokio::test]
async fn refresh_marks_a_mined_draft_confirmed() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(
        &mut server,
        437,
        TestTxState::Confirmed {
            height: 435,
            confirmations: 3,
        },
    )
    .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcasted_draft(&conn, "drf1");
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["nodeReachable"], serde_json::json!(true));
    assert_eq!(res["confirmed"], serde_json::json!(1));

    let row = draft_row(&app, "drf1");
    assert_eq!(row.status, "confirmed");
    assert_eq!(row.confirmation_height, Some(435));
}

#[tokio::test]
async fn refresh_marks_a_long_unfound_draft_dropped() {
    let mut server = mockito::Server::new_async().await;
    // Node reachable, but the tx is not found (evicted / never confirmed).
    let _mocks = mock_node(&mut server, 500, TestTxState::Unknown).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcasted_draft(&conn, "drf2");
    // Backdate past the (10-minute, I5) grace window so it's judged dropped,
    // not still-pending. Backdated via `updated_at`, NOT `created_at`: the
    // dropped-grace check measures from the draft's last update (when it was
    // set `broadcasted`, or reorg-reverted back to it — see
    // `refresh_reverted_draft_gets_a_fresh_dropped_grace_window` below)
    // so an old draft that was just reorg-reverted still gets a fresh window
    // instead of being evicted on the very next poll.
    conn.execute(
        "UPDATE wallet_tx_drafts SET updated_at = datetime('now','-700 seconds') WHERE id = 'drf2'",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["dropped"], serde_json::json!(1));

    let row = draft_row(&app, "drf2");
    assert_eq!(row.status, "dropped");
    assert!(
        row.error_message.is_some(),
        "dropped draft carries an explanation"
    );
}

#[tokio::test]
async fn refresh_keeps_a_fresh_unfound_draft_pending() {
    // A just-broadcast tx the node hasn't indexed yet must NOT be killed early —
    // it stays `broadcasted` until the grace window elapses.
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(&mut server, 500, TestTxState::Unknown).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcasted_draft(&conn, "drf3"); // created_at = now (age ~0s)
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["dropped"], serde_json::json!(0));
    assert_eq!(draft_row(&app, "drf3").status, "broadcasted");
}

#[tokio::test]
async fn refresh_is_a_soft_noop_when_node_unreachable() {
    // Unreachable node (nothing listening) → never reclassify drafts.
    let conn = seeded_conn("http://127.0.0.1:1", 2_000_000);
    seed_broadcasted_draft(&conn, "drf4");
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["nodeReachable"], serde_json::json!(false));
    assert_eq!(draft_row(&app, "drf4").status, "broadcasted");
}

// --- reorg-aware confirmation tracking (I5) ---------------------------------

/// Seed a `confirmed` draft (with a txid + recorded height) for the active
/// profile.
fn seed_confirmed_draft(conn: &rusqlite::Connection, id: &str, height: i64) {
    db::queries::insert_tx_draft(conn, id, PROFILE, "send_hns", "00", "{}", "{}").unwrap();
    db::queries::update_tx_draft_status(
        conn,
        id,
        "broadcasted",
        None,
        Some(&evidence_transaction().0),
    )
    .unwrap();
    db::queries::update_tx_draft_confirmation(conn, id, height, None).unwrap();
}

#[tokio::test]
async fn refresh_reverts_a_reorged_confirmed_draft_to_broadcasted() {
    // The draft was confirmed at height 490 (well within the 12-confirmation
    // finality depth at tip 495: 495-490+1 = 6 confs). The node has now
    // returned definitive unknown-transaction evidence — a reorg un-mined
    // it. It must revert to `broadcasted` (re-entering mempool tracking) with
    // its height cleared, NOT stay silently `confirmed`.
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(&mut server, 495, TestTxState::Unknown).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_confirmed_draft(&conn, "drf5", 490);
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["nodeReachable"], serde_json::json!(true));
    assert_eq!(res["checked"], serde_json::json!(1));

    let row = draft_row(&app, "drf5");
    assert_eq!(
        row.status, "broadcasted",
        "un-mined confirmed draft must revert to broadcasted"
    );
    assert_eq!(
        row.confirmation_height, None,
        "height must be cleared on revert"
    );
    assert_eq!(
        row.txid.as_deref(),
        Some(evidence_transaction().0.as_str()),
        "txid must survive the revert"
    );
    assert!(
        row.error_message.is_some(),
        "the revert should be explained"
    );
}

#[tokio::test]
async fn refresh_reverted_draft_gets_a_fresh_dropped_grace_window() {
    // Reproduces the Task 8 review finding: a draft that was built/confirmed
    // long ago (old `created_at`), then reorg-reverted back to `broadcasted`,
    // must NOT be instantly judged `dropped` on the very next poll just
    // because its `created_at` is ancient. The revert refreshes `updated_at`
    // (see `revert_tx_draft_to_broadcasted`), and the dropped-grace check
    // must key off THAT, not `created_at` — otherwise a real confirmed send
    // that gets briefly reorged out shows "dropped — coins were not moved"
    // forever (dropped rows are never re-polled), even though the tx is
    // still perfectly capable of re-mining.
    let mut server = mockito::Server::new_async().await;
    // Node never finds the tx across both polls below — first read is
    // interpreted as "reorg un-mined the confirmed draft", second read (now
    // that it's back to `broadcasted`) is the dropped-grace decision point.
    let _mocks = mock_node(&mut server, 495, TestTxState::Unknown).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_confirmed_draft(&conn, "drf12", 490);
    // Backdate ONLY `created_at`, simulating a draft built long ago — its
    // `updated_at` stays fresh (set by `seed_confirmed_draft`'s status/
    // confirmation writes), exactly like a real long-lived confirmed draft.
    conn.execute(
        "UPDATE wallet_tx_drafts SET created_at = datetime('now','-700 seconds') WHERE id = 'drf12'",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    // Poll 1: node no longer knows the tx at all → reorg revert to
    // `broadcasted` (clears height, refreshes `updated_at`).
    let res1 = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res1["reverted"], serde_json::json!(1));
    let row = draft_row(&app, "drf12");
    assert_eq!(row.status, "broadcasted");
    assert_eq!(row.confirmation_height, None);

    // Poll 2: still not found, but the revert just refreshed `updated_at` —
    // must stay `broadcasted` within grace, NOT be marked `dropped`.
    let res2 = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(
        res2["dropped"],
        serde_json::json!(0),
        "a just-reverted draft must get a fresh grace window, not an instant drop"
    );
    let row = draft_row(&app, "drf12");
    assert_eq!(
        row.status, "broadcasted",
        "reorg-reverted draft must survive the next poll instead of being dropped forever"
    );
}

#[tokio::test]
async fn refresh_does_not_repoll_a_deeply_buried_confirmed_draft() {
    // Confirmed at height 100, tip 111 → 111-100+1 = 12 confirmations, exactly
    // at the finality depth. Must NOT be re-polled at all (cheap exit) — the
    // Transaction-evidence mock asserts it is never hit.
    let mut server = mockito::Server::new_async().await;
    let _info = mock_chain_tip(&mut server, 111, 1.0).await;
    let _tx = server
        .mock("POST", "/api/v1/wallet")
        .match_body(mockito::Matcher::Regex("transaction_evidence".into()))
        .expect(0)
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_confirmed_draft(&conn, "drf6", 100);
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["nodeReachable"], serde_json::json!(true));
    assert_eq!(
        res["checked"],
        serde_json::json!(0),
        "deeply-buried draft must not even be listed"
    );

    let row = draft_row(&app, "drf6");
    assert_eq!(
        row.status, "confirmed",
        "deeply-buried draft must stay confirmed, untouched"
    );
    assert_eq!(row.confirmation_height, Some(100));
}

// --- broadcast_pending auto-resolution (folded-in from Task 5 review) ------

/// Seed a `broadcast_pending` draft (transport-ambiguous broadcast, no DB
/// txid) whose `summary_json` carries the locally-computed txid, exactly as
/// `build_send_hns_draft`/`sign_tx_draft` persist it in production.
fn seed_broadcast_pending_draft(conn: &rusqlite::Connection, id: &str) {
    let summary = format!(r#"{{"txid":"{}"}}"#, evidence_transaction().0);
    db::queries::insert_tx_draft(conn, id, PROFILE, "send_hns", "00", "{}", &summary).unwrap();
    db::queries::update_tx_draft_status(conn, id, "broadcast_pending", Some("ambiguous"), None)
        .unwrap();
}

#[tokio::test]
async fn refresh_promotes_broadcast_pending_to_broadcasted_when_node_knows_it() {
    // The node knows the tx (mempool, 0 confirmations) — the earlier
    // transport-ambiguous broadcast actually landed. Must promote to
    // `broadcasted` and persist the locally-computed txid (the DB `txid`
    // column was NULL until now).
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(&mut server, 500, TestTxState::Mempool).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcast_pending_draft(&conn, "drf7");
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["nodeReachable"], serde_json::json!(true));

    let row = draft_row(&app, "drf7");
    assert_eq!(row.status, "broadcasted");
    assert_eq!(row.txid.as_deref(), Some(evidence_transaction().0.as_str()));
}

#[tokio::test]
async fn refresh_promotes_broadcast_pending_straight_to_confirmed_when_already_mined() {
    // The node already mined it while the broadcast outcome was ambiguous —
    // the exact "mined-then-retried mislabel" this closes (Task 5 review).
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(
        &mut server,
        500,
        TestTxState::Confirmed {
            height: 499,
            confirmations: 2,
        },
    )
    .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcast_pending_draft(&conn, "drf8");
    let app = app_with(conn);

    refresh_tx_confirmations(app.state(), None).await.unwrap();

    let row = draft_row(&app, "drf8");
    assert_eq!(row.status, "confirmed");
    assert_eq!(row.confirmation_height, Some(499));
    assert_eq!(row.txid.as_deref(), Some(evidence_transaction().0.as_str()));
}

#[tokio::test]
async fn refresh_fails_broadcast_pending_after_grace_and_releases_reservation() {
    // The node definitively never learned of the tx (Rpc "not found"), and
    // the grace window since the draft's LAST UPDATE (not its original
    // creation) has elapsed — treat like a failed broadcast: `failed` status,
    // reservation released, so the coin isn't held hostage forever.
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(&mut server, 500, TestTxState::Unknown).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcast_pending_draft(&conn, "drf9");
    // Reserve the profile's coin for this draft, as a real broadcast_pending
    // draft would hold (Task 5's reservation rules).
    conn.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = 'drf9' WHERE txid = ?1",
        params![COIN_TXID],
    )
    .unwrap();
    // Backdate past the grace window via updated_at, NOT created_at.
    conn.execute(
        "UPDATE wallet_tx_drafts SET updated_at = datetime('now','-700 seconds') WHERE id = 'drf9'",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    refresh_tx_confirmations(app.state(), None).await.unwrap();

    let row = draft_row(&app, "drf9");
    assert_eq!(row.status, "failed");
    assert!(row.error_message.is_some());
    assert!(
        reserved_txids_for(&app, "drf9").is_empty(),
        "a definitively-failed broadcast_pending draft must release its reservation"
    );
}

#[tokio::test]
async fn refresh_leaves_a_fresh_broadcast_pending_draft_untouched_within_grace() {
    // Same "not found" answer, but the draft was updated moments ago — still
    // within the grace window, so it must NOT be judged failed yet.
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_node(&mut server, 500, TestTxState::Unknown).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcast_pending_draft(&conn, "drf10"); // updated_at = now
    let app = app_with(conn);

    refresh_tx_confirmations(app.state(), None).await.unwrap();

    assert_eq!(draft_row(&app, "drf10").status, "broadcast_pending");
}

#[tokio::test]
async fn refresh_leaves_broadcast_pending_untouched_when_node_unreachable() {
    // A transport-level failure (here: the whole node is unreachable) must
    // never resolve a broadcast_pending draft either way — it stays ambiguous
    // until a definitive answer arrives.
    let conn = seeded_conn("http://127.0.0.1:1", 2_000_000);
    seed_broadcast_pending_draft(&conn, "drf11");
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["nodeReachable"], serde_json::json!(false));
    assert_eq!(draft_row(&app, "drf11").status, "broadcast_pending");
}

// --- guards -----------------------------------------------------------------

#[tokio::test]
async fn build_rejects_non_positive_amount() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();
    for bad in [0i64, -1, -500_000] {
        let err = build_send_hns_draft(app.state(), to.clone(), bad, Some(1), None)
            .await
            .expect_err("non-positive amount must be rejected");
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }
}

#[tokio::test]
async fn build_rejects_watch_only_profile() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    // Flip the active profile to watch-only.
    conn.execute(
        "UPDATE wallet_profiles SET watch_only = 1 WHERE id = ?1",
        params![PROFILE],
    )
    .unwrap();
    let app = app_with(conn);
    let err = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .expect_err("watch-only profile cannot send");
    match err {
        AppError::InvalidInput(m) => assert!(m.contains("watch-only"), "msg: {m}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_rejects_unsigned_draft() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let draft = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .unwrap();
    // Skip signing; broadcasting must refuse.
    let err = broadcast_tx_draft(app.state(), draft.id)
        .await
        .expect_err("unsigned draft must not broadcast");
    match err {
        AppError::InvalidInput(m) => assert!(m.contains("not signed"), "msg: {m}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn sign_rejects_profile_mismatch() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let draft = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .unwrap();
    // Unlock a session bound to a DIFFERENT profile id.
    unlock(&app, "some-other-profile");
    let err = sign_tx_draft_inner(&app.state(), &draft.id)
        .await
        .expect_err("signer for a different profile must not sign");
    match err {
        AppError::InvalidInput(m) => assert!(m.contains("different wallet profile"), "msg: {m}"),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn sign_rejects_when_locked() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let draft = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .unwrap();
    // No unlock at all.
    let err = sign_tx_draft_inner(&app.state(), &draft.id)
        .await
        .expect_err("locked signer must not sign");
    assert!(matches!(err, AppError::WalletLocked), "got {err:?}");
}

#[tokio::test]
async fn remote_sidecar_source_can_broadcast() {
    // A configured REMOTE node must be able to broadcast (the old
    // allow_remote_broadcast gate was removed — configuring the node is the
    // opt-in). Same build→sign→broadcast flow, but chain_source = remote_sidecar.
    let mut server = mockito::Server::new_async().await;
    let node_txid = "fee0000000000000000000000000000000000000000000000000000000000abc";
    let _mocks = mock_broadcast_success(&mut server, 1_000, node_txid).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    db::queries::set_setting(&conn, "chain_source", "remote_sidecar").unwrap();
    let app = app_with(conn);

    let draft = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft.id)
        .await
        .expect("sign");

    let result = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect("remote-node broadcast must succeed");
    assert_eq!(result.status, "broadcasted");
    assert_eq!(result.txid, node_txid);
}

// --- sync_wallet_state against a mock node ---------------------------------

#[tokio::test]
async fn sync_wallet_state_fetches_coins_and_reports_reachable() {
    let mut server = mockito::Server::new_async().await;
    let addr = recv_addr();
    let (_version, program) = crate::noncustodial::address::decode(Network::Main, &addr).unwrap();
    let _tip = mock_chain_tip(&mut server, 150_000, 1.0).await;
    let _coins = server
        .mock("POST", "/api/v1/wallet")
        .match_body(mockito::Matcher::Regex("confirmed_scripts_page".into()))
        .with_header("content-type", "application/json")
        .with_body(wallet_result(serde_json::json!({
            "chain_epoch": TEST_CHAIN_EPOCH,
            "tip": test_tip(150_000),
            "history": [],
            "utxos": [{
                "script_index": 0,
                "coin": {
                    "outpoint": { "txid": COIN_TXID, "index": 7 },
                    "value": 2_000_000,
                    "height": 120,
                    "coinbase": false,
                    "address": { "version": 0, "hash": hex::encode(program) },
                    "covenant": { "kind": 0, "items": [] }
                }
            }],
            "script_examinations": 1,
            "continuation": null
        })))
        .expect_at_least(1)
        .create_async()
        .await;
    let _mempool = mock_mempool_snapshot(&mut server, 150_000).await;
    let _tx = mock_transaction_evidence(&mut server, 150_000, TestTxState::Unknown).await;

    // seeded_conn already inserts a tracked_utxo for COIN_TXID; the sync will
    // upsert the node-reported coin over it.
    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);

    let res = sync_wallet_state(app.state(), None).await.expect("sync ok");
    assert_eq!(res["nodeReachable"], serde_json::json!(true));
    assert_eq!(res["height"], serde_json::json!(150000));
    assert_eq!(res["utxoCount"], serde_json::json!(1));
    // The synced liquid coin (covenant NONE) is reflected in the balance.
    assert_eq!(res["liquidDoos"], serde_json::json!(2000000));
}

// --- get_write_capability: synced + wallet-index gating --------------------

#[tokio::test]
async fn write_capability_blocks_while_node_syncing() {
    let mut server = mockito::Server::new_async().await;
    let _node = mock_chain_tip(&mut server, 100, 0.4).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE); // signer unlocked, so only node-readiness can block

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(!cap.can_write, "syncing node must block writes");
    assert!(
        cap.reason
            .unwrap_or_default()
            .to_lowercase()
            .contains("syncing"),
        "reason should mention syncing",
    );
}

// --- Secure confirmation path (F3) -----------------------------------------

#[tokio::test]
async fn sign_rejects_when_user_cancels_confirmation() {
    let mut server = mockito::Server::new_async().await;
    // The node must NOT be called when the user cancels; this mock asserts 0
    // hits below.
    let m = server.mock("POST", "/").expect(0).create_async().await;

    let conn = seeded_conn(&server.url(), 1_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    // Build a draft.
    let draft_id = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .expect("build")
        .id;

    // Pre-queue a "user cancelled" response for the secure prompt.
    use crate::commands::secure_prompt::{push_test_answer, SecurePromptResult};
    push_test_answer(SecurePromptResult {
        value: None,
        confirmed: false,
    });

    // Call sign_tx_draft (the command, not _inner) — it should reject with
    // UserRejected and NOT sign the draft.
    let err = sign_tx_draft_confirmed(&app.state(), app.handle(), &draft_id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, crate::error::AppError::UserRejected),
        "expected UserRejected, got {err:?}"
    );

    // The draft must still be unsigned.
    let row = draft_row(&app, &draft_id);
    assert!(
        row.signed_tx_hex.is_none(),
        "draft must remain unsigned after cancellation"
    );

    // The node was never called.
    m.assert_async().await;
}

#[tokio::test]
async fn write_capability_blocks_when_wallet_index_is_unavailable() {
    let mut server = mockito::Server::new_async().await;
    let _node = mock_chain_tip(&mut server, 100, 1.0).await;
    let _wallet_index = server
        .mock("POST", "/api/v1/wallet")
        .match_body(mockito::Matcher::Regex("confirmed_scripts_page".into()))
        .with_header("content-type", "application/json")
        .with_body(wallet_error(
            "wallet_index_unavailable",
            "wallet index is not ready",
        ))
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(
        !cap.can_write,
        "an unavailable wallet index must block writes"
    );
    assert!(
        cap.reason
            .unwrap_or_default()
            .to_lowercase()
            .contains("wallet index"),
        "reason should mention the wallet index",
    );
}

#[tokio::test]
async fn write_capability_allows_when_synced_indexed_and_unlocked() {
    let mut server = mockito::Server::new_async().await;
    let _node = mock_chain_tip(&mut server, 100, 1.0).await;
    let _wallet_index = mock_chain_binding(&mut server, 100).await;
    let _mempool = mock_mempool_snapshot(&mut server, 100).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(
        cap.can_write,
        "synced + indexed + unlocked must allow writes"
    );
    assert!(
        cap.reason.is_none(),
        "no blocking reason expected, got {:?}",
        cap.reason
    );
}

#[tokio::test]
async fn sync_wallet_state_reports_unreachable_node_softly() {
    // An unreachable node is NOT an error — reads come from the explorer; we just
    // can't refresh spendable coins. The command returns nodeReachable:false.
    let conn = seeded_conn("http://127.0.0.1:1", 2_000_000);
    let app = app_with(conn);
    let res = sync_wallet_state(app.state(), None)
        .await
        .expect("unreachable node must not error");
    assert_eq!(res["nodeReachable"], serde_json::json!(false));
}

// --- Send Max (sweep) -------------------------------------------------------

#[tokio::test]
async fn build_send_max_sweeps_all_coins_minus_fee() {
    // One coin of 2,000,000 doos; max mode → output = inputTotal − fee, no change.
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft = build_send_hns_draft(app.state(), to, 0, Some(1), Some(true))
        .await
        .expect("max build");
    let row = draft_row(&app, &draft.id);
    let s = summary_of(&row);
    assert_eq!(s.input_total_doos, 2_000_000);
    assert_eq!(s.change_doos, 0, "sweep has no change");
    assert_eq!(s.num_inputs, 1, "spends every coin");
    assert_eq!(
        s.send_total_doos,
        s.input_total_doos - s.fee_doos,
        "recipient gets inputTotal − fee",
    );
}

// --- coin reservation across drafts (I3) ------------------------------------

#[tokio::test]
async fn second_build_cannot_claim_the_only_coin_already_reserved_by_the_first() {
    // A single coin: the first build reserves it entirely for its own draft.
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .expect("first build reserves the only coin");
    assert_eq!(
        reserved_txids_for(&app, &draft1.id),
        vec![COIN_TXID.to_string()]
    );

    // The coin is reserved by draft1 — a second build has nothing left to
    // select from and must fail, NOT silently reuse draft1's coin.
    let err = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect_err("second build must not see draft1's reserved coin");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
}

#[tokio::test]
async fn two_drafts_reserve_disjoint_coins_when_liquidity_allows() {
    // Two coins: 2,000,000 (largest, selected first) + 1,000,000.
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    add_second_coin(&conn, 1_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to.clone(), 900_000, Some(1), None)
        .await
        .expect("draft1 build");
    let draft2 = build_send_hns_draft(app.state(), to, 900_000, Some(1), None)
        .await
        .expect("draft2 build must find the second, still-free coin");

    let r1 = reserved_txids_for(&app, &draft1.id);
    let r2 = reserved_txids_for(&app, &draft2.id);
    assert!(!r1.is_empty() && !r2.is_empty());
    assert!(
        r1.iter().all(|t| !r2.contains(t)),
        "draft1 ({r1:?}) and draft2 ({r2:?}) must not share any reserved coin"
    );
}

#[tokio::test]
async fn deleting_a_draft_frees_its_coin_for_a_later_draft() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .expect("draft1 build");
    // Reserved — a second build fails while draft1 lives.
    build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .expect_err("coin still reserved by draft1");

    delete_tx_draft(app.state(), draft1.id.clone())
        .await
        .expect("delete draft1");

    // The draft row is gone.
    let state = app.state::<AppState>();
    {
        let conn = state.db.lock().unwrap();
        assert!(db::queries::get_tx_draft(&conn, &draft1.id)
            .unwrap()
            .is_none());
    }

    // draft3 can now claim the freed coin.
    let draft3 = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect("draft3 reuses the coin draft1 released");
    assert_eq!(
        reserved_txids_for(&app, &draft3.id),
        vec![COIN_TXID.to_string()]
    );
}

#[tokio::test]
async fn deleting_a_broadcasted_draft_is_refused() {
    let mut server = mockito::Server::new_async().await;
    let node_txid = "abc0000000000000000000000000000000000000000000000000000000000def";
    let _mocks = mock_broadcast_success(&mut server, 1_000, node_txid).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .unwrap();
    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft.id).await.unwrap();
    broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .unwrap();

    let err = delete_tx_draft(app.state(), draft.id.clone())
        .await
        .expect_err("a broadcasted draft must not be deletable");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
}

#[tokio::test]
async fn resigning_a_draft_still_works_using_its_own_reserved_coin() {
    // Two coins so there is something for a SECOND draft to (unsuccessfully)
    // reach for — proves re-sign uses its own reservation, not just "there
    // happened to be only one coin anyway".
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    add_second_coin(&conn, 1_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to, 900_000, Some(1), None)
        .await
        .expect("draft1 build");

    unlock(&app, PROFILE);
    // Re-signing draft1 must succeed even though its input is reserved —
    // by ITSELF.
    sign_tx_draft_inner(&app.state(), &draft1.id)
        .await
        .expect("re-sign of a draft must succeed using its own reserved coin(s)");
    let row = draft_row(&app, &draft1.id);
    assert!(row.signed_tx_hex.is_some());
}

#[tokio::test]
async fn resign_uses_the_reserved_coin_even_if_a_larger_coin_arrived_later() {
    // Build reserves COIN_TXID (2M). A LARGER coin then syncs in. Re-signing
    // the draft must still spend the coin the draft reserved — not silently
    // drift (largest-first) onto the new coin, which was never reserved and
    // could be claimed by another draft before this one broadcasts.
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect("draft1 build");
    assert_eq!(
        reserved_txids_for(&app, &draft1.id),
        vec![COIN_TXID.to_string()]
    );

    // A bigger coin appears after the build (e.g. via sync).
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        add_second_coin(&conn, 5_000_000);
    }

    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft1.id)
        .await
        .expect("sign");

    // Handshake txids are NOT byte-reversed, so a spent prevout appears in the
    // signed hex as the txid hex verbatim.
    let row = draft_row(&app, &draft1.id);
    let hex = row.signed_tx_hex.expect("signed");
    assert!(
        hex.contains(COIN_TXID),
        "signed tx must spend the reserved coin"
    );
    assert!(
        !hex.contains(COIN_TXID_2),
        "signed tx must NOT drift onto the newer, unreserved coin"
    );
}

#[tokio::test]
async fn broadcast_rejection_frees_the_coin_for_a_new_draft() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_broadcast_rejection(&mut server, 1_000).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .unwrap();
    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft1.id).await.unwrap();
    broadcast_tx_draft(app.state(), draft1.id.clone())
        .await
        .expect_err("node rejects the broadcast");

    // The rejected draft's coin must be free again immediately.
    let draft2 = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect("a rejected broadcast's coin must be reclaimed right away");
    assert_eq!(
        reserved_txids_for(&app, &draft2.id),
        vec![COIN_TXID.to_string()]
    );
}

#[tokio::test]
async fn fee_quote_transport_error_keeps_signed_reservation() {
    // If the mandatory pre-broadcast policy quote cannot be obtained, relay is
    // never attempted and the signed draft remains safely retryable.
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .unwrap();
    unlock(&app, PROFILE);
    sign_tx_draft_inner(&app.state(), &draft.id).await.unwrap();

    // Repoint at an unreachable address just before broadcasting.
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        db::queries::set_setting(&conn, "hsrd_rpc_url", "http://127.0.0.1:1").unwrap();
    }

    let err = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect_err("an unreachable node must still surface as an error");
    assert!(
        !matches!(err, AppError::Rpc(_)),
        "a transport failure must NOT be classified as a definitive node rejection, got {err:?}"
    );

    // The quote failed before relay, so the draft stays signed and retryable.
    let row = draft_row(&app, &draft.id);
    assert_eq!(row.status, "signed");
    assert!(row.error_message.is_none());

    // Critically: the coin reservation survives — a new build must NOT be
    // able to claim it while the first draft's fate is unknown.
    assert_eq!(
        reserved_txids_for(&app, &draft.id),
        vec![COIN_TXID.to_string()]
    );
    let err2 = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect_err("a failed policy quote must keep its reservation");
    assert!(matches!(err2, AppError::InvalidInput(_)), "got {err2:?}");
}

#[tokio::test]
async fn ttl_expired_reservation_is_selectable_again() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .expect("draft1 build");

    // Backdate draft1 well past RESERVATION_TTL_SECS (1 hour) without going
    // through delete/broadcast — simulates an abandoned build.
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE wallet_tx_drafts SET created_at = datetime('now','-7200 seconds') WHERE id = ?1",
            params![draft1.id],
        )
        .unwrap();
    }

    let draft2 = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect("stale reservation must be reclaimed after TTL");
    assert_eq!(
        reserved_txids_for(&app, &draft2.id),
        vec![COIN_TXID.to_string()]
    );
    // draft1's stale claim was cleared, not merely bypassed.
    assert!(reserved_txids_for(&app, &draft1.id).is_empty());
}

#[tokio::test]
async fn explicit_release_frees_the_coin_without_deleting_the_draft() {
    let conn = seeded_conn("http://127.0.0.1:12037", 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft1 = build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
        .await
        .expect("draft1 build");

    let result = release_tx_draft_reservation(app.state(), draft1.id.clone())
        .await
        .expect("explicit release");
    assert_eq!(result["coinsReleased"], serde_json::json!(1));

    // The draft row itself is untouched.
    assert!(
        db::queries::get_tx_draft(&app.state::<AppState>().db.lock().unwrap(), &draft1.id)
            .unwrap()
            .is_some(),
        "release must not delete the draft row"
    );

    // A new draft can now claim the freed coin.
    let draft2 = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .expect("draft2 reuses the explicitly-released coin");
    assert_eq!(
        reserved_txids_for(&app, &draft2.id),
        vec![COIN_TXID.to_string()]
    );
}

// --- get_write_capability: authenticated sync status -----------------------

#[tokio::test]
async fn write_capability_blocks_at_tip_with_low_progress() {
    // blocks == headers but verification_progress is only 0.9997 — the node
    // reports "tip reached" while still far behind the real chain. The progress
    // gate must block writes in this case.
    let mut server = mockito::Server::new_async().await;
    let _node = mock_chain_tip(&mut server, 317, 0.9997).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(
        !cap.can_write,
        "low progress must block writes even at apparent tip"
    );
    assert!(
        cap.reason.as_deref().unwrap_or("").contains("syncing"),
        "reason should mention syncing; got {:?}",
        cap.reason,
    );
}

#[tokio::test]
async fn write_capability_blocks_when_behind_tip() {
    // blocks < headers → genuinely mid-sync → blocked, even with high progress.
    let mut server = mockito::Server::new_async().await;
    let _node = mock_chain_tip(&mut server, 100, 0.5).await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(!cap.can_write, "behind tip must block writes");
    assert!(
        cap.reason
            .unwrap_or_default()
            .to_lowercase()
            .contains("syncing"),
        "reason should mention syncing",
    );
}
