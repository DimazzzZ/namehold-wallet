//! Integration test for node-free owned-name discovery (`discover_owned_names`).
//!
//! Drives the REAL command against a `mockito` explorer that mimics HNSFans:
//! per-address tx list, per-tx detail (outputs flattened with action+name+
//! address), name history, and name detail. Proves the wallet discovers a name
//! it currently owns and EXCLUDES one it received but later transferred away.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::read::{discover_owned_names, read_names};
use crate::db;
use crate::AppState;

const PROFILE: &str = "disc1";
const MINE: &str = "hs1qmineaddr0000000000000000000000000000";
const OTHER: &str = "hs1qotheraddr000000000000000000000000000";

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

/// Migrated in-memory DB with one profile owning a single derived address
/// `MINE`, and the explorer URL pointed at the mock server.
fn seeded_conn(explorer_url: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn, PROFILE, "Disc", "mnemonic_hot", "mainnet", "xpubFAKE", 0, false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", explorer_url).unwrap();
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, '0014aa', '02aa')",
        params![PROFILE, MINE],
    )
    .unwrap();
    conn
}

#[tokio::test]
async fn discovers_owned_name_and_excludes_transferred_away() {
    let mut server = mockito::Server::new_async().await;

    // 1. The address's tx list: it touched txA (owns "mine") and txB (received
    //    "gone", later transferred away).
    let _txs = server
        .mock("GET", "/api/txs")
        .match_query(mockito::Matcher::Any)
        .with_body(r#"{"limit":25,"offset":0,"total":2,"result":[{"hash":"txA"},{"hash":"txB"}]}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    // 2. Per-tx detail. txA: a FINALIZE for "mine" paying our address at index 2.
    let _tx_a = server
        .mock("GET", "/api/txs/txA")
        .with_body(format!(
            r#"{{"outputs":[{{"address":"{OTHER}","value":0}},{{"action":"NONE","address":"{MINE}","value":5}},{{"action":"FINALIZE","name":"mine","address":"{MINE}","value":400000}}]}}"#
        ))
        .expect_at_least(1)
        .create_async()
        .await;
    // txB: a TRANSFER output for "gone" paying our address (so it's a candidate).
    let _tx_b = server
        .mock("GET", "/api/txs/txB")
        .with_body(format!(
            r#"{{"outputs":[{{"action":"TRANSFER","name":"gone","address":"{MINE}","value":1}}]}}"#
        ))
        .create_async()
        .await;
    // txC: the later tx where "gone" was transferred AWAY to someone else.
    let _tx_c = server
        .mock("GET", "/api/txs/txC")
        .with_body(format!(
            r#"{{"outputs":[{{"action":"FINALIZE","name":"gone","address":"{OTHER}","value":1}}]}}"#
        ))
        .create_async()
        .await;

    // 3. History: "mine" currently lives at txA[2] (ours); "gone" at txC[0] (not).
    let _hist_mine = server
        .mock("GET", "/api/names/mine/history")
        .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":2}]}"#)
        .create_async()
        .await;
    let _hist_gone = server
        .mock("GET", "/api/names/gone/history")
        .with_body(r#"{"result":[{"action":"Finalize","txid":"txC","index":0}]}"#)
        .create_async()
        .await;

    // 4. Name detail for the confirmed-owned name.
    let _name_mine = server
        .mock("GET", "/api/names/mine")
        .with_body(r#"{"name":"mine","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
        .create_async()
        .await;

    // Seed the migration INVENTORY with a name the wallet does NOT own (the
    // regression: these used to be unioned into "Owned Names").
    let conn = seeded_conn(&server.url());
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('notmine', 'not_started')",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    // Run discovery.
    let res = discover_owned_names(app.state()).await.expect("discover");
    assert_eq!(res["discovered"].as_u64(), Some(1), "exactly one owned name");
    let names: Vec<&str> = res["names"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(names, vec!["mine"], "owns 'mine', excludes transferred-away 'gone'");

    // read_names serves ONLY owned names — the inventory-only 'notmine' must NOT
    // appear, and the transferred-away 'gone' must NOT appear.
    let listed = read_names(app.state(), None).await.expect("read_names");
    let arr = listed.as_array().expect("array");
    let listed_names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert_eq!(listed_names, vec!["mine"], "Owned Names excludes inventory + transferred-away");
    assert_eq!(arr[0]["state"].as_str(), Some("CLOSED"));
    assert_eq!(arr[0]["renewal"].as_i64(), Some(200));
}

#[tokio::test]
async fn discovery_no_addresses_is_empty() {
    // A profile with no derived addresses discovers nothing (no crawl).
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn, PROFILE, "Disc", "mnemonic_hot", "mainnet", "xpubFAKE", 0, false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    let app = app_with(conn);
    let res = discover_owned_names(app.state()).await.expect("discover");
    assert_eq!(res["discovered"].as_u64(), Some(0));
}
