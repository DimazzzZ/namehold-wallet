//! Tests for `commands::history` — covers the async command
//! `read_action_history`, the client-injected fetch/dedup helper
//! `fetch_and_dedup_txs_with_client`, and the DB helper
//! `load_wallet_addresses`.
//!
//! `classify_tx` (the pure classifier) is already fully covered by the inline
//! `#[cfg(test)]` module in `src/commands/history.rs`; we do not duplicate it
//! here.
//!
//! Two harnesses:
//! - `MockNodeRpc` for isolated unit tests of `fetch_and_dedup_txs_with_client`
//!   (fast, no HTTP/async-runtime wiring beyond `#[tokio::test]`).
//! - `mockito` + the `app_with` mock-Tauri harness for `read_action_history`,
//!   which reads `node_rpc_url` from DB settings and hits the REST route
//!   `GET /tx/address/:addr`.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::history::{
    fetch_and_dedup_txs_with_client, load_wallet_addresses, read_action_history,
};
use crate::db;
use crate::tests::mock_node_rpc::MockNodeRpc;
use crate::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn empty_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn add_profile(conn: &rusqlite::Connection, id: &str, network: &str) {
    db::queries::insert_wallet_profile(
        conn,
        id,
        id,
        "mnemonic_hot",
        network,
        "xpubDUMMY",
        0,
        false,
    )
    .unwrap();
}

fn add_derived_address(conn: &rusqlite::Connection, profile: &str, child_index: i64, addr: &str) {
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, ?2, ?3, '00', '00')",
        params![profile, child_index, addr],
    )
    .unwrap();
}

/// A plain receive tx: external input, one output landing on `to`.
fn receive_tx(hash: &str, height: i64, to: &str, value: i64) -> serde_json::Value {
    serde_json::json!({
        "hash": hash,
        "height": height,
        "time": 1_700_000_000i64,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": value + 1000, "address": "hs1qexternal",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": value, "address": to,
             "covenant": {"type": 0, "action": "NONE", "items": []}}
        ]
    })
}

// ===========================================================================
// Priority 1: fetch_and_dedup_txs_with_client (MockNodeRpc)
// ===========================================================================

#[tokio::test]
async fn dedup_happy_path_single_address_single_tx() {
    let mock = MockNodeRpc::new().with_txs_by_address(vec![receive_tx("aa", 100, "addr1", 500)]);
    let map = fetch_and_dedup_txs_with_client(&mock, &["addr1".to_string()]).await;
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("aa"));
}

#[tokio::test]
async fn dedup_multiple_txs_from_one_address() {
    let mock = MockNodeRpc::new().with_txs_by_address(vec![
        receive_tx("aa", 100, "addr1", 500),
        receive_tx("bb", 101, "addr1", 600),
        receive_tx("cc", 102, "addr1", 700),
    ]);
    let map = fetch_and_dedup_txs_with_client(&mock, &["addr1".to_string()]).await;
    assert_eq!(map.len(), 3);
    assert!(map.contains_key("aa") && map.contains_key("bb") && map.contains_key("cc"));
}

#[tokio::test]
async fn dedup_same_txid_from_multiple_addresses() {
    // MockNodeRpc returns the same tx list for any address, so calling with
    // two addresses would insert each hash twice — dedup keeps one entry each.
    let mock = MockNodeRpc::new().with_txs_by_address(vec![
        receive_tx("aa", 100, "addr1", 500),
        receive_tx("bb", 101, "addr2", 600),
    ]);
    let map =
        fetch_and_dedup_txs_with_client(&mock, &["addr1".to_string(), "addr2".to_string()]).await;
    assert_eq!(map.len(), 2, "duplicate txids across addresses collapse");
}

#[tokio::test]
async fn dedup_all_addresses_error_yields_empty_map() {
    let mock = MockNodeRpc::new().with_txs_by_address_err("index disabled");
    let map = fetch_and_dedup_txs_with_client(
        &mock,
        &["a".to_string(), "b".to_string(), "c".to_string()],
    )
    .await;
    assert!(map.is_empty(), "per-address errors are swallowed");
}

