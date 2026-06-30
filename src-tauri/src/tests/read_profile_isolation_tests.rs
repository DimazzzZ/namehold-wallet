//! Per-wallet read isolation: `read_balance` / `read_names` must return the data
//! for the profile they're ASKED about (`wallet_profile_id`), not whatever the
//! active profile happens to be. This is what stops dashboard values from
//! swapping when the active profile changes mid-switch.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::read::{read_balance, read_names};
use crate::db;
use crate::AppState;

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

fn add_profile(conn: &rusqlite::Connection, id: &str, network: &str) {
    db::queries::insert_wallet_profile(conn, id, id, "mnemonic_hot", network, "xpubDUMMY", 0, false)
        .unwrap();
}

/// A liquid coin worth `value` doos for a profile. No derived_addresses are
/// seeded, so `read_balance` skips the explorer and reads the per-profile cache
/// deterministically (the regtest/offline path).
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

/// Two profiles A and B with distinct balances + owned names; active = A.
fn seeded() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();

    add_profile(&conn, "A", "mainnet");
    add_profile(&conn, "B", "regtest");
    db::queries::set_active_profile(&conn, "A").unwrap();

    add_liquid(&conn, "A", "a1", 100_000_000); // A: 100 HNS
    add_liquid(&conn, "B", "b1", 200_000_000); // B: 200 HNS
    add_owned_name(&conn, "A", "alpha", "a1");
    add_owned_name(&conn, "B", "bravo", "b1");
    conn
}

#[tokio::test]
async fn read_balance_honors_the_requested_profile_over_the_active_one() {
    let app = app_with(seeded()); // active = A

    // Ask for B explicitly → must get B's balance even though A is active.
    let b = read_balance(app.state(), Some("B".into())).await.unwrap();
    assert_eq!(b["confirmed"], serde_json::json!(200_000_000), "got: {b}");

    // Ask for A → A's balance.
    let a = read_balance(app.state(), Some("A".into())).await.unwrap();
    assert_eq!(a["confirmed"], serde_json::json!(100_000_000), "got: {a}");

    // No id → falls back to the active profile (A).
    let active = read_balance(app.state(), None).await.unwrap();
    assert_eq!(active["confirmed"], serde_json::json!(100_000_000), "got: {active}");
}

#[tokio::test]
async fn read_names_honors_the_requested_profile_over_the_active_one() {
    let app = app_with(seeded()); // active = A

    let names_b = read_names(app.state(), Some("B".into())).await.unwrap();
    let b: Vec<&str> = names_b
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(b, vec!["bravo"], "B's names only, not A's");

    let names_a = read_names(app.state(), Some("A".into())).await.unwrap();
    let a: Vec<&str> = names_a
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(a, vec!["alpha"]);
}
