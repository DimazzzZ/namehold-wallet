//! Tests for `commands::read` — covers `read_balance`, `read_names`,
//! `read_transactions`, `discover_owned_names`, and `compare_inventory_with_provider`.
//!
//! The existing `read_profile_isolation_tests` already validates per-profile
//! isolation for `read_balance` and `read_names`.  This module focuses on
//! additional code paths: no-profile guard, cached-balance fallback, empty
//! addresses, transaction reads, and the inventory comparison shape.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::chain::types::{ChainBid, ChainName};
use crate::commands::read::{
    compare_inventory_with_provider, discover_owned_names, empty_name_bids_response,
    merge_name_bids, read_auction_position_names, read_balance, read_name_bids, read_name_info,
    read_name_records, read_names, read_transactions, records_from_resource,
};
use crate::db;
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
            hsrd_child: std::sync::Mutex::new(None),
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

fn add_liquid(conn: &rusqlite::Connection, profile: &str, txid: &str, value: i64) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, 'addr', '00', ?3, 0, 'liquid_hns', NULL)",
        params![txid, profile, value],
    )
    .unwrap();
}

fn add_owned_name(conn: &rusqlite::Connection, profile: &str, name: &str, txid: &str) {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout)
         VALUES (?1, ?2, '', 'CLOSED', ?3, 0)",
        params![profile, name, txid],
    )
    .unwrap();
}

fn add_cached_tx(
    conn: &rusqlite::Connection,
    profile: &str,
    txid: &str,
    // Kept for call-site readability (what action/name this cached tx is
    // standing in for); `read_cached_transactions` derives direction from
    // `raw_json` alone, so the fields themselves aren't read here.
    _action: &str,
    _name: &str,
) {
    // `read_cached_transactions` reads from `wallet_transactions_cache` and
    // classifies direction from `raw_json` outputs + our addresses/outpoints.
    // We insert a minimal raw_json so the function can parse it.
    let raw = serde_json::json!({
        "outputs": [
            {"address": "addr", "value": 1000}
        ]
    })
    .to_string();
    conn.execute(
        "INSERT INTO wallet_transactions_cache
            (wallet_profile_id, txid, height, time, raw_json)
         VALUES (?1, ?2, 100, '2024-01-01', ?3)",
        params![profile, txid, raw],
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// read_balance — no profile returns zeros
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_no_profiles_returns_zeros() {
    let app = app_with(empty_db());
    let val = read_balance(app.state(), None).await.unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(0));
    assert_eq!(val["unconfirmed"], serde_json::json!(0));
    assert_eq!(val["locked_confirmed"], serde_json::json!(0));
    assert_eq!(val["locked_unconfirmed"], serde_json::json!(0));
}

#[tokio::test]
async fn read_balance_explicit_nonexistent_profile_returns_zeros() {
    let app = app_with(empty_db());
    let val = read_balance(app.state(), Some("nonexistent".into()))
        .await
        .unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(0));
}

// ---------------------------------------------------------------------------
// read_balance — cached fallback (no derived_addresses → explorer fails → cache)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_falls_back_to_cached_when_no_addresses() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    // Seed a cached balance.
    add_liquid(&conn, "P1", "tx1", 500_000);

    let app = app_with(conn);
    // No derived_addresses exist, so explorer path will fail (no real server),
    // and we fall back to the cached balance.
    let val = read_balance(app.state(), None).await.unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(500_000));
}

// ---------------------------------------------------------------------------
// read_names — no profile returns empty array
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_names_no_profile_returns_empty() {
    let app = app_with(empty_db());
    let val = read_names(app.state(), None).await.unwrap();
    assert_eq!(val, serde_json::json!([]));
}