#[tokio::test]
async fn dedup_empty_address_list() {
    let mock = MockNodeRpc::new().with_txs_by_address(vec![receive_tx("aa", 100, "addr1", 500)]);
    let map = fetch_and_dedup_txs_with_client(&mock, &[]).await;
    assert!(map.is_empty());
}

#[tokio::test]
async fn dedup_empty_tx_list_from_address() {
    let mock = MockNodeRpc::new().with_txs_by_address(vec![]);
    let map = fetch_and_dedup_txs_with_client(&mock, &["addr1".to_string()]).await;
    assert!(map.is_empty());
}

#[tokio::test]
async fn dedup_tx_without_hash_is_skipped() {
    // A tx object missing the "hash" field must not be inserted.
    let no_hash = serde_json::json!({"height": 100, "outputs": []});
    let mock =
        MockNodeRpc::new().with_txs_by_address(vec![no_hash, receive_tx("aa", 100, "addr1", 500)]);
    let map = fetch_and_dedup_txs_with_client(&mock, &["addr1".to_string()]).await;
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("aa"));
}

// ===========================================================================
// Priority 2: load_wallet_addresses (in-memory DB)
// ===========================================================================

#[test]
fn load_addresses_none_returns_empty() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    let out = load_wallet_addresses(&conn, "P1").unwrap();
    assert!(out.is_empty());
}

#[test]
fn load_addresses_multiple_sorted() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    add_derived_address(&conn, "P1", 0, "hs1qccc");
    add_derived_address(&conn, "P1", 1, "hs1qaaa");
    add_derived_address(&conn, "P1", 2, "hs1qbbb");
    let out = load_wallet_addresses(&conn, "P1").unwrap();
    // Query is `ORDER BY address` — lexicographic, independent of insert order.
    assert_eq!(out, vec!["hs1qaaa", "hs1qbbb", "hs1qccc"]);
}

#[test]
fn load_addresses_error_on_missing_table() {
    // Prepare fails when the schema lacks the table → error path exercised.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let err = load_wallet_addresses(&conn, "P1");
    assert!(err.is_err(), "prepare against missing table must error");
}

// ===========================================================================
// Priority 3: read_action_history (mockito + app harness)
// ===========================================================================

#[tokio::test]
async fn history_no_active_profile_returns_empty() {
    let conn = empty_db();
    // No active profile set, no explicit id passed.
    let app = app_with(conn);
    let rows = read_action_history(app.state(), None).await.unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn history_no_addresses_returns_empty() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert!(rows.is_empty(), "profile with no derived addresses → empty");
}

#[tokio::test]
async fn history_happy_path_receive() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(
            serde_json::to_string(&vec![receive_tx("aa", 100, "hs1qmine", 100_000_000)]).unwrap(),
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "receive");
    assert_eq!(rows[0].direction, "receive");
    assert!(rows[0].value_doos > 0);
}

#[tokio::test]
async fn history_happy_path_send() {
    let send_tx = serde_json::json!({
        "hash": "bb",
        "height": 101,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 400_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 250_000_000i64, "address": "hs1qdest",
             "covenant": {"type": 0, "action": "NONE", "items": []}},
            {"value": 149_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 0, "action": "NONE", "items": []}}
        ]
    });
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(serde_json::to_string(&vec![send_tx]).unwrap())
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "send");
    assert_eq!(rows[0].direction, "send");
    assert!(rows[0].value_doos < 0);
    assert_eq!(rows[0].counterparty.as_deref(), Some("hs1qdest"));
}

#[tokio::test]
async fn history_happy_path_bid_decodes_name() {
    // items = [nameHash, u32(start), rawName hex, blind]; "foo" -> 666f6f.
    let bid_tx = serde_json::json!({
        "hash": "cc",
        "height": 200,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 1_000_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 3, "action": "BID",
                          "items": ["deadbeef", "000000c8", "666f6f", "cafebabe"]}}
        ]
    });
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(serde_json::to_string(&vec![bid_tx]).unwrap())
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "bid");
    assert_eq!(rows[0].name.as_deref(), Some("foo"));
    assert_eq!(rows[0].name_hash.as_deref(), Some("deadbeef"));
}

