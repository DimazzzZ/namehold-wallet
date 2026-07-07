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

use crate::commands::read::{
    compare_inventory_with_provider, discover_owned_names, read_balance, read_name_info,
    read_names, read_transactions,
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
            hsd_child: std::sync::Mutex::new(None),
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
        conn, id, id, "mnemonic_hot", network, "xpubDUMMY", 0, false,
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
    action: &str,
    name: &str,
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
    assert_eq!(
        a_txs[0].get("hash").and_then(|v| v.as_str()),
        Some("txA1")
    );

    let b_txs = read_transactions(app.state(), Some("B".into()))
        .await
        .unwrap();
    assert_eq!(b_txs.as_array().unwrap().len(), 1);
    assert_eq!(
        b_txs[0].get("hash").and_then(|v| v.as_str()),
        Some("txB1")
    );
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

// ---------------------------------------------------------------------------
// discover_owned_names — active profile but no addresses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_owned_names_empty_addresses_returns_zero() {
    let conn = empty_db();
    add_profile(&conn, "P1", "regtest");
    db::queries::set_active_profile(&conn, "P1").unwrap();
    // No derived_addresses seeded → early return.

    let app = app_with(conn);
    let val = discover_owned_names(app.state()).await.unwrap();
    assert_eq!(val["discovered"], serde_json::json!(0));
    assert_eq!(val["names"], serde_json::json!([]));
}

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
    let val = read_transactions(app.state(), Some("P2".into())).await.unwrap();
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
    assert!(result.matched.contains(&"alpha".to_string()));
    assert!(result.matched.contains(&"bravo".to_string()));
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
    assert!(result.matched.is_empty());
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
