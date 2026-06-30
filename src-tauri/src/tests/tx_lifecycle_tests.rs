//! Full transaction lifecycle integration tests against a mock hsd node.
//!
//! These drive the REAL `#[tauri::command]` functions (`build_send_hns_draft`,
//! `sign_tx_draft`, `broadcast_tx_draft`) with a managed `AppState` over a fully
//! -migrated in-memory DB and a `mockito` JSON-RPC node. They prove the
//! end-to-end orchestration — coin selection → draft persistence → signing →
//! broadcast/status — behaves correctly, and that every guard that protects
//! funds (watch-only, non-positive amount, profile mismatch, broadcasting an
//! unsigned draft, recording a failed broadcast as failed) actually fires.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::tx::{
    broadcast_tx_draft, build_send_hns_draft, get_write_capability, refresh_tx_confirmations,
    sign_tx_draft, sync_wallet_state,
};
use crate::db;
use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::types::TxSummary;
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
    let (sk, pk, addr) = crate::noncustodial::hd::derive_address(Network::Main, s, 0, 0, 0).unwrap();
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
    db::queries::set_setting(&conn, "node_rpc_url", node_url).unwrap();

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

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn unlock(app: &tauri::App<tauri::test::MockRuntime>, profile_id: &str) {
    let state = app.state::<AppState>();
    *state.signer.lock().unwrap() =
        Some(SignerSession::unlock(profile_id.to_string(), Network::Main, master(), 600_000));
}

fn recv_addr() -> String {
    leaf00().0
}

/// Fetch the persisted draft row for assertions about signed hex / status.
fn draft_row(
    app: &tauri::App<tauri::test::MockRuntime>,
    id: &str,
) -> db::queries::TxDraftRow {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    db::queries::get_tx_draft(&c, id).unwrap().unwrap()
}

fn summary_of(row: &db::queries::TxDraftRow) -> TxSummary {
    serde_json::from_str(&row.summary_json).expect("summary parses")
}

// --- happy path: build -> sign -> broadcast --------------------------------

#[tokio::test]
async fn full_lifecycle_build_sign_broadcast_succeeds() {
    let mut server = mockito::Server::new_async().await;
    // hsd returns the txid string as the JSON-RPC result.
    let node_txid = "abc0000000000000000000000000000000000000000000000000000000000def";
    let m = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"result":"{node_txid}","error":null,"id":1}}"#))
        .create_async()
        .await;

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
    sign_tx_draft(app.state(), draft_id.clone()).await.expect("sign");
    {
        let row = draft_row(&app, &draft_id);
        assert!(row.signed_tx_hex.is_some(), "draft must be signed");
        assert!(summary_of(&row).txid.is_some(), "signed summary carries a txid");
    }

    // 3. Broadcast — sends to the mock node; status + node txid recorded.
    let result = broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .expect("broadcast");
    assert_eq!(result.status, "broadcasted");
    assert_eq!(result.txid, node_txid);
    m.assert_async().await;

    // The draft row reflects the broadcast.
    let row = draft_row(&app, &draft_id);
    assert_eq!(row.status, "broadcasted");
    assert_eq!(row.txid.as_deref(), Some(node_txid));
}

// --- failure path: node rejects -> recorded as failed, NOT sent ------------