#[tokio::test]
async fn history_sort_order_unconfirmed_then_height_desc() {
    let unconfirmed = serde_json::json!({
        "hash": "u1", "height": -1, "block": null, "confirmations": 0,
        "inputs": [{"coin": {"value": 10i64, "address": "hs1qext",
                             "covenant": {"type": 0, "items": []}}}],
        "outputs": [{"value": 5i64, "address": "hs1qmine",
                     "covenant": {"type": 0, "items": []}}]
    });
    let txs = vec![
        receive_tx("h100", 100, "hs1qmine", 100),
        unconfirmed,
        receive_tx("h200", 200, "hs1qmine", 200),
    ];
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(serde_json::to_string(&txs).unwrap())
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].txid, "u1", "unconfirmed leads");
    assert!(!rows[0].confirmed);
    assert_eq!(rows[1].txid, "h200", "then highest height");
    assert_eq!(rows[2].txid, "h100");
}

#[tokio::test]
async fn history_dedup_same_tx_from_multiple_addresses() {
    // Both addresses return the same tx list; the row must appear once.
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(
            serde_json::to_string(&vec![receive_tx("aa", 100, "hs1qmine1", 100_000_000)]).unwrap(),
        )
        .expect_at_least(2)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine1");
    add_derived_address(&conn, "W1", 1, "hs1qmine2");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "same txid across addresses collapses to one row"
    );
}

#[tokio::test]
async fn history_node_error_is_swallowed_returns_empty() {
    // Per-address fetch errors are swallowed by
    // `fetch_and_dedup_txs_with_client`, so a 500 yields an empty result set
    // rather than an `Err`.
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_status(500)
        .with_body(r#"{"error":{"message":"boom"}}"#)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn history_transfer_captures_counterparty() {
    let transfer_tx = serde_json::json!({
        "hash": "ee",
        "height": 300,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 5_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 6, "items": ["deadbeef"]}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qbuyer",
             "covenant": {"type": 9, "action": "TRANSFER",
                          "items": ["deadbeef", "00", "aabbccddeeff"]}}
        ]
    });
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(serde_json::to_string(&vec![transfer_tx]).unwrap())
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "transfer");
    assert_eq!(rows[0].counterparty.as_deref(), Some("hs1qbuyer"));
}

#[tokio::test]
async fn history_unrelated_txs_are_filtered_out() {
    // Tx touches neither the wallet's inputs nor outputs → classify_tx → None.
    let unrelated = serde_json::json!({
        "hash": "zz",
        "height": 1,
        "inputs": [{"coin": {"value": 1i64, "address": "hs1qstranger",
                             "covenant": {"type": 0, "items": []}}}],
        "outputs": [{"value": 1i64, "address": "hs1qelse",
                     "covenant": {"type": 0, "items": []}}]
    });
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(serde_json::to_string(&vec![unrelated]).unwrap())
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    assert!(rows.is_empty(), "txs not touching wallet are dropped");
}

#[tokio::test]
async fn history_resolves_active_profile_when_id_omitted() {
    // Exercises the `resolve_profile(None)` → active-profile branch of the
    // command, complementing the explicit-id cases above.
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(
            serde_json::to_string(&vec![receive_tx("aa", 100, "hs1qmine", 100_000_000)]).unwrap(),
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let rows = read_action_history(app.state(), None).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].txid, "aa");
}

// ===========================================================================
// Priority 4: extended classify_tx branches (covenant labels, sort ties)
//
// These use `read_action_history` to reach `classify_tx` through the real
// command surface — that way one call exercises the classifier branches AND
// the surrounding wiring at the same time. The inline classify_tx tests in
// `src/commands/history.rs` cover: receive, send, bid, reveal, transfer,
// unconfirmed, unrelated, register-from-own-reveal, plus one more; the labels
// below (OPEN/REDEEM/UPDATE/RENEW/FINALIZE/REVOKE/CLAIM/other + internal-only
// spend + BID with empty raw name + covenant with empty address) fill the
// remaining match arms.
// ===========================================================================