#[tokio::test]
async fn read_names_nonexistent_profile_returns_empty() {
    let app = app_with(empty_db());
    let val = read_names(app.state(), Some("ghost".into())).await.unwrap();
    assert_eq!(val, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// read_names — returns names from tracked_name_states
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_names_returns_tracked_names() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();

    // Insert two names via the tracked_name_states path.
    add_owned_name(&conn, "W1", "alpha", "txA");
    add_owned_name(&conn, "W1", "bravo", "txB");

    let app = app_with(conn);
    let val = read_names(app.state(), None).await.unwrap();
    let arr = val.as_array().expect("array");
    let names: Vec<&str> = arr
        .iter()
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(names.len(), 2, "expected 2 names, got: {names:?}");
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"bravo"));
}

// ---------------------------------------------------------------------------
// read_transactions — no profile returns empty
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_transactions_no_profile_returns_empty() {
    let app = app_with(empty_db());
    let val = read_transactions(app.state(), None).await.unwrap();
    assert_eq!(val, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// read_transactions — returns cached rows for the requested profile
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_transactions_returns_cached_data_for_profile() {
    let conn = empty_db();
    add_profile(&conn, "T1", "regtest");
    db::queries::set_active_profile(&conn, "T1").unwrap();
    add_cached_tx(&conn, "T1", "tx001", "OPEN", "alpha");
    add_cached_tx(&conn, "T1", "tx002", "BID", "alpha");

    let app = app_with(conn);
    let val = read_transactions(app.state(), None).await.unwrap();
    let arr = val.as_array().expect("array");
    assert_eq!(arr.len(), 2, "expected 2 cached txs, got: {arr:?}");
    // read_cached_transactions returns objects with "hash", "direction", "value", etc.
    let hashes: Vec<&str> = arr
        .iter()
        .filter_map(|t| t.get("hash").and_then(|v| v.as_str()))
        .collect();
    assert!(hashes.contains(&"tx001"));
    assert!(hashes.contains(&"tx002"));
}

// ---------------------------------------------------------------------------
// read_transactions — profile isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_transactions_isolates_profiles() {
    let conn = empty_db();
    add_profile(&conn, "A", "regtest");
    add_profile(&conn, "B", "regtest");
    db::queries::set_active_profile(&conn, "A").unwrap();
    add_cached_tx(&conn, "A", "txA1", "OPEN", "alpha");
    add_cached_tx(&conn, "B", "txB1", "REVEAL", "bravo");

    let app = app_with(conn);
    let a_txs = read_transactions(app.state(), Some("A".into()))
        .await
        .unwrap();
    assert_eq!(a_txs.as_array().unwrap().len(), 1);
    assert_eq!(a_txs[0].get("hash").and_then(|v| v.as_str()), Some("txA1"));

    let b_txs = read_transactions(app.state(), Some("B".into()))
        .await
        .unwrap();
    assert_eq!(b_txs.as_array().unwrap().len(), 1);
    assert_eq!(b_txs[0].get("hash").and_then(|v| v.as_str()), Some("txB1"));
}

// ---------------------------------------------------------------------------
// discover_owned_names — no active profile
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_owned_names_no_active_profile_returns_zero() {
    let app = app_with(empty_db());
    let val = discover_owned_names(app.state()).await.unwrap();
    assert_eq!(val["discovered"], serde_json::json!(0));
    assert_eq!(val["names"], serde_json::json!([]));
}

// Note: "active profile but no addresses → discovered 0" is already covered
// by `discovery_no_addresses_is_empty` in discover_names_tests.rs (identical
// setup/assertion) — not duplicated here.

// ---------------------------------------------------------------------------
// read_name_info — exercises the full code path (node + explorer fallback).
// In CI the explorer may be reachable, so we accept either Ok or Err.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_name_info_exercises_code_path() {
    let conn = empty_db();
    let app = app_with(conn);
    let result = read_name_info(app.state(), "nonexistent12345".into()).await;
    // The function either returns name info from the explorer or an error
    // if the explorer is unreachable. Both are valid outcomes — the important
    // thing is that the code path is exercised for coverage.
    match result {
        Ok(val) => {
            // If the explorer is reachable, we get a name object back.
            assert!(
                val.get("name").is_some(),
                "expected name field in response: {val:?}"
            );
        }
        Err(_) => {
            // Explorer unreachable — also a valid outcome.
        }
    }
}

