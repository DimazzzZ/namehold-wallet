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
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
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
        &conn,
        PROFILE,
        "Disc",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
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
        .with_body(
            r#"{"name":"mine","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#,
        )
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
    assert_eq!(
        res["discovered"].as_u64(),
        Some(1),
        "exactly one owned name"
    );
    let names: Vec<&str> = res["names"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["mine"],
        "owns 'mine', excludes transferred-away 'gone'"
    );

    // read_names serves ONLY owned names — the inventory-only 'notmine' must NOT
    // appear, and the transferred-away 'gone' must NOT appear.
    let listed = read_names(app.state(), None).await.expect("read_names");
    let arr = listed.as_array().expect("array");
    let listed_names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert_eq!(
        listed_names,
        vec!["mine"],
        "Owned Names excludes inventory + transferred-away"
    );
    assert_eq!(arr[0]["state"].as_str(), Some("CLOSED"));
    assert_eq!(arr[0]["renewal"].as_i64(), Some(200));
}

// ---------------------------------------------------------------------------
// resolve_owner_via_history — the shared ownership resolver extracted from
// discover_owned_names (Task 2). Exercises it directly against a mocked
// explorer, independent of the full discovery crawl.
// ---------------------------------------------------------------------------

mod resolve_owner_via_history_tests {
    use crate::commands::sync::resolve_owner_via_history;
    use crate::providers::hnsfans::HnsFansClient;
    use std::collections::HashSet;

    const MINE: &str = "hs1qmineaddr0000000000000000000000000000";
    const OTHER: &str = "hs1qotheraddr000000000000000000000000000";

    #[tokio::test]
    async fn owner_output_address_in_wallet_set_resolves_owned_true() {
        let mut server = mockito::Server::new_async().await;
        let _hist = server
            .mock("GET", "/api/names/mine/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":2}]}"#)
            .create_async()
            .await;
        let _tx = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"address":"{OTHER}","value":0}},{{"action":"NONE","address":"{OTHER}","value":5}},{{"action":"FINALIZE","name":"mine","address":"{MINE}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let client = HnsFansClient::new(&server.url());
        let addr_set: HashSet<String> = [MINE.to_string()].into_iter().collect();

        let resolution = resolve_owner_via_history(&client, "mine", &addr_set)
            .await
            .expect("resolver call ok")
            .expect("some resolution");

        assert_eq!(resolution.owner_txid, "txA");
        assert_eq!(resolution.owner_vout, 2);
        assert_eq!(resolution.owner_address, MINE);
        assert!(resolution.owned_by_wallet, "address is in addr_set");
    }

    #[tokio::test]
    async fn owner_output_address_not_in_wallet_set_resolves_owned_false() {
        let mut server = mockito::Server::new_async().await;
        let _hist = server
            .mock("GET", "/api/names/notmine/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":0}]}"#)
            .create_async()
            .await;
        let _tx = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"notmine","address":"{OTHER}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let client = HnsFansClient::new(&server.url());
        let addr_set: HashSet<String> = [MINE.to_string()].into_iter().collect();

        let resolution = resolve_owner_via_history(&client, "notmine", &addr_set)
            .await
            .expect("resolver call ok")
            .expect("some resolution");

        assert_eq!(resolution.owner_address, OTHER);
        assert!(!resolution.owned_by_wallet, "address is NOT in addr_set");
    }

    #[tokio::test]
    async fn no_history_resolves_none() {
        let mut server = mockito::Server::new_async().await;
        let _hist = server
            .mock("GET", "/api/names/nohistory/history")
            .with_body(r#"{"result":[]}"#)
            .create_async()
            .await;

        let client = HnsFansClient::new(&server.url());
        let addr_set: HashSet<String> = [MINE.to_string()].into_iter().collect();

        let resolution = resolve_owner_via_history(&client, "nohistory", &addr_set)
            .await
            .expect("resolver call ok");

        assert!(resolution.is_none(), "no history means no resolution");
    }

    #[tokio::test]
    async fn no_output_at_owner_vout_resolves_none() {
        let mut server = mockito::Server::new_async().await;
        // History points at index 5, but the tx only has outputs at 0 and 1.
        let _hist = server
            .mock("GET", "/api/names/mismatched/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":5}]}"#)
            .create_async()
            .await;
        let _tx = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"address":"{OTHER}","value":0}},{{"action":"FINALIZE","name":"mismatched","address":"{MINE}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let client = HnsFansClient::new(&server.url());
        let addr_set: HashSet<String> = [MINE.to_string()].into_iter().collect();

        let resolution = resolve_owner_via_history(&client, "mismatched", &addr_set)
            .await
            .expect("resolver call ok");

        assert!(
            resolution.is_none(),
            "no output at the recorded owner_vout means no resolution"
        );
    }
}