fn covenant_tx(hash: &str, height: i64, cov_type: u64, cov_addr: &str) -> serde_json::Value {
    serde_json::json!({
        "hash": hash,
        "height": height,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 5_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": cov_addr,
             "covenant": {"type": cov_type, "items": ["deadbeef"]}}
        ]
    })
}

async fn history_with_txs(txs: Vec<serde_json::Value>) -> Vec<crate::commands::history::ActionRow> {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", mockito::Matcher::Regex(r"^/tx/address/".into()))
        .with_body(serde_json::to_string(&txs).unwrap())
        .create_async()
        .await;
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_derived_address(&conn, "W1", 0, "hs1qmine");
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    let app = app_with(conn);
    // Hold `server` until after the call so the mock stays live.
    let rows = read_action_history(app.state(), Some("W1".into()))
        .await
        .unwrap();
    drop(server);
    rows
}

#[tokio::test]
async fn classify_covenant_labels_cover_all_arms() {
    // COV_OPEN=2, BID=3, REVEAL=4, REDEEM=5, REGISTER=6, UPDATE=7,
    // RENEW=8, TRANSFER=9, FINALIZE=10, REVOKE=11, CLAIM=1.
    // Test each label via a wallet-initiated covenant output landing on the
    // wallet's own address (so it's classified as name-covenant action).
    let txs = vec![
        covenant_tx("op", 100, 2, "hs1qmine"),  // open
        covenant_tx("rd", 101, 5, "hs1qmine"),  // redeem
        covenant_tx("up", 102, 7, "hs1qmine"),  // update
        covenant_tx("rn", 103, 8, "hs1qmine"),  // renew
        covenant_tx("fn", 104, 10, "hs1qmine"), // finalize
        covenant_tx("rv", 105, 11, "hs1qmine"), // revoke
        covenant_tx("cl", 106, 1, "hs1qmine"),  // claim
        covenant_tx("ot", 107, 99, "hs1qmine"), // unknown → "other"
    ];
    let rows = history_with_txs(txs).await;
    let by_txid: std::collections::HashMap<_, _> =
        rows.iter().map(|r| (r.txid.clone(), r.clone())).collect();
    assert_eq!(by_txid["op"].action, "open");
    assert_eq!(by_txid["rd"].action, "redeem");
    assert_eq!(by_txid["up"].action, "update");
    assert_eq!(by_txid["rn"].action, "renew");
    assert_eq!(by_txid["fn"].action, "finalize");
    assert_eq!(by_txid["rv"].action, "revoke");
    assert_eq!(by_txid["cl"].action, "claim");
    assert_eq!(by_txid["ot"].action, "other");
}

#[tokio::test]
async fn classify_send_internal_when_no_external_output() {
    // spends_ours = true (input from our address), but 100% of outputs land
    // back on our own addresses → sent_to_external == 0 → dir "internal".
    let tx = serde_json::json!({
        "hash": "int",
        "height": 200,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 100_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 99_500_000i64, "address": "hs1qmine",
             "covenant": {"type": 0, "items": []}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "send");
    assert_eq!(rows[0].direction, "internal");
    assert_eq!(rows[0].value_doos, 0);
    assert!(rows[0].counterparty.is_none());
}

#[tokio::test]
async fn classify_bid_with_missing_raw_name_leaves_name_none() {
    // BID covenant carrying only the nameHash (items has just 1 entry) — no
    // items[2] → `name` stays None. Exercises the `items.get(2)` None branch.
    let tx = serde_json::json!({
        "hash": "bidshort",
        "height": 300,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 1_000_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 3, "action": "BID", "items": ["deadbeef"]}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "bid");
    assert!(rows[0].name.is_none());
    assert_eq!(rows[0].name_hash.as_deref(), Some("deadbeef"));
}

#[tokio::test]
async fn classify_bid_with_undecodable_raw_name_leaves_name_none() {
    // items[2] present but not valid hex → hex::decode Err path.
    let tx = serde_json::json!({
        "hash": "bidbadhex",
        "height": 301,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 1_000_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 3, "action": "BID",
                          "items": ["deadbeef", "000000c8", "zzznothex", "cafebabe"]}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].name.is_none());
}