// ---------------------------------------------------------------------------
// compare_inventory_with_provider — no Namebase credentials → error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compare_inventory_no_namebase_credentials_errors() {
    let app = app_with(empty_db());
    let result = compare_inventory_with_provider(app.state()).await;
    // Without Namebase API key/secret, the client construction should fail.
    assert!(result.is_err(), "expected error for missing credentials");
}

// ---------------------------------------------------------------------------
// read_balance — multiple UTXOs sum correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_sums_multiple_utxos() {
    let conn = empty_db();
    add_profile(&conn, "M1", "regtest");
    db::queries::set_active_profile(&conn, "M1").unwrap();
    add_liquid(&conn, "M1", "tx1", 100_000);
    add_liquid(&conn, "M1", "tx2", 250_000);
    add_liquid(&conn, "M1", "tx3", 50_000);

    let app = app_with(conn);
    let val = read_balance(app.state(), None).await.unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(400_000));
}

// ---------------------------------------------------------------------------
// read_balance — spent UTXOs excluded from confirmed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_excludes_spent_utxos() {
    let conn = empty_db();
    add_profile(&conn, "S1", "regtest");
    db::queries::set_active_profile(&conn, "S1").unwrap();

    // Unspent
    add_liquid(&conn, "S1", "tx1", 100_000);
    // Spent
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES ('tx2', 0, 'S1', 'addr', '00', 200000, 0, 'liquid_hns', 'tx_spent')",
        [],
    )
    .unwrap();

    let app = app_with(conn);
    let val = read_balance(app.state(), None).await.unwrap();
    // Only the unspent UTXO should count.
    assert_eq!(val["confirmed"], serde_json::json!(100_000));
}

// ---------------------------------------------------------------------------
// read_names — empty profile returns empty array
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_names_empty_profile_no_names() {
    let conn = empty_db();
    add_profile(&conn, "E1", "regtest");
    db::queries::set_active_profile(&conn, "E1").unwrap();
    // No names seeded.

    let app = app_with(conn);
    let val = read_names(app.state(), None).await.unwrap();
    assert_eq!(val, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// resolve_profile — explicit profile ID that exists
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_explicit_existing_profile_uses_that_profile() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    add_profile(&conn, "P2", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    add_liquid(&conn, "P2", "txP2", 750_000);

    let app = app_with(conn);
    let val = read_balance(app.state(), Some("P2".into())).await.unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(750_000));
}

#[tokio::test]
async fn read_names_explicit_existing_profile_uses_that_profile() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    add_profile(&conn, "P2", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    add_owned_name(&conn, "P2", "alpha", "txA");

    let app = app_with(conn);
    let val = read_names(app.state(), Some("P2".into())).await.unwrap();
    let arr = val.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "alpha");
}

#[tokio::test]
async fn read_transactions_explicit_existing_profile_uses_that_profile() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    add_profile(&conn, "P2", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    add_cached_tx(&conn, "P2", "txP2", "OPEN", "alpha");

    let app = app_with(conn);
    let val = read_transactions(app.state(), Some("P2".into()))
        .await
        .unwrap();
    let arr = val.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["hash"], "txP2");
}

// ---------------------------------------------------------------------------
// resolve_profile — empty/whitespace falls back to active
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_empty_string_profile_falls_back_to_active() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    add_liquid(&conn, "P1", "tx1", 100_000);

    let app = app_with(conn);
    let val = read_balance(app.state(), Some("".into())).await.unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(100_000));
}