// ---------------------------------------------------------------------------
// discover_step (sync.rs) — the background-sync counterpart to
// `discover_owned_names`. Task 4: phase 2 used to gate ownership on a dead
// check (`info.owner.as_ref().map(...)` — `owner.hash` is a txid, never an
// address) and made a redundant standalone `get_name_current_owner` call.
// These tests drive `discover_step` directly (it's `pub(crate)`) against a
// file-backed DB (the function opens several independent connections, so an
// in-memory DB won't share state across them) and a mocked HNSFans explorer.
// ---------------------------------------------------------------------------

mod discover_step_tests {
    use crate::commands::sync::{discover_step, SyncStatus};
    use crate::db;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const PROFILE: &str = "discstep1";
    const MINE: &str = "hs1qmineaddr0000000000000000000000000000";
    const OTHER: &str = "hs1qotheraddr000000000000000000000000000";

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A fresh file-backed, migrated DB path seeded with one profile owning
    /// derived address `MINE`, and the explorer URL pointed at the mock
    /// server. Returns the path (as a String, the shape `discover_step`
    /// takes) plus a guard that deletes the file (+ WAL/SHM sidecars) on drop.
    struct TempDb {
        path: std::path::PathBuf,
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
            let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
        }
    }

    fn seeded_db(explorer_url: &str) -> TempDb {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        // nextest runs each test in its own process, so the per-process COUNTER
        // resets to 0 every time — two processes would otherwise collide on the
        // same temp file and one hits "attempt to write a readonly database".
        // Mix in the PID to guarantee a unique path per process.
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("namehold_discover_step_test_{pid}_{n}.db"));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        db::migrations::run(&conn).unwrap();
        db::queries::insert_wallet_profile(
            &conn,
            PROFILE,
            "DiscStep",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKE",
            0,
            false,
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
        drop(conn);
        TempDb { path }
    }

    #[tokio::test]
    async fn candidate_owned_by_wallet_gets_upserted_with_owner_address() {
        let mut server = mockito::Server::new_async().await;

        // Phase 1 crawl: our address touched one tx, which pays us via a
        // FINALIZE output naming "mine" — a candidate.
        let _txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"{"limit":25,"offset":0,"total":1,"result":[{"hash":"txA"}]}"#)
            .create_async()
            .await;
        let _tx_a = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"mine","address":"{MINE}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        // Phase 2: name-info lookup, then history-based owner resolution —
        // owner tx output pays OUR address.
        let _name = server
            .mock("GET", "/api/names/mine")
            .with_body(
                r#"{"name":"mine","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#,
            )
            .create_async()
            .await;
        let _hist = server
            .mock("GET", "/api/names/mine/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txB","index":2}]}"#)
            .create_async()
            .await;
        let _owner_tx = server
            .mock("GET", "/api/txs/txB")
            .with_body(format!(
                r#"{{"outputs":[{{"address":"{OTHER}","value":0}},{{"action":"NONE","address":"{OTHER}","value":5}},{{"action":"FINALIZE","name":"mine","address":"{MINE}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let db = seeded_db(&server.url());
        let db_path = db.path.to_str().unwrap().to_string();
        let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

        discover_step(&status, &db_path, PROFILE).await;

        let s = status.lock().await;
        assert_eq!(s.discovered, 1, "one name discovered and upserted");
        drop(s);

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let (owner_address, owner_txid, owner_vout): (Option<String>, Option<String>, Option<i64>) =
            conn.query_row(
                "SELECT owner_address, owner_txid, owner_vout FROM tracked_name_states
                 WHERE wallet_profile_id = ?1 AND name = 'mine'",
                rusqlite::params![PROFILE],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("tracked row exists for owned candidate");
        assert_eq!(
            owner_address.as_deref(),
            Some(MINE),
            "owner_address is our address, resolved via history"
        );
        assert_eq!(
            owner_txid.as_deref(),
            Some("txB"),
            "owner_txid comes from the resolver, not the redundant call"
        );
        assert_eq!(owner_vout, Some(2));
    }

    #[tokio::test]
    async fn candidate_owned_by_foreign_address_not_upserted() {
        let mut server = mockito::Server::new_async().await;

        // Phase 1 crawl: our address touched a tx with a TRANSFER output
        // naming "notmine" that pays us — a candidate (we received it), but
        // its CURRENT owner (per history) is a foreign address.
        let _txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"{"limit":25,"offset":0,"total":1,"result":[{"hash":"txA"}]}"#)
            .create_async()
            .await;
        let _tx_a = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"TRANSFER","name":"notmine","address":"{MINE}","value":1}}]}}"#
            ))
            .create_async()
            .await;

        let _name = server
            .mock("GET", "/api/names/notmine")
            .with_body(r#"{"name":"notmine","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
            .create_async()
            .await;
        let _hist = server
            .mock("GET", "/api/names/notmine/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txC","index":0}]}"#)
            .create_async()
            .await;
        let _owner_tx = server
            .mock("GET", "/api/txs/txC")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"notmine","address":"{OTHER}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let db = seeded_db(&server.url());
        let db_path = db.path.to_str().unwrap().to_string();
        let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

        discover_step(&status, &db_path, PROFILE).await;

        let s = status.lock().await;
        assert_eq!(
            s.discovered, 0,
            "foreign-owned candidate is not counted as discovered"
        );
        drop(s);

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_name_states
                 WHERE wallet_profile_id = ?1 AND name = 'notmine'",
                rusqlite::params![PROFILE],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "no tracked row for a candidate not currently owned by us"
        );
    }

    // -----------------------------------------------------------------------
    // Task B: phase 2 has NO budget cap — more than the old 20-name limit are
    // processed in a SINGLE call, and the "paused (budget)" label is gone.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn phase2_processes_more_than_twenty_candidates_in_one_call() {
        let mut server = mockito::Server::new_async().await;

        // Phase 1: one tx paying us via 25 distinct FINALIZE outputs → 25
        // candidates (more than the removed budget of 20).
        let outputs: String = (0..25)
            .map(|i| {
                format!(
                    r#"{{"action":"FINALIZE","name":"name{i}","address":"{MINE}","value":400000}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let tx_body = format!(r#"{{"outputs":[{outputs}]}}"#);

        let _txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"{"limit":25,"offset":0,"total":1,"result":[{"hash":"txA"}]}"#)
            .create_async()
            .await;
        let _tx_a = server
            .mock("GET", "/api/txs/txA")
            .with_body(tx_body)
            .create_async()
            .await;

        // Phase 2: every name resolves to us (history → txB[0] pays MINE), so
        // each of the 25 candidates is discovered — proving no budget cap.
        let _name = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/api/names/[^/]+$".to_string()),
            )
            .with_body(
                r#"{"name":"n","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#,
            )
            .expect_at_least(21)
            .create_async()
            .await;
        let _hist = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/api/names/[^/]+/history$".to_string()),
            )
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txB","index":0}]}"#)
            .create_async()
            .await;
        let _owner_tx = server
            .mock("GET", "/api/txs/txB")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"n","address":"{MINE}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let db = seeded_db(&server.url());
        let db_path = db.path.to_str().unwrap().to_string();
        let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

        discover_step(&status, &db_path, PROFILE).await;

        let s = status.lock().await;
        assert_eq!(
            s.discovered, 25,
            "all 25 candidates processed in one call (no budget cap)"
        );
        assert!(
            !s.progress_label.contains("budget"),
            "the old 'paused (budget)' label must be gone, got {:?}",
            s.progress_label
        );
    }

    // -----------------------------------------------------------------------
    // Task B: the "recently checked" memo. A candidate whose `assets` row has a
    // fresh `last_synced_at` is skipped — the explorer is NOT hit for it. Proven
    // via mockito `.expect(0)` on that name's name/history routes.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn memo_skips_recently_synced_candidate() {
        let mut server = mockito::Server::new_async().await;

        // Phase 1: one tx names TWO candidates paying us — "fresh" and "stale".
        let _txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"{"limit":25,"offset":0,"total":1,"result":[{"hash":"txA"}]}"#)
            .create_async()
            .await;
        let _tx_a = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"fresh","address":"{MINE}","value":1}},{{"action":"FINALIZE","name":"stale","address":"{MINE}","value":1}}]}}"#
            ))
            .create_async()
            .await;

        // "fresh" is memoized (recent `last_synced_at`): its routes must NEVER
        // be hit. `.expect(0)` is verified when the mock guard drops.
        let fresh_name = server
            .mock("GET", "/api/names/fresh")
            .expect(0)
            .create_async()
            .await;
        let fresh_hist = server
            .mock("GET", "/api/names/fresh/history")
            .expect(0)
            .create_async()
            .await;

        // "stale" has no fresh memo → it IS checked (empty history → not owned).
        let _stale_name = server
            .mock("GET", "/api/names/stale")
            .with_body(
                r#"{"name":"stale","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#,
            )
            .expect_at_least(1)
            .create_async()
            .await;
        let _stale_hist = server
            .mock("GET", "/api/names/stale/history")
            .with_body(r#"{"result":[]}"#)
            .expect_at_least(1)
            .create_async()
            .await;

        let db = seeded_db(&server.url());
        // Seed a FRESH inventory row for "fresh" so it lands in the memo set.
        {
            let conn = rusqlite::Connection::open(&db.path).unwrap();
            conn.execute(
                "INSERT INTO assets (tld, status, last_synced_at) VALUES ('fresh', 'not_started', datetime('now'))",
                [],
            )
            .unwrap();
        }
        let db_path = db.path.to_str().unwrap().to_string();
        let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

        discover_step(&status, &db_path, PROFILE).await;

        let s = status.lock().await;
        assert_eq!(s.discovered, 0, "neither candidate is owned by the wallet");
        drop(s);

        // The memoized name's explorer routes were never called.
        fresh_name.assert_async().await;
        fresh_hist.assert_async().await;
    }

    // -----------------------------------------------------------------------
    // Task B: cancellation. A pre-set `cancel_requested` makes discover_step
    // bail before any explorer crawl — proven by `.expect(0)` on /api/txs.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn cancellation_before_run_bails_out_without_explorer_calls() {
        let mut server = mockito::Server::new_async().await;

        let txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .expect(0)
            .create_async()
            .await;

        let db = seeded_db(&server.url());
        let db_path = db.path.to_str().unwrap().to_string();
        let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
        {
            let mut s = status.lock().await;
            s.cancel_requested = true;
        }

        discover_step(&status, &db_path, PROFILE).await;

        let s = status.lock().await;
        assert_eq!(s.progress_label, "Sync cancelled");
        assert_eq!(s.discovered, 0, "cancelled run discovered nothing");
        drop(s);

        // No crawl happened.
        txs.assert_async().await;
    }
}

#[tokio::test]
async fn discovery_no_addresses_is_empty() {
    // A profile with no derived addresses discovers nothing (no crawl).
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Disc",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    let app = app_with(conn);
    let res = discover_owned_names(app.state()).await.expect("discover");
    assert_eq!(res["discovered"].as_u64(), Some(0));
}