#[tokio::test]
async fn classify_covenant_with_empty_address_yields_none_counterparty() {
    // Covenant output with empty `address` string — exercises the
    // `name_cov_addr = if addr.is_empty() { None }` branch.
    let tx = serde_json::json!({
        "hash": "covemptyaddr",
        "height": 400,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 5_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "",
             "covenant": {"type": 7, "action": "UPDATE", "items": ["deadbeef"]}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "update");
    assert!(rows[0].counterparty.is_none());
}

#[tokio::test]
async fn classify_covenant_empty_name_hash_leaves_none() {
    // items[0] present but an empty string — exercises the `!h.is_empty()`
    // guard so `name_hash` stays None.
    let tx = serde_json::json!({
        "hash": "cov_empty_hash",
        "height": 410,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 5_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 7, "action": "UPDATE", "items": [""]}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "update");
    assert!(rows[0].name_hash.is_none());
}

#[tokio::test]
async fn classify_bid_with_empty_raw_name_leaves_none() {
    // items[2] is empty string → hex::decode("") = Ok([]) → String::from_utf8
    // succeeds with "" → guarded by `!s.is_empty()` so name stays None.
    let tx = serde_json::json!({
        "hash": "bidemptyraw",
        "height": 411,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 1_000_000_000i64, "address": "hs1qmine",
                      "covenant": {"type": 0, "items": []}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 3, "action": "BID",
                          "items": ["deadbeef", "000000c8", "", "cafebabe"]}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].name.is_none());
}

#[tokio::test]
async fn history_sort_ties_by_txid_two_unconfirmed() {
    // Two unconfirmed txs → both height None → tie-break by txid ascending
    // ("aaa" < "bbb"). Covers the `(None, None) => a.txid.cmp(&b.txid)` arm.
    let mk_unconf = |hash: &str| {
        serde_json::json!({
            "hash": hash, "height": -1, "block": null,
            "inputs": [{"coin": {"value": 10i64, "address": "hs1qext",
                                 "covenant": {"type": 0, "items": []}}}],
            "outputs": [{"value": 5i64, "address": "hs1qmine",
                         "covenant": {"type": 0, "items": []}}]
        })
    };
    let txs = vec![mk_unconf("bbb"), mk_unconf("aaa")];
    let rows = history_with_txs(txs).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].txid, "aaa");
    assert_eq!(rows[1].txid, "bbb");
}

#[tokio::test]
async fn history_sort_ties_by_txid_same_height() {
    // Two confirmed txs at the SAME height → tie-break by txid asc.
    // Covers the `then_with(|| a.txid.cmp(&b.txid))` continuation.
    let txs = vec![
        receive_tx("zzz", 100, "hs1qmine", 100),
        receive_tx("aaa", 100, "hs1qmine", 100),
    ];
    let rows = history_with_txs(txs).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].txid, "aaa");
    assert_eq!(rows[1].txid, "zzz");
}

#[tokio::test]
async fn classify_covenant_received_direction() {
    // Covenant output lands on OUR address but the inputs are all external
    // (we did NOT spend our own coins) → spends_ours = false,
    // name_cov_addr_is_ours = true → direction "receive". Models a name
    // FINALIZE arriving at our wallet from a transfer initiated elsewhere.
    let tx = serde_json::json!({
        "hash": "recvcov",
        "height": 500,
        "inputs": [
            {"prevout": {"hash": "pp", "index": 0},
             "coin": {"value": 5_000_000i64, "address": "hs1qexternal",
                      "covenant": {"type": 9, "items": ["deadbeef"]}}}
        ],
        "outputs": [
            {"value": 5_000_000i64, "address": "hs1qmine",
             "covenant": {"type": 10, "action": "FINALIZE", "items": ["deadbeef"]}}
        ]
    });
    let rows = history_with_txs(vec![tx]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "finalize");
    assert_eq!(rows[0].direction, "receive");
}