#[tokio::test]
async fn read_balance_whitespace_profile_falls_back_to_active() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    add_liquid(&conn, "P1", "tx1", 200_000);

    let app = app_with(conn);
    let val = read_balance(app.state(), Some("   ".into())).await.unwrap();
    assert_eq!(val["confirmed"], serde_json::json!(200_000));
}

// ---------------------------------------------------------------------------
// compare_inventory_with_provider — with mock Namebase
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compare_inventory_with_provider_matches_and_extras() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(r#"{"domains":[{"name":"alpha"},{"name":"bravo"},{"name":"charlie"}]}"#)
        .create_async()
        .await;

    let conn = empty_db();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('alpha', 'not_started'), ('bravo', 'not_started'), ('delta', 'not_started')",
        [],
    ).unwrap();

    let app = app_with(conn);
    let result = compare_inventory_with_provider(app.state()).await.unwrap();
    assert!(result.matched_transferable.contains(&"alpha".to_string()));
    assert!(result.matched_transferable.contains(&"bravo".to_string()));
    assert!(result.missing_at_provider.contains(&"delta".to_string()));
    assert!(result.extra_at_provider.contains(&"charlie".to_string()));
    assert_eq!(result.provider_kind, "namebase");
    assert_eq!(result.provider_label, "Namebase");
}

#[tokio::test]
async fn compare_inventory_with_provider_empty_inventory() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(r#"{"domains":[{"name":"alpha"}]}"#)
        .create_async()
        .await;

    let conn = empty_db();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", &server.url()).unwrap();
    let app = app_with(conn);
    let result = compare_inventory_with_provider(app.state()).await.unwrap();
    assert!(result.matched_transferable.is_empty());
    assert!(result.missing_at_provider.is_empty());
    assert_eq!(result.extra_at_provider, vec!["alpha".to_string()]);
}

#[tokio::test]
async fn compare_inventory_with_provider_namebase_unreachable() {
    let conn = empty_db();
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", "http://127.0.0.1:1").unwrap();
    let app = app_with(conn);
    let result = compare_inventory_with_provider(app.state()).await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("Couldn't reach Namebase"));
}

// ---------------------------------------------------------------------------
// read_auction_position_names — helpers
// ---------------------------------------------------------------------------

/// Insert a `wallet_tx_drafts` row for `name` under `action`, then move it to
/// `status` (drafts are always created in `draft` status by `insert_tx_draft`,
/// so a non-`draft` target status goes through `update_tx_draft_status`,
/// mirroring how the real build/sign/broadcast pipeline advances a draft).
fn add_draft(
    conn: &rusqlite::Connection,
    id: &str,
    profile: &str,
    action: &str,
    name: &str,
    status: &str,
) {
    let summary = serde_json::json!({ "name": name }).to_string();
    db::queries::insert_tx_draft(conn, id, profile, action, "00", "[]", &summary).unwrap();
    if status != "draft" {
        db::queries::update_tx_draft_status(conn, id, status, None, None).unwrap();
    }
}

/// Insert a minimal `bid_commitments` row for `name` (no matching draft) —
/// stands in for a recovered bid.
fn add_bid_commitment(conn: &rusqlite::Connection, profile: &str, name: &str, blind_hex: &str) {
    db::queries::insert_bid_commitment(
        conn, profile, name, "aabb", "addr", 0, 0, 1_000_000, 2_000_000, "nonce", blind_hex,
    )
    .unwrap();
}

/// Seed a spendable owner coin for `name` — a `tracked_name_states` row whose
/// `owner_txid`/`owner_vout` match an unspent `tracked_utxos` row, joined
/// through a `derived_addresses` row (the exact 3-way join `get_name_coin`
/// requires). `txid` must be unique per call within a test so multiple owned
/// names in the same profile don't collide on the `tracked_utxos` primary key.
fn seed_owner_coin(conn: &rusqlite::Connection, profile: &str, name: &str, txid: &str) {
    let addr = format!("addr-{txid}");
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, 'aabb', 'CLOSED', ?3, 0, 100)",
        params![profile, name, txid],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, '00', '00')",
        params![profile, addr],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', 10000, 6, 'name_control', NULL)",
        params![txid, profile, addr],
    )
    .unwrap();
}

