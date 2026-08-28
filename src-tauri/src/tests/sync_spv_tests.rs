//! Coverage tests for `commands/sync_spv.rs` — the SPV-mode sync step.
//!
//! SPV mode is read-only: sync_spv_step confirms the SPV node is reachable
//! (via `getblockchaininfo`), does a non-blocking explorer health probe, and
//! advances the sync cursor. It is a thin adapter — these tests drive the
//! REAL `sync_spv_step` against a `mockito` node RPC + explorer to exercise
//! every branch (open_conn Err, node reachable/unreachable, explorer
//! reachable/unreachable, set_sync_cursor success/failure).
//!
//! Reuses the file-backed-DB pattern from `sync_race_tests.rs`.

use crate::commands::sync_spv::sync_spv_step;
use crate::db;

const PROFILE: &str = "syncspv_a";

/// A migrated, file-backed DB with one profile. Node/explorer settings are
/// left unset so individual tests can point them at their own mock server (or
/// leave them unset to fail fast against an unroutable default).
fn seeded_db() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("namehold_sync_spv_test_{pid}_{n}.db"));
    let _ = std::fs::remove_file(&path);

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "A",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKEA",
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    drop(conn);
    path
}

struct TempDbGuard(std::path::PathBuf);
impl Drop for TempDbGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("db-shm"));
    }
}

fn set_setting(path: &std::path::Path, key: &str, val: &str) {
    let conn = rusqlite::Connection::open(path).unwrap();
    db::queries::set_setting(&conn, key, val).unwrap();
}

// ---------------------------------------------------------------------------
// open_conn Err path
// ---------------------------------------------------------------------------

/// If `db_path` cannot be opened (parent directory does not exist), the very
/// first `open_conn` inside `sync_spv_step` returns `Err` and the whole step
/// short-circuits with `false` — before any node RPC or explorer traffic.
#[tokio::test]
async fn open_conn_error_returns_false() {
    // A path whose parent directory does not exist -> open_conn fails.
    let bogus = "/nonexistent/definitely/not/there/namehold.db";
    let result = sync_spv_step(bogus, PROFILE).await;
    assert!(
        !result,
        "sync_spv_step must return false when the DB cannot be opened"
    );
}

// ---------------------------------------------------------------------------
// SPV node unreachable
// ---------------------------------------------------------------------------

/// With settings loaded successfully but the SPV node pointing at an
/// unroutable URL, `get_blockchain_info` errors and the step returns `false`
/// without touching the explorer or the sync cursor.
#[tokio::test]
async fn spv_node_unreachable_returns_false() {
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());
    // Deliberately unroutable — connection refused fires fast.
    set_setting(&path, "node_rpc_url", "http://127.0.0.1:1");
    set_setting(&path, "explorer_api_url", "http://127.0.0.1:1");

    let result = sync_spv_step(path.to_str().unwrap(), PROFILE).await;
    assert!(
        !result,
        "sync_spv_step must return false when the SPV node is unreachable"
    );

    // Cursor must NOT have advanced — the function bailed before that block.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let height =
        crate::noncustodial::sync::get_sync_height(&conn, PROFILE).expect("read sync height");
    assert_eq!(height, 0, "cursor must remain 0 when SPV node is unreachable");
}

// ---------------------------------------------------------------------------
// Happy path: node reachable + explorer healthy -> cursor advances, true
// ---------------------------------------------------------------------------

#[tokio::test]
async fn happy_path_advances_cursor_and_returns_true() {
    let mut server = mockito::Server::new_async().await;
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());

    // Node RPC POST / — getblockchaininfo returns height 1234.
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"chain":"mainnet","blocks":1234,"headers":1234,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .create_async()
        .await;
    // Explorer health probe: GET /api/txs?limit=1 -> 200.
    let _health = server
        .mock("GET", mockito::Matcher::Regex("^/api/txs".into()))
        .with_body(r#"{"result":[]}"#)
        .create_async()
        .await;

    // Both node RPC and explorer share the same mock server — the paths don't
    // collide (POST / vs GET /api/txs).
    set_setting(&path, "node_rpc_url", &server.url());
    set_setting(&path, "explorer_api_url", &server.url());

    let result = sync_spv_step(path.to_str().unwrap(), PROFILE).await;
    assert!(result, "sync_spv_step should succeed with node + explorer up");

    let conn = rusqlite::Connection::open(&path).unwrap();
    let height =
        crate::noncustodial::sync::get_sync_height(&conn, PROFILE).expect("read sync height");
    assert_eq!(height, 1234, "cursor must advance to the reported height");
}

