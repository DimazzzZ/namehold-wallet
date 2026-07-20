//! Integration tests for `repair_owned_names` (read.rs).
//!
//! Drives the REAL command against a `mockito` explorer. Proves the ownership
//! check now goes through `resolve_owner_via_history` (owner tx output ADDRESS),
//! that a confirmed-owned inventory name gets a `tracked_name_states` row with
//! `owner_address` set AND its `assets.status` advanced to `finalized_owned`,
//! and that a name owned by a FOREIGN address creates no tracked row but still
//! stamps `assets.last_synced_at` so repeated runs converge.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::read::repair_owned_names;
use crate::db;
use crate::AppState;

const PROFILE: &str = "rep1";
const MINE: &str = "hs1qmineaddr0000000000000000000000000000";
const OTHER: &str = "hs1qotheraddr000000000000000000000000000";

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// Migrated in-memory DB: one active profile owning derived address `MINE`,
/// explorer pointed at the mock server.
fn seeded_conn(explorer_url: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn, PROFILE, "Rep", "mnemonic_hot", "mainnet", "xpubFAKE", 0, false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", explorer_url).unwrap();
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, '0014aa', '02aa')",
        rusqlite::params![PROFILE, MINE],
    )
    .unwrap();
    conn
}

#[tokio::test]
async fn inventory_name_owned_by_wallet_gets_tracked_row_and_finalized_status() {
    let mut server = mockito::Server::new_async().await;

    // Name detail (get_name_info_optional).
    let _name = server
        .mock("GET", "/api/names/mine")
        .with_body(r#"{"name":"mine","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
        .create_async()
        .await;
    // History → current owner outpoint txA[2].
    let _hist = server
        .mock("GET", "/api/names/mine/history")
        .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":2}]}"#)
        .create_async()
        .await;
    // Owner tx → output at index 2 pays OUR address.
    let _tx = server
        .mock("GET", "/api/txs/txA")
        .with_body(format!(
            r#"{{"outputs":[{{"address":"{OTHER}","value":0}},{{"action":"NONE","address":"{OTHER}","value":5}},{{"action":"FINALIZE","name":"mine","address":"{MINE}","value":400000}}]}}"#
        ))
        .create_async()
        .await;

    // Inventory-only name: an `assets` row, NO `tracked_name_states` row.
    let conn = seeded_conn(&server.url());
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('mine','not_started')",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    let res = repair_owned_names(app.state()).await.expect("repair");
    assert_eq!(res["repaired"].as_u64(), Some(1), "one name repaired");

    // A tracked row now exists with owner_address set to our address.
    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let (owner_address, state): (Option<String>, String) = db
        .query_row(
            "SELECT owner_address, state FROM tracked_name_states
             WHERE wallet_profile_id = ?1 AND name = 'mine'",
            rusqlite::params![PROFILE],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("tracked row exists");
    assert_eq!(owner_address.as_deref(), Some(MINE), "owner_address is our address");
    assert_eq!(state, "CLOSED");

    // The inventory row advanced to finalized_owned + got a sync timestamp.
    let (status, synced): (String, Option<String>) = db
        .query_row(
            "SELECT status, last_synced_at FROM assets WHERE tld = 'mine'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "finalized_owned");
    assert!(synced.is_some(), "last_synced_at stamped");
}

#[tokio::test]
async fn inventory_name_owned_by_foreign_address_touches_but_creates_no_tracked_row() {
    let mut server = mockito::Server::new_async().await;

    let _name = server
        .mock("GET", "/api/names/foreign")
        .with_body(r#"{"name":"foreign","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
        .create_async()
        .await;
    let _hist = server
        .mock("GET", "/api/names/foreign/history")
        .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":0}]}"#)
        .create_async()
        .await;
    // Owner tx → output pays a FOREIGN address (not ours).
    let _tx = server
        .mock("GET", "/api/txs/txA")
        .with_body(format!(
            r#"{{"outputs":[{{"action":"FINALIZE","name":"foreign","address":"{OTHER}","value":400000}}]}}"#
        ))
        .create_async()
        .await;

    let conn = seeded_conn(&server.url());
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('foreign','not_started')",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    let res = repair_owned_names(app.state()).await.expect("repair");
    assert_eq!(res["repaired"].as_u64(), Some(0), "not owned → nothing repaired");

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    // No tracked row was created for the foreign-owned name.
    let tracked_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM tracked_name_states
             WHERE wallet_profile_id = ?1 AND name = 'foreign'",
            rusqlite::params![PROFILE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tracked_count, 0, "no tracked row for foreign-owned name");

    // But the check was recorded: last_synced_at is set, status unchanged.
    let (status, synced): (String, Option<String>) = db
        .query_row(
            "SELECT status, last_synced_at FROM assets WHERE tld = 'foreign'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "not_started", "status unchanged when not owned");
    assert!(synced.is_some(), "last_synced_at stamped so repeated runs converge");
}
