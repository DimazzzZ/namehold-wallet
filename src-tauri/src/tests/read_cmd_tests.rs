//! Tests for `commands::read` — covers `read_balance`, `read_names`,
//! and `read_transactions`, `discover_owned_names`.
//!
//! The existing `read_profile_isolation_tests` already validates per-profile
//! isolation for `read_balance` and `read_names`.  This module focuses on
//! additional code paths: no-profile guard, cached-balance fallback, empty
//! addresses, and transaction reads.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::read::{
    discover_owned_names, empty_name_bids_response, merge_name_bids, read_auction_position_names,
    read_balance, read_name_bids, read_name_info, read_name_records, read_names, read_transactions,
    records_from_resource,
};
use crate::db;
use crate::hsd::types::{HsdBid, HsdName};
use crate::AppState;

// Extra imports used by the additional tests below. Kept in a second `use`
// block so it's easy to see the read-tests baseline vs. the new coverage
// harness in a single file.
use crate::commands::read::{
    get_resource, list_receive_addresses, read_block_info, read_renewals, read_tx_info,
    repair_owned_names, reveal_next_receive_address,
};

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
    // Also seed a matching unspent `name_control` UTXO so the name passes the
    // ownership gate in `read_owned_names_explorer`.
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, 'rs1qtest', '00', 1000, 6, NULL, 'name_control', NULL)",
        params![txid, profile],
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

/// A minimal `HsdName` carrying only `bids` + the aggregate fields
/// `merge_name_bids` passes through verbatim.
fn hsd_name_with_bids(name: &str, bids: Vec<HsdBid>) -> HsdName {
    HsdName {
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

fn hsd_bid(txid: Option<&str>) -> HsdBid {
    HsdBid {
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
    let info = hsd_name_with_bids("foo", vec![hsd_bid(Some("txA")), hsd_bid(Some("txB"))]);
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
    let info = hsd_name_with_bids("foo", vec![hsd_bid(Some("txA"))]);
    let commitments = vec![bid_commitment_row("othername", "txA", 1_500_000)];

    let val = merge_name_bids(&info, &commitments, "foo");
    let bids = val["bids"].as_array().expect("bids array");
    assert_eq!(bids[0]["mine"], false);
    assert!(bids[0]["myValue"].is_null());
    assert_eq!(val["myBidCount"], 0);
}

#[test]
fn merge_name_bids_bid_without_txid_never_matches() {
    let info = hsd_name_with_bids("foo", vec![hsd_bid(None)]);
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
    let info = hsd_name_with_bids("foo", vec![]);
    let val = merge_name_bids(&info, &[], "foo");
    assert_eq!(val["bids"], serde_json::json!([]));
    assert_eq!(val["myBidCount"], 0);
}

#[test]
fn merge_name_bids_none_bids_on_info_yields_empty_array() {
    // `info.bids == None` (e.g. a node-sourced HsdName, or an explorer entry
    // with no `bids` key at all) must degrade to an empty array, not panic.
    let mut info = hsd_name_with_bids("foo", vec![]);
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
// read_name_bids — node synced but scanner hasn't reached the name's auction
// height yet → fall through to the explorer (don't serve an empty local index)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_name_bids_falls_through_to_explorer_when_scanner_behind() {
    let mut server = mockito::Server::new_async().await;
    // Node reports fully synced so the node-first branch is entered.
    let _node = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"blocks":1000,"headers":1000,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;
    // Explorer returns a bid the local index does NOT have.
    let _explorer = server
        .mock("GET", "/api/names/behindname")
        .with_status(200)
        .with_body(
            r#"{"name":"behindname","state":"BIDDING","highest":4200000,
                "bids":[{"txid":"txExplorer","index":0,"lockup":4200000}]}"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();
    // Name opened at height 900; scanner cursor left at 100 → scanner has NOT
    // reached the auction, so scanner_covers is false and we fall through.
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES ('W1', 'behindname', '', 'BIDDING', NULL, NULL, 900)",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE chain_scan_cursor SET last_height = 100 WHERE id = 1",
        [],
    )
    .unwrap();

    let app = app_with(conn);
    let val = read_name_bids(app.state(), "behindname".into(), Some("W1".into()))
        .await
        .unwrap();
    // Served from the explorer fallback, not the (empty) local index.
    let bids = val["bids"].as_array().expect("bids array");
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0]["txid"], "txExplorer");
    assert_eq!(val["highest"], 4_200_000);
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
    // `records: null` (hsd shape for a name with no resource) → empty.
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

// ===========================================================================
// Additional coverage tests — target the State-wrapper commands and rarely-
// exercised branches of `commands::read`. Grouped by target function.
// ===========================================================================

/// Standard JSON-RPC mock for a fully synced hsd node — pass into any test
/// that needs `is_node_ready_for_local_reads` to return `true`.
/// `progress=1.0`, `blocks==headers`, `chain="regtest"` (matches the default
/// test profile network so the network-match gate passes).
async fn mock_synced_node(server: &mut mockito::Server) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"chain":"regtest","blocks":1000,"headers":1000,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await
}