// ---------------------------------------------------------------------------
// Node reachable but explorer unhealthy -> still returns true, cursor advances
// ---------------------------------------------------------------------------

/// The explorer health probe is intentionally non-blocking: an unreachable
/// explorer must log a warning but NOT fail the sync step, because
/// repair/discover handle their own request failures. The cursor still
/// advances and the step returns `true`.
#[tokio::test]
async fn explorer_unreachable_still_returns_true_and_advances_cursor() {
    let mut server = mockito::Server::new_async().await;
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());

    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"chain":"mainnet","blocks":42,"headers":42,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .create_async()
        .await;

    set_setting(&path, "node_rpc_url", &server.url());
    // Explorer at an unroutable URL — health() returns Err (transport error).
    set_setting(&path, "explorer_api_url", "http://127.0.0.1:1");

    let result = sync_spv_step(path.to_str().unwrap(), PROFILE).await;
    assert!(
        result,
        "sync_spv_step must return true even when the explorer health probe fails"
    );

    let conn = rusqlite::Connection::open(&path).unwrap();
    let height =
        crate::noncustodial::sync::get_sync_height(&conn, PROFILE).expect("read sync height");
    assert_eq!(
        height, 42,
        "cursor must still advance when the explorer is down"
    );
}

// ---------------------------------------------------------------------------
// set_sync_cursor error path: unknown profile_id -> FK constraint fails.
// The function still returns true (cursor-update failure is logged, not fatal).
// ---------------------------------------------------------------------------

/// If `set_sync_cursor` fails (e.g. the profile_id does not exist and the
/// FK constraint rejects the INSERT), `sync_spv_step` logs the error and
/// still returns `true` — reaching the SPV node and probing the explorer
/// already succeeded, and a cursor-write hiccup is not a sync-level failure.
#[tokio::test]
async fn set_sync_cursor_error_still_returns_true() {
    let mut server = mockito::Server::new_async().await;
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());

    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(
            r#"{"result":{"chain":"mainnet","blocks":7,"headers":7,"verificationprogress":1.0},"error":null,"id":1}"#,
        )
        .create_async()
        .await;
    let _health = server
        .mock("GET", mockito::Matcher::Regex("^/api/txs".into()))
        .with_body(r#"{"result":[]}"#)
        .create_async()
        .await;

    set_setting(&path, "node_rpc_url", &server.url());
    set_setting(&path, "explorer_api_url", &server.url());

    // Profile id that does not exist in `wallet_profiles` — the FK on
    // `sync_cursors.wallet_profile_id` will reject the INSERT.
    let result = sync_spv_step(path.to_str().unwrap(), "no_such_profile").await;
    assert!(
        result,
        "sync_spv_step returns true even if the cursor update fails"
    );
}

// ---------------------------------------------------------------------------
// NOTES on lines that are NOT unit-testable in isolation.
// ---------------------------------------------------------------------------
//
// * The `queries::get_settings(&conn) => Err(_)` arm (get_settings failure)
//   is a defensive guard against a corrupt/unmigrated settings row. Reaching
//   it from a unit test would require simulating a rusqlite failure between a
//   successful `open_conn` and a failing `get_settings`, which the queries
//   layer exposes no seam for. `queries::get_settings` has its own unit tests,
//   so leaving this branch uncovered here is intentional and low-risk.
//
// * The SECOND `open_conn` Err arm (the re-open after the RPC call succeeds)
//   can only trip if the DB file is deleted mid-run between the first and
//   second `open_conn`. That is a TOCTOU race no unit test can reproduce
//   deterministically without extra file-system hooks. The first `open_conn`
//   Err arm IS exercised by `open_conn_error_returns_false` above, giving
//   identical structural coverage of the same call.