#[tokio::test]
async fn broadcast_failure_marks_draft_failed_and_errors() {
    let mut server = mockito::Server::new_async().await;
    // hsd-style RPC error envelope (e.g. "missing inputs" / "bad-txns").
    let _m = server
        .mock("POST", "/")
        .with_body(r#"{"result":null,"error":{"message":"TX rejected: bad-txns-inputs-missingorspent","code":-26},"id":1}"#)
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    let to = recv_addr();

    let draft = build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
        .await
        .unwrap();
    unlock(&app, PROFILE);
    sign_tx_draft(app.state(), draft.id.clone()).await.unwrap();

    let err = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect_err("broadcast must surface the node rejection");
    assert!(matches!(err, AppError::Rpc(_)), "got {err:?}");

    // Critically: the draft is marked failed, never "broadcasted".
    let stored = draft_row(&app, &draft.id);
    assert_eq!(stored.status, "failed");
    assert!(stored.signed_tx_hex.is_some(), "signed hex retained for inspection");
}

// --- confirmation tracking (broadcasted -> confirmed / dropped) ------------

const DRAFT_TXID: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef0";

/// Seed a `broadcasted` draft (with a txid) for the active profile.
fn seed_broadcasted_draft(conn: &rusqlite::Connection, id: &str) {
    db::queries::insert_tx_draft(conn, id, PROFILE, "send_hns", "00", "{}", "{}").unwrap();
    db::queries::update_tx_draft_status(conn, id, "broadcasted", None, Some(DRAFT_TXID)).unwrap();
}

/// getblockchaininfo + getrawtransaction mocks (matched by method in the body).
async fn mock_node(
    server: &mut mockito::Server,
    tip: i64,
    getrawtx_body: &str,
) -> (mockito::Mock, mockito::Mock) {
    let info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(format!(
            r#"{{"result":{{"blocks":{tip},"headers":{tip}}},"error":null,"id":1}}"#
        ))
        .expect_at_least(1)
        .create_async()
        .await;
    let tx = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getrawtransaction".into()))
        .with_body(getrawtx_body.to_string())
        .create_async()
        .await;
    (info, tx)
}

#[tokio::test]
async fn refresh_marks_a_mined_draft_confirmed() {
    let mut server = mockito::Server::new_async().await;
    let (_info, _tx) = mock_node(
        &mut server,
        437,
        r#"{"result":{"confirmations":3,"height":435},"error":null,"id":1}"#,
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
    let (_info, _tx) = mock_node(
        &mut server,
        500,
        r#"{"result":null,"error":{"message":"TX not found.","code":-5},"id":1}"#,
    )
    .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    seed_broadcasted_draft(&conn, "drf2");
    // Backdate past the grace window so it's judged dropped, not still-pending.
    conn.execute(
        "UPDATE wallet_tx_drafts SET created_at = datetime('now','-600 seconds') WHERE id = 'drf2'",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    let res = refresh_tx_confirmations(app.state(), None).await.unwrap();
    assert_eq!(res["dropped"], serde_json::json!(1));

    let row = draft_row(&app, "drf2");
    assert_eq!(row.status, "dropped");
    assert!(row.error_message.is_some(), "dropped draft carries an explanation");
}

#[tokio::test]
async fn refresh_keeps_a_fresh_unfound_draft_pending() {
    // A just-broadcast tx the node hasn't indexed yet must NOT be killed early —
    // it stays `broadcasted` until the grace window elapses.
    let mut server = mockito::Server::new_async().await;
    let (_info, _tx) = mock_node(
        &mut server,
        500,
        r#"{"result":null,"error":{"message":"TX not found.","code":-5},"id":1}"#,
    )
    .await;

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
    let err = sign_tx_draft(app.state(), draft.id)
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
    let err = sign_tx_draft(app.state(), draft.id)
        .await
        .expect_err("locked signer must not sign");
    assert!(matches!(err, AppError::WalletLocked), "got {err:?}");
}

#[tokio::test]
async fn remote_node_source_can_broadcast() {
    // A configured REMOTE node must be able to broadcast (the old
    // allow_remote_broadcast gate was removed — configuring the node is the
    // opt-in). Same build→sign→broadcast flow, but chain_source = remote_node.
    let mut server = mockito::Server::new_async().await;
    let node_txid = "fee0000000000000000000000000000000000000000000000000000000000abc";
    let _m = server
        .mock("POST", "/")
        .with_body(format!(r#"{{"result":"{node_txid}","error":null,"id":1}}"#))
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    db::queries::set_setting(&conn, "chain_source", "remote_node").unwrap();
    let app = app_with(conn);

    let draft = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app, PROFILE);
    sign_tx_draft(app.state(), draft.id.clone()).await.expect("sign");

    let result = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect("remote-node broadcast must succeed");
    assert_eq!(result.status, "broadcasted");
    assert_eq!(result.txid, node_txid);
}

#[tokio::test]
async fn explorer_source_refuses_broadcast_before_any_rpc() {
    // In read-only (explorer) mode broadcasting must be refused at the command
    // level BEFORE touching the node. The mock server would return success if it
    // were ever called, so erroring out proves the guard fired (no funds moved).
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_body(r#"{"result":"deadbeef","error":null,"id":1}"#)
        .expect(0) // must never be hit
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    db::queries::set_setting(&conn, "chain_source", "explorer").unwrap();
    let app = app_with(conn);

    let draft = build_send_hns_draft(app.state(), recv_addr(), 500_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app, PROFILE);
    sign_tx_draft(app.state(), draft.id.clone()).await.expect("sign");

    let err = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect_err("explorer mode must refuse to broadcast");
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("read-only"), "msg: {msg}"),
        other => panic!("expected read-only InvalidInput, got {other:?}"),
    }
    // The draft must NOT be recorded as broadcasted, and the node was never hit.
    assert_ne!(draft_row(&app, &draft.id).status, "broadcasted");
    m.assert_async().await;
}

// --- sync_wallet_state against a mock node ---------------------------------

#[tokio::test]
async fn sync_wallet_state_fetches_coins_and_reports_reachable() {
    let mut server = mockito::Server::new_async().await;
    let addr = recv_addr();
    let coin = format!(
        r#"{{"hash":"{COIN_TXID}","index":7,"value":2000000,"address":"{addr}","height":120,"covenant":{{"type":0,"action":"NONE","items":[]}}}}"#
    );
    // Specific matchers first; the catch-all (best-effort getrawtransaction /
    // getnameinfo) last so it only handles what the specific ones don't.
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"chain":"main","blocks":150000},"error":null,"id":1}"#)
        .create_async()
        .await;
    // Address coins come from the node REST route, NOT JSON-RPC — return the raw
    // coin array (no envelope).
    let _coins = server
        .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
        .with_body(format!(r#"[{coin}]"#))
        .create_async()
        .await;
    let _other = server
        .mock("POST", "/")
        .with_body(r#"{"result":null,"error":{"message":"not found"},"id":1}"#)
        .create_async()
        .await;

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

// --- get_write_capability: synced + address-indexed gating -----------------

/// hsd `getblockchaininfo` body with the given (lowercase) verificationprogress.
fn bi_body(progress: f64) -> String {
    format!(r#"{{"result":{{"chain":"main","blocks":100,"verificationprogress":{progress}}},"error":null,"id":1}}"#)
}

#[tokio::test]
async fn write_capability_blocks_while_node_syncing() {
    let mut server = mockito::Server::new_async().await;
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(bi_body(0.4))
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE); // signer unlocked, so only node-readiness can block

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(!cap.can_write, "syncing node must block writes");
    assert!(
        cap.reason.unwrap_or_default().to_lowercase().contains("syncing"),
        "reason should mention syncing",
    );
}

#[tokio::test]
async fn write_capability_blocks_when_not_address_indexed() {
    let mut server = mockito::Server::new_async().await;
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(bi_body(0.9999))
        .create_async()
        .await;
    // Synced, but the node's REST coin route rejects (address index disabled).
    let _coins = server
        .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
        .with_status(400)
        .with_body(r#"{"error":{"message":"Address indexing not available."}}"#)
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(!cap.can_write, "un-indexed node must block writes");
    assert!(
        cap.reason.unwrap_or_default().to_lowercase().contains("address-index"),
        "reason should mention address indexing",
    );
}

#[tokio::test]
async fn write_capability_allows_when_synced_indexed_and_unlocked() {
    let mut server = mockito::Server::new_async().await;
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(bi_body(0.9999))
        .create_async()
        .await;
    let _coins = server
        .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
        .with_body("[]")
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(cap.can_write, "synced + indexed + unlocked must allow writes");
    assert!(cap.reason.is_none(), "no blocking reason expected, got {:?}", cap.reason);
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

// --- get_write_capability: synced = chain tip reached (blocks >= headers) ----

/// getblockchaininfo body with explicit blocks/headers + (low) progress.
fn bi_full(blocks: i64, headers: i64, progress: f64) -> String {
    format!(
        r#"{{"result":{{"chain":"main","blocks":{blocks},"headers":{headers},"verificationprogress":{progress}}},"error":null,"id":1}}"#
    )
}

#[tokio::test]
async fn write_capability_allows_at_tip_despite_low_progress() {
    // Regtest-style: blocks == headers (tip reached) but progress only 0.9997.
    // The OLD 0.9999 gate wrongly blocked this; tip-reached must allow writes.
    let mut server = mockito::Server::new_async().await;
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(bi_full(317, 317, 0.9997))
        .create_async()
        .await;
    let _coins = server
        .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
        .with_body("[]")
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(cap.can_write, "node at tip must allow writes; reason={:?}", cap.reason);
}

#[tokio::test]
async fn write_capability_blocks_when_behind_tip() {
    // blocks < headers → genuinely mid-sync → blocked, even with high progress.
    let mut server = mockito::Server::new_async().await;
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(bi_full(100, 200, 0.9999))
        .create_async()
        .await;

    let conn = seeded_conn(&server.url(), 2_000_000);
    let app = app_with(conn);
    unlock(&app, PROFILE);

    let cap = get_write_capability(app.state()).await.expect("cap");
    assert!(!cap.can_write, "behind tip must block writes");
    assert!(
        cap.reason.unwrap_or_default().to_lowercase().contains("syncing"),
        "reason should mention syncing",
    );
}