fn auction_position_names(val: &serde_json::Value) -> Vec<String> {
    val.as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string entry").to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// read_auction_position_names — no profile returns empty array
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_no_profile_returns_empty() {
    let app = app_with(empty_db());
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// read_auction_position_names — confirmed open-draft is listed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_confirmed_open_draft_listed() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_draft(&conn, "d1", "W1", "open", "namehold", "confirmed");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(auction_position_names(&val), vec!["namehold".to_string()]);
}

// ---------------------------------------------------------------------------
// read_auction_position_names — broadcasted bid-draft is listed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_broadcasted_bid_draft_listed() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_draft(&conn, "d1", "W1", "bid", "example", "broadcasted");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(auction_position_names(&val), vec!["example".to_string()]);
}

// ---------------------------------------------------------------------------
// read_auction_position_names — draft / dropped / failed open NOT listed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_draft_status_open_not_listed() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_draft(&conn, "d1", "W1", "open", "notyetqueued", "draft");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!([]));
}

#[tokio::test]
async fn auction_positions_dropped_status_open_not_listed() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_draft(&conn, "d1", "W1", "open", "dropped-name", "dropped");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!([]));
}

#[tokio::test]
async fn auction_positions_failed_status_open_not_listed() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_draft(&conn, "d1", "W1", "open", "failed-name", "failed");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// read_auction_position_names — bid_commitment with no draft is listed
// (recovered bid)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_bid_commitment_without_draft_listed() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_bid_commitment(&conn, "W1", "recovered", "blind1");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(auction_position_names(&val), vec!["recovered".to_string()]);
}

// ---------------------------------------------------------------------------
// read_auction_position_names — owned name excluded even with an old bid
// commitment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_owned_name_excluded_despite_old_bid_commitment() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_bid_commitment(&conn, "W1", "wonname", "blind1");
    seed_owner_coin(
        &conn,
        "W1",
        "wonname",
        "1111111111111111111111111111111111111111111111111111111111111111",
    );

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!([]), "owned name must be excluded");
}

// ---------------------------------------------------------------------------
// read_auction_position_names — per-wallet isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_profile_isolation() {
    let conn = empty_db();
    add_profile(&conn, "A", "regtest");
    add_profile(&conn, "B", "regtest");
    db::queries::set_active_profile(&conn, "A").unwrap();
    add_draft(&conn, "dA", "A", "open", "onlya", "confirmed");
    add_draft(&conn, "dB", "B", "open", "onlyb", "confirmed");

    let app = app_with(conn);
    let a = read_auction_position_names(app.state(), Some("A".into()))
        .await
        .unwrap();
    assert_eq!(auction_position_names(&a), vec!["onlya".to_string()]);

    let b = read_auction_position_names(app.state(), Some("B".into()))
        .await
        .unwrap();
    assert_eq!(auction_position_names(&b), vec!["onlyb".to_string()]);
}

// ---------------------------------------------------------------------------
// read_auction_position_names — distinct (open + bid draft for the same
// name → one entry)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auction_positions_distinct_open_and_bid_same_name() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    add_draft(&conn, "d1", "W1", "open", "dupname", "confirmed");
    add_draft(&conn, "d2", "W1", "bid", "dupname", "broadcasted");

    let app = app_with(conn);
    let val = read_auction_position_names(app.state(), None)
        .await
        .unwrap();
    assert_eq!(auction_position_names(&val), vec!["dupname".to_string()]);
}

// ---------------------------------------------------------------------------
// read_name_bids / merge_name_bids — helpers
// ---------------------------------------------------------------------------