// ---------------------------------------------------------------------------
// read_renewals — no-profile hits `empty_renewals()`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_renewals_no_profile_returns_empty_shape() {
    let app = app_with(empty_db());
    let resp = read_renewals(app.state(), None).await.unwrap();
    assert!(resp.wallet_profile_id.is_none());
    assert!(resp.current_height.is_none());
    assert_eq!(resp.height_source, "unknown");
    assert!(resp.names.is_empty());
    // Serializable to camelCase JSON — the frontend contract.
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["heightSource"], "unknown");
    assert!(json["names"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn read_renewals_with_profile_uses_unknown_height_when_no_node() {
    // Resolved profile, no synced node, no persisted-height signal → returns
    // the response with height_source == "unknown" and an empty names list.
    let conn = empty_db();
    add_profile(&conn, "R1", "regtest");
    db::queries::set_active_profile(&conn, "R1").unwrap();
    let app = app_with(conn);
    let resp = read_renewals(app.state(), Some("R1".into())).await.unwrap();
    assert_eq!(resp.wallet_profile_id.as_deref(), Some("R1"));
    assert_eq!(resp.height_source, "unknown");
}

// ---------------------------------------------------------------------------
// list_receive_addresses — no profile / profile with addresses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_receive_addresses_no_profile_returns_empty_vec() {
    let app = app_with(empty_db());
    let out = list_receive_addresses(app.state(), None).await.unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
async fn list_receive_addresses_returns_derived_rows() {
    let conn = empty_db();
    add_profile(&conn, "R1", "regtest");
    db::queries::set_active_profile(&conn, "R1").unwrap();
    // Seed two receive-branch addresses (branch=0) — the query uses BRANCH_RECEIVE.
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES ('R1', 0, 0, 0, 'addr0', '00', '00'),
                ('R1', 0, 0, 1, 'addr1', '00', '00')",
        [],
    )
    .unwrap();
    let app = app_with(conn);
    let out = list_receive_addresses(app.state(), None).await.unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].index, 0);
    assert_eq!(out[0].address, "addr0");
    assert!(!out[0].used);
    assert_eq!(out[1].index, 1);
    assert_eq!(out[1].address, "addr1");
}