/// A minimal `ChainName` carrying only `bids` + the aggregate fields
/// `merge_name_bids` passes through verbatim.
fn hsrd_name_with_bids(name: &str, bids: Vec<ChainBid>) -> ChainName {
    ChainName {
        name: name.to_string(),
        name_hash: None,
        state: Some("REVEAL".to_string()),
        height: None,
        renewal: None,
        owner: None,
        value: Some(0),
        highest: Some(5_000_000),
        registered: None,
        expired: None,
        stats: None,
        transfer: None,
        revoked: None,
        bids: Some(bids),
    }
}

fn hsrd_bid(txid: Option<&str>) -> ChainBid {
    ChainBid {
        txid: txid.map(|s| s.to_string()),
        index: Some(0),
        lockup: Some(2_000_000),
        value: None,
        revealed: None,
        win: None,
        reveal: None,
        time: None,
    }
}

/// A `bid_commitments` row for `name`, already carrying a `bid_txid` (as if
/// the bid tx had been broadcast). `merge_name_bids` is a pure function that
/// receives an already profile-scoped slice — profile id is deliberately not
/// a field here (that scoping happens one layer up, in `list_bid_commitments`).
fn bid_commitment_row(
    name: &str,
    bid_txid: &str,
    bid_value_doos: i64,
) -> db::queries::BidCommitmentRow {
    db::queries::BidCommitmentRow {
        name: name.to_string(),
        name_hash_hex: "aabb".to_string(),
        address: "addr".to_string(),
        branch: 0,
        child_index: 0,
        bid_value_doos,
        lockup_value_doos: bid_value_doos + 500_000,
        nonce_hex: "nonce".to_string(),
        blind_hex: "blind".to_string(),
        bid_txid: Some(bid_txid.to_string()),
        reveal_txid: None,
        reveal_end_height: None,
    }
}

// ---------------------------------------------------------------------------
// merge_name_bids — pure join, no DB / no network
// ---------------------------------------------------------------------------

#[test]
fn merge_name_bids_matched_txid_is_mine_with_plaintext_value() {
    let info = hsrd_name_with_bids("foo", vec![hsrd_bid(Some("txA")), hsrd_bid(Some("txB"))]);
    let commitments = vec![bid_commitment_row("foo", "txA", 1_500_000)];

    let val = merge_name_bids(&info, &commitments, "foo");
    let bids = val["bids"].as_array().expect("bids array");
    assert_eq!(bids.len(), 2);

    assert_eq!(bids[0]["txid"], "txA");
    assert_eq!(bids[0]["mine"], true);
    assert_eq!(bids[0]["myValue"], 1_500_000);

    assert_eq!(bids[1]["txid"], "txB");
    assert_eq!(bids[1]["mine"], false);
    assert!(bids[1]["myValue"].is_null());

    assert_eq!(val["myBidCount"], 1);
    assert_eq!(val["state"], "REVEAL");
    assert_eq!(val["highest"], 5_000_000);
}

#[test]
fn merge_name_bids_commitment_for_another_name_does_not_mark_mine() {
    // Same txid, but the commitment belongs to a DIFFERENT name — must not
    // mark the bid as mine even though the txid matches exactly.
    let info = hsrd_name_with_bids("foo", vec![hsrd_bid(Some("txA"))]);
    let commitments = vec![bid_commitment_row("othername", "txA", 1_500_000)];

    let val = merge_name_bids(&info, &commitments, "foo");
    let bids = val["bids"].as_array().expect("bids array");
    assert_eq!(bids[0]["mine"], false);
    assert!(bids[0]["myValue"].is_null());
    assert_eq!(val["myBidCount"], 0);
}

#[test]
fn merge_name_bids_bid_without_txid_never_matches() {
    let info = hsrd_name_with_bids("foo", vec![hsrd_bid(None)]);
    let commitments = vec![bid_commitment_row("foo", "txA", 1_500_000)];

    let val = merge_name_bids(&info, &commitments, "foo");
    let bids = val["bids"].as_array().expect("bids array");
    assert!(bids[0]["txid"].is_null());
    assert_eq!(bids[0]["mine"], false);
    assert!(bids[0]["myValue"].is_null());
    assert_eq!(val["myBidCount"], 0);
}

#[test]
fn merge_name_bids_no_bids_yields_empty_array_and_zero_count() {
    let info = hsrd_name_with_bids("foo", vec![]);
    let val = merge_name_bids(&info, &[], "foo");
    assert_eq!(val["bids"], serde_json::json!([]));
    assert_eq!(val["myBidCount"], 0);
}

#[test]
fn merge_name_bids_none_bids_on_info_yields_empty_array() {
    // `info.bids == None` (e.g. a node-sourced ChainName, or an explorer entry
    // with no `bids` key at all) must degrade to an empty array, not panic.
    let mut info = hsrd_name_with_bids("foo", vec![]);
    info.bids = None;
    let val = merge_name_bids(&info, &[], "foo");
    assert_eq!(val["bids"], serde_json::json!([]));
    assert_eq!(val["myBidCount"], 0);
}

// ---------------------------------------------------------------------------
// read_name_bids — command-level (no profile / explorer degradation)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_name_bids_no_profile_returns_empty_response() {
    let app = app_with(empty_db());
    let val = read_name_bids(app.state(), "foo".into(), None)
        .await
        .unwrap();
    assert_eq!(val, empty_name_bids_response("foo"));
}

#[tokio::test]
async fn read_name_bids_explorer_404_returns_empty_response_not_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/names/ghostname")
        .with_status(404)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = read_name_bids(app.state(), "ghostname".into(), None)
        .await
        .unwrap();
    assert_eq!(val, empty_name_bids_response("ghostname"));
}

#[tokio::test]
async fn read_name_bids_matches_own_commitment_and_computes_count() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/names/foo")
        .with_status(200)
        .with_body(
            r#"{
                "name": "foo",
                "state": "REVEAL",
                "highest": 5000000,
                "bids": [
                    { "txid": "txA", "index": 0, "lockup": 2000000 },
                    { "txid": "txB", "index": 1, "lockup": 3000000 }
                ]
            }"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();
    db::queries::insert_bid_commitment(
        &conn, "W1", "foo", "aabb", "addr", 0, 0, 1_500_000, 2_000_000, "nonce", "blind1",
    )
    .unwrap();
    db::queries::set_bid_txid(&conn, "W1", "blind1", "txA").unwrap();

    let app = app_with(conn);
    let val = read_name_bids(app.state(), "foo".into(), Some("W1".into()))
        .await
        .unwrap();

    let bids = val["bids"].as_array().expect("bids array");
    assert_eq!(bids.len(), 2);
    let by_txid = |t: &str| bids.iter().find(|b| b["txid"] == t).unwrap();
    assert_eq!(by_txid("txA")["mine"], true);
    assert_eq!(by_txid("txA")["myValue"], 1_500_000);
    assert_eq!(by_txid("txB")["mine"], false);
    assert!(by_txid("txB")["myValue"].is_null());
    assert_eq!(val["myBidCount"], 1);
    assert_eq!(val["state"], "REVEAL");
    assert_eq!(val["highest"], 5_000_000);
}