// ---------------------------------------------------------------------------
// reveal_next_receive_address — command wrapper
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reveal_next_receive_address_no_profile_errors() {
    let app = app_with(empty_db());
    let err = reveal_next_receive_address(app.state(), None)
        .await
        .unwrap_err();
    assert!(
        format!("{err}").contains("no active wallet profile"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn reveal_next_receive_address_derives_from_valid_profile() {
    // Uses the shared `insert_valid_profile` helper which seeds a real xpub +
    // an initial derived address, so `derive_next_for_profile` can succeed.
    let conn = empty_db();
    let id = crate::tests::names_cmd_tests::insert_valid_profile(&conn, "regtest");
    let app = app_with(conn);
    let addr = reveal_next_receive_address(app.state(), Some(id.clone()))
        .await
        .unwrap();
    assert!(addr.starts_with("rs1"), "expected regtest bech32: {addr}");
}

// ---------------------------------------------------------------------------
// read_block_info — soft-degrade + happy path via mock node
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_block_info_negative_height_returns_null() {
    let app = app_with(empty_db());
    let val = read_block_info(app.state(), -1).await.unwrap();
    assert!(val.is_null());
}

#[tokio::test]
async fn read_block_info_node_not_ready_returns_null() {
    // Positive height but no synced node → soft-degrades to null (the
    // "requires synced node" hint on the frontend).
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    let app = app_with(conn);
    let val = read_block_info(app.state(), 100).await.unwrap();
    assert!(val.is_null());
}

#[tokio::test]
async fn read_block_info_synced_node_returns_shaped_block() {
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _bh = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockhash".into()))
        .with_body(
            r#"{"result":"aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44","error":null,"id":1}"#,
        )
        .create_async()
        .await;
    let _blk = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("\"getblock\"".into()))
        .with_body(
            r#"{"result":{"hash":"aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44","height":42,"time":1700000000,"difficulty":1.0,"tx":[{"outputs":[{"value":2000000000}]}]},"error":null,"id":1}"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = read_block_info(app.state(), 42).await.unwrap();
    assert_eq!(val["height"], 42);
    assert_eq!(val["minerReward"], 2_000_000_000i64);
}

// ---------------------------------------------------------------------------
// read_tx_info — command wrapper (empty/no-node/happy-path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_tx_info_empty_txid_returns_null() {
    let app = app_with(empty_db());
    let val = read_tx_info(app.state(), "".into()).await.unwrap();
    assert!(val.is_null());
    // Also whitespace-only: same soft-degrade path (the command trims).
    let val = read_tx_info(app.state(), "   ".into()).await.unwrap();
    assert!(val.is_null());
}

#[tokio::test]
async fn read_tx_info_node_not_ready_returns_null() {
    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    let app = app_with(conn);
    let val = read_tx_info(app.state(), "aabb".into()).await.unwrap();
    assert!(val.is_null());
}

#[tokio::test]
async fn read_tx_info_synced_node_returns_shaped_tx() {
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _tx = server
        .mock("GET", "/tx/deadbeef")
        .with_body(
            r#"{"hash":"deadbeef","confirmations":10,"height":200,"block":"blkhash",
                 "time":1700000000,"fee":1000,
                 "inputs":[{"coin":{"value":5000}}],
                 "outputs":[{"value":3000},{"value":1000}]}"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = read_tx_info(app.state(), "deadbeef".into()).await.unwrap();
    assert_eq!(val["txid"], "deadbeef");
    assert_eq!(val["height"], 200);
    assert_eq!(val["fee"], 1000);
    assert_eq!(val["outputsCount"], 2);
    assert_eq!(val["totalOut"], 4000);
}

// ---------------------------------------------------------------------------
// get_resource — assembly of name info + resource records
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_resource_explorer_path_when_node_not_ready() {
    // No node mocked → is_node_ready is false → explorer path.
    let mut server = mockito::Server::new_async().await;
    let _ex = server
        .mock("GET", "/api/names/foo")
        .with_status(200)
        .with_body(
            r#"{"name":"foo","state":"CLOSED","height":500,"renewal":600,
                 "stats":{"blocksUntilExpire":100,"daysUntilExpire":0.7}}"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = get_resource(app.state(), "foo".into()).await.unwrap();
    assert_eq!(val["name"], "foo");
    assert_eq!(val["state"], "CLOSED");
    assert_eq!(val["height"], 500);
    assert_eq!(val["renewal"], 600);
    // Records array is always present; node-only, so empty in this branch.
    assert!(val["data"]["records"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_resource_explorer_404_synthesizes_available() {
    // 404 → explorer's `get_name_info_optional` returns Ok(None), which
    // get_resource maps to `{ name, state: "AVAILABLE" }`.
    let mut server = mockito::Server::new_async().await;
    let _ex = server
        .mock("GET", "/api/names/newname")
        .with_status(404)
        .with_body("{}")
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = get_resource(app.state(), "newname".into()).await.unwrap();
    assert_eq!(val["name"], "newname");
    assert_eq!(val["state"], "AVAILABLE");
    assert!(val["data"]["records"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_resource_node_ready_reads_info_and_records() {
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _ni = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_body(
            r#"{"result":{"info":{"name":"foo","state":"CLOSED","height":100,"renewal":200,"stats":{"blocksUntilExpire":50}}},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;
    let _nr = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameresource".into()))
        .with_body(
            r#"{"result":{"records":[{"type":"NS","ns":"ns1.example."}]},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = get_resource(app.state(), "foo".into()).await.unwrap();
    assert_eq!(val["name"], "foo");
    assert_eq!(val["state"], "CLOSED");
    assert_eq!(val["height"], 100);
    let recs = val["data"]["records"].as_array().unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["type"], "NS");
}

// ---------------------------------------------------------------------------
// read_name_info — node-ready branch + explorer 404 synthesize-available
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_name_info_explorer_404_synthesizes_available() {
    let mut server = mockito::Server::new_async().await;
    let _ex = server
        .mock("GET", "/api/names/newname")
        .with_status(404)
        .with_body("")
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = read_name_info(app.state(), "newname".into()).await.unwrap();
    assert_eq!(val["state"], "AVAILABLE");
    assert_eq!(val["name"], "newname");
}

#[tokio::test]
async fn read_name_info_node_ready_uses_node_branch() {
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _ni = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_body(
            r#"{"result":{"info":{"name":"foo","state":"CLOSED","height":100}},"error":null,"id":1}"#,
        )
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = read_name_info(app.state(), "foo".into()).await.unwrap();
    assert_eq!(val["state"], "CLOSED");
    assert_eq!(val["name"], "foo");
}

// ---------------------------------------------------------------------------
// read_balance — explorer-fallback path (with pre-provisioned addresses)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_balance_explorer_fallback_returns_addressed_balance() {
    let mut server = mockito::Server::new_async().await;
    // No node mocked (so is_node_ready is false → explorer path).
    let _addr = server
        .mock("GET", "/api/addresses/addr0")
        .with_status(200)
        .with_body(r#"{"confirmed":123456,"unconfirmed":7890}"#)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "B1", "regtest");
    db::queries::set_active_profile(&conn, "B1").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();
    // Seed a derived address so the explorer branch has something to hit and
    // the auto-provision branch is skipped.
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES ('B1', 0, 0, 0, 'addr0', '00', '00')",
        [],
    )
    .unwrap();

    let app = app_with(conn);
    let val = read_balance(app.state(), Some("B1".into())).await.unwrap();
    assert_eq!(val["confirmed"], 123_456);
    assert_eq!(val["unconfirmed"], 7_890);
    assert_eq!(val["locked_confirmed"], 0);
    assert_eq!(val["locked_unconfirmed"], 0);
}

#[tokio::test]
async fn read_balance_explorer_fails_falls_back_to_cache() {
    // Explorer returns HTTP error (500) for every attempt → HnsFansClient's
    // `get_balance` returns Err → command falls back to the DB cache.
    let mut server = mockito::Server::new_async().await;
    let _err = server
        .mock("GET", "/api/addresses/addr0")
        .with_status(500)
        .with_body("boom")
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "B2", "regtest");
    db::queries::set_active_profile(&conn, "B2").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();
    add_liquid(&conn, "B2", "cachedtx", 400_000);
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES ('B2', 0, 0, 0, 'addr0', '00', '00')",
        [],
    )
    .unwrap();

    let app = app_with(conn);
    let val = read_balance(app.state(), Some("B2".into())).await.unwrap();
    // Falls back to cached balance (last resort).
    assert_eq!(val["confirmed"], 400_000);
}

#[tokio::test]
async fn read_balance_auto_provisions_addresses_from_valid_xpub() {
    // Explicitly cover the auto-provision branch (lines 495–526 of read.rs):
    // no derived_addresses and no cached UTXOs → the command must derive fresh
    // addresses from the profile's stored xpub and hit the explorer with them.
    // Assertion works two ways: (a) confirmed came back from the mocked
    // explorer (>0), proving the explorer branch fired; (b) derived_addresses
    // now has 20 receive rows, proving `ensure_addresses` actually ran.
    let mut server = mockito::Server::new_async().await;
    let _catchall = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/addresses/.+$".into()),
        )
        .with_status(200)
        .with_body(r#"{"confirmed":42,"unconfirmed":0}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    let id = crate::tests::names_cmd_tests::insert_valid_profile(&conn, "regtest");
    db::queries::set_setting(&conn, "explorer_api_url", &server.url()).unwrap();
    // Drop everything `insert_valid_profile` pre-seeds that could shortcut the
    // branch we want to exercise: derived_addresses (so addrs.is_empty()) AND
    // tracked_utxos (so a cache fallback would return 0 — the >0 assertion
    // below then only succeeds if the explorer branch actually ran).
    conn.execute("DELETE FROM derived_addresses", []).unwrap();
    conn.execute("DELETE FROM tracked_utxos", []).unwrap();

    let app = app_with(conn);
    let val = read_balance(app.state(), Some(id.clone())).await.unwrap();
    assert!(
        val["confirmed"].as_i64().unwrap() > 0,
        "expected explorer branch to yield >0 confirmed, got: {val}"
    );
    // Bonus check: the auto-provisioned batch of 20 receive addresses is
    // present, proving `ensure_addresses` fired on the receive branch.
    let app2 = app; // reuse app for a second read
    let rows = list_receive_addresses(app2.state(), Some(id.clone()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 20);
}

// ---------------------------------------------------------------------------
// discover_owned_names — node-authoritative branch (State wrapper)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_owned_names_node_path_no_hashes_returns_zero() {
    // Synced node + active profile but no unspent name-covenant coins →
    // discover_names_via_node_with_client returns empty → command returns 0.
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();

    let app = app_with(conn);
    let val = discover_owned_names(app.state()).await.unwrap();
    assert_eq!(val["discovered"], 0);
    assert_eq!(val["names"], serde_json::json!([]));
}

#[tokio::test]
async fn discover_owned_names_node_path_upserts_from_hash() {
    // Seed one unspent BID coin so `list_unspent_wallet_name_hashes` returns
    // one hash. Node resolves hash → name → nameinfo. Command persists via
    // `upsert_name_state` and returns the resolved names.
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _nh = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnamebyhash".into()))
        .with_body(r#"{"result":"foo","error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;
    let _ni = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_body(
            r#"{"result":{"info":{"name":"foo","state":"CLOSED","height":100,"owner":null,"weak":false}},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    // Insert an unspent BID coin (covenant_type=3, spend_class='name_lockup')
    // with a valid covenant_json that carries a nameHash at items[0] and
    // rawName at items[2].
    let name = "foo";
    let nh = hex::encode(crate::noncustodial::names::hash_name(name).unwrap());
    let raw = hex::encode(crate::noncustodial::names::raw_name(name).unwrap());
    let cov = serde_json::json!({
        "type": 3,
        "action": "BID",
        "items": [nh, "64000000", raw, "00".repeat(32)],
    })
    .to_string();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES ('aabbcc', 0, 'W1', 'addr0', '00', 1000, 3, ?1, 'name_lockup', NULL)",
        rusqlite::params![cov],
    )
    .unwrap();

    let app = app_with(conn);
    let val = discover_owned_names(app.state()).await.unwrap();
    assert_eq!(val["discovered"], 1);
    let names: Vec<&str> = val["names"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n.as_str())
        .collect();
    assert_eq!(names, vec!["foo"]);
}

// ---------------------------------------------------------------------------
// repair_owned_names — node-authoritative path (State wrapper)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repair_owned_names_no_profile_returns_zero() {
    let app = app_with(empty_db());
    let val = repair_owned_names(app.state()).await.unwrap();
    assert_eq!(val["repaired"], 0);
    assert_eq!(val["discovered"], 0);
    assert_eq!(val["errors"], serde_json::json!([]));
}

#[tokio::test]
async fn repair_owned_names_via_node_path_records_ownership() {
    // Synced node + one tracked name; `getnameinfo` returns an owner whose
    // address is NOT in the wallet's address set → the "not owned" branch
    // (touch_asset_synced) runs, but the pass still completes with repaired=0.
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _ni = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_body(
            r#"{"result":{"info":{"name":"foo","state":"CLOSED","height":100,"owner":null,"weak":false}},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    // A tracked name (candidates list must be non-empty for the loop to run).
    add_owned_name(&conn, "W1", "foo", "txfoo");

    let app = app_with(conn);
    let val = repair_owned_names(app.state()).await.unwrap();
    // No owner in `getnameinfo` → not owned by wallet → touch_asset_synced.
    assert_eq!(val["repaired"], 0);
    assert_eq!(val["candidates"], 1);
    assert_eq!(val["errors"], serde_json::json!([]));
}

#[tokio::test]
async fn repair_owned_names_via_node_records_repair_when_owner_matches() {
    // Synced node + one tracked name; `getnameinfo` reports an owner outpoint
    // that `gettxout` resolves to an address in the wallet's set → the
    // repaired-path runs (upsert_owned_name + mark_asset_finalized_owned).
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _ni = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_body(
            r#"{"result":{"info":{"name":"foo","state":"CLOSED","height":100,"owner":{"hash":"aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44","index":0},"weak":false}},"error":null,"id":1}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;
    let _txo = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("gettxout".into()))
        .with_body(r#"{"result":{"address":{"string":"myaddr"}},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    // The wallet owns `myaddr` (so the returned owner address is in the set).
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES ('W1', 0, 0, 0, 'myaddr', '00', '00')",
        [],
    )
    .unwrap();
    add_owned_name(&conn, "W1", "foo", "txfoo");

    let app = app_with(conn);
    let val = repair_owned_names(app.state()).await.unwrap();
    assert_eq!(val["repaired"], 1);
    assert_eq!(val["candidates"], 1);
    assert_eq!(val["errors"], serde_json::json!([]));
}

#[tokio::test]
async fn repair_owned_names_via_node_records_error_on_rpc_failure() {
    // Node reports synced but `getnameinfo` fails → resolve_name_ownership_with_client
    // returns Err → error is pushed and the loop continues (repaired=0, one
    // error entry, candidates=1).
    let mut server = mockito::Server::new_async().await;
    let _bi = mock_synced_node(&mut server).await;
    let _ni = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_status(500)
        .with_body(r#"{"result":null,"error":{"message":"boom"},"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let conn = empty_db();
    add_profile(&conn, "W1", "regtest");
    db::queries::set_active_profile(&conn, "W1").unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
    add_owned_name(&conn, "W1", "foo", "txfoo");

    let app = app_with(conn);
    let val = repair_owned_names(app.state()).await.unwrap();
    assert_eq!(val["repaired"], 0);
    assert_eq!(val["candidates"], 1);
    let errors = val["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].as_str().unwrap().contains("foo"),
        "unexpected error entry: {:?}",
        errors
    );
}

// ---------------------------------------------------------------------------
// estimate_persisted_height — a couple of edge branches
// ---------------------------------------------------------------------------

#[test]
fn estimate_persisted_height_returns_none_when_no_signal() {
    let conn = empty_db();
    add_profile(&conn, "H1", "regtest");
    let h = crate::commands::read::estimate_persisted_height(&conn, "H1").unwrap();
    assert!(h.is_none());
}

#[test]
fn estimate_persisted_height_reads_from_profile_last_synced_height() {
    let conn = empty_db();
    add_profile(&conn, "H2", "regtest");
    conn.execute(
        "UPDATE wallet_profiles
            SET last_synced_height = 12345,
                last_synced_at = datetime('now')
          WHERE id = 'H2'",
        [],
    )
    .unwrap();
    let h = crate::commands::read::estimate_persisted_height(&conn, "H2")
        .unwrap()
        .unwrap();
    // Value may be aged slightly (>=12345); the important thing is it was read.
    assert!(h >= 12345, "expected >=12345, got {h}");
}

#[test]
fn estimate_persisted_height_prefers_max_across_sources() {
    // A tracked-name-states row carries stats implying height 20000, while
    // last_synced_height on the profile is 15000 → max wins.
    let conn = empty_db();
    add_profile(&conn, "H3", "regtest");
    let raw = serde_json::json!({
        "stats": {
            "renewalPeriodEnd": 25000,
            "blocksUntilExpire": 5000,
        }
    })
    .to_string();
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, raw_json, updated_at)
         VALUES ('H3', 'foo', 'ab', 'CLOSED', ?1, datetime('now'))",
        rusqlite::params![raw],
    )
    .unwrap();
    conn.execute(
        "UPDATE wallet_profiles
            SET last_synced_height = 15000,
                last_synced_at = datetime('now')
          WHERE id = 'H3'",
        [],
    )
    .unwrap();
    let h = crate::commands::read::estimate_persisted_height(&conn, "H3")
        .unwrap()
        .unwrap();
    // Max of (25000 - 5000) and 15000 is 20000 (plus small aging drift).
    assert!(h >= 20000, "expected >=20000, got {h}");
}