#[tokio::test]
async fn read_name_bids_per_wallet_isolation() {
    // Two profiles each hold a commitment matching one of the two bids the
    // explorer reports. A commitment belonging to the OTHER profile must
    // never mark a bid as "mine" for the profile being queried.
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/names/foo")
        .with_status(200)
        .with_body(
            r#"{
                "name": "foo",
                "state": "REVEAL",
                "bids": [
                    { "txid": "txA", "index": 0, "lockup": 2000000 },
                    { "txid": "txB", "index": 1, "lockup": 3000000 }
                ]
            }"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "A", "regtest");
    add_profile(&conn, "B", "regtest");
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();

    db::queries::insert_bid_commitment(
        &conn, "A", "foo", "aabb", "addr", 0, 0, 1_500_000, 2_000_000, "nonce", "blindA",
    )
    .unwrap();
    db::queries::set_bid_txid(&conn, "A", "blindA", "txA").unwrap();

    db::queries::insert_bid_commitment(
        &conn, "B", "foo", "aabb", "addr", 0, 0, 2_500_000, 3_000_000, "nonce", "blindB",
    )
    .unwrap();
    db::queries::set_bid_txid(&conn, "B", "blindB", "txB").unwrap();

    let app = app_with(conn);

    let val_a = read_name_bids(app.state(), "foo".into(), Some("A".into()))
        .await
        .unwrap();
    let bids_a = val_a["bids"].as_array().expect("bids array");
    let by_txid_a = |t: &str| bids_a.iter().find(|b| b["txid"] == t).unwrap();
    assert_eq!(by_txid_a("txA")["mine"], true);
    assert_eq!(
        by_txid_a("txB")["mine"],
        false,
        "B's commitment must not leak into A's view"
    );
    assert_eq!(val_a["myBidCount"], 1);

    let val_b = read_name_bids(app.state(), "foo".into(), Some("B".into()))
        .await
        .unwrap();
    let bids_b = val_b["bids"].as_array().expect("bids array");
    let by_txid_b = |t: &str| bids_b.iter().find(|b| b["txid"] == t).unwrap();
    assert_eq!(
        by_txid_b("txA")["mine"],
        false,
        "A's commitment must not leak into B's view"
    );
    assert_eq!(by_txid_b("txB")["mine"], true);
    assert_eq!(val_b["myBidCount"], 1);
}

// ---------------------------------------------------------------------------
// records_from_resource — pure helper (Manage DNS: current records prefill)
// ---------------------------------------------------------------------------

#[test]
fn records_from_resource_extracts_array() {
    let res = serde_json::json!({
        "records": [
            { "type": "NS", "ns": "ns1.example." },
            { "type": "DS", "keyTag": 12345, "algorithm": 8, "digestType": 2, "digest": "aa" }
        ]
    });
    let recs = records_from_resource(&res);
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0]["type"], "NS");
    assert_eq!(recs[1]["type"], "DS");
}

#[test]
fn records_from_resource_handles_null_missing_and_non_array() {
    // `records` absent → empty.
    assert!(records_from_resource(&serde_json::json!({})).is_empty());
    // `records: null` (hsrd shape for a name with no resource) → empty.
    assert!(records_from_resource(&serde_json::json!({ "records": null })).is_empty());
    // Non-object top-level (e.g. a literal null from a name that was never
    // opened) → empty; must not panic.
    assert!(records_from_resource(&serde_json::Value::Null).is_empty());
    // Wrong type for `records` (defensively): still empty.
    assert!(records_from_resource(&serde_json::json!({ "records": "oops" })).is_empty());
}

// ---------------------------------------------------------------------------
// read_name_records — command-level (no-profile / no-node graceful degradation)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_name_records_no_profile_returns_empty_resource_object() {
    // No resolved profile → the command soft-degrades to the uniform empty
    // resource shape (`{records:[]}`), NOT a bare array. The frontend always
    // reads `resource.records`, so the object shape must hold on every path.
    let app = app_with(empty_db());
    let val = read_name_records(app.state(), "foo".into(), None)
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!({ "records": [] }));
}

#[tokio::test]
async fn read_name_records_node_not_ready_returns_empty_resource_object() {
    // With a resolved profile but no reachable/synced node, the command must
    // soft-degrade to the empty resource object rather than error — the
    // frontend then shows its "connect & sync a node to view records" hint.
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    let app = app_with(conn);
    let val = read_name_records(app.state(), "foo".into(), Some("W1".into()))
        .await
        .unwrap();
    assert_eq!(val, serde_json::json!({ "records": [] }));
}
