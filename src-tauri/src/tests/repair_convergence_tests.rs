//! Integration tests for the `repair_step` convergence loop (sync.rs, Task A).
//!
//! `repair_step_windowed` pages through the whole candidate backlog in
//! fixed-size windows until it converges, so ONE Sync click finishes the job.
//! These tests drive it directly (it's `pub(crate)`) against a file-backed DB
//! (the function opens several independent connections per call, so an
//! in-memory DB won't share state across them) and a mocked HNSFans explorer.
//!
//! Covered:
//! * convergence across MULTIPLE windows — one call stamps `last_synced_at` on
//!   every candidate even when there are more of them than a single window;
//! * all-transport-errors — the loop aborts (never hangs) and records an error;
//! * cancellation — a pre-set `cancel_requested` makes the step bail out fast
//!   without touching the explorer or the DB;
//! * tracked-only names (no `assets` row) — these can never be stamped via
//!   `touch_asset_synced`/`mark_asset_finalized_owned` (both are no-op UPDATEs
//!   against `assets` when the tld isn't there), so the in-run `attempted`
//!   `HashSet` is the ONLY thing that stops them being re-fetched by
//!   `list_repair_candidates` forever. Without it this case hangs the
//!   background sync thread.

use crate::commands::sync::{repair_step_windowed, SyncStatus};
use crate::db;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const PROFILE: &str = "repconv1";
const MINE: &str = "hs1qmineaddr0000000000000000000000000000";

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh file-backed, migrated DB seeded with one profile owning derived
/// address `MINE` and the explorer URL pointed at the mock server. The guard
/// deletes the file (+ WAL/SHM sidecars) on drop.
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
    let path = std::env::temp_dir().join(format!("namehold_repair_conv_test_{n}.db"));
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "RepConv",
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

/// Seed `n` inventory `assets` rows (`name0..name{n-1}`), all `not_started`.
fn seed_assets(db: &TempDb, n: usize) {
    let conn = rusqlite::Connection::open(&db.path).unwrap();
    for i in 0..n {
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES (?1, 'not_started')",
            rusqlite::params![format!("name{i}")],
        )
        .unwrap();
    }
}

/// Seed `n` names DIRECTLY into `tracked_name_states` for [`PROFILE`], with NO
/// corresponding `assets` row — the "tracked-only" case whose only
/// convergence guarantee is the in-run `attempted` set (see module docs).
fn seed_tracked_only(db: &TempDb, n: usize) {
    let conn = rusqlite::Connection::open(&db.path).unwrap();
    for i in 0..n {
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state)
             VALUES (?1, ?2, 'hh', 'CLOSED')",
            rusqlite::params![PROFILE, format!("trackedonly{i}")],
        )
        .unwrap();
    }
}

fn count_synced(db: &TempDb) -> i64 {
    let conn = rusqlite::Connection::open(&db.path).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM assets WHERE last_synced_at IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// 1. Convergence across MULTIPLE windows.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn converges_across_multiple_windows_in_one_call() {
    let mut server = mockito::Server::new_async().await;

    // Every name-info lookup returns a valid (not-owned-relevant) body.
    let _name = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+$".to_string()),
        )
        .with_body(r#"{"name":"n","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
        .expect_at_least(5)
        .create_async()
        .await;
    // Every name's history is empty => resolver returns None => "not owned"
    // => touch_asset_synced stamps last_synced_at.
    let _hist = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+/history$".to_string()),
        )
        .with_body(r#"{"result":[]}"#)
        .expect_at_least(5)
        .create_async()
        .await;

    let db = seeded_db(&server.url());
    seed_assets(&db, 5);
    let db_path = db.path.to_str().unwrap().to_string();
    let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

    // Window of 2 forces at least 3 windows to cover 5 candidates.
    repair_step_windowed(&status, &db_path, PROFILE, 2).await;

    // Every candidate was checked and stamped in a SINGLE call.
    assert_eq!(count_synced(&db), 5, "all 5 candidates stamped in one run");

    let s = status.lock().await;
    assert_eq!(s.repair_candidates, 5, "stable denominator = total backlog");
    assert_eq!(s.repair_remaining, 0, "remaining converged to 0");
    assert_eq!(s.repaired, 0, "none owned by wallet (empty histories)");
    assert!(s.errors.is_empty(), "clean convergence, no errors");
}

// ---------------------------------------------------------------------------
// 2. 100% transport errors → aborts (no hang) with an error message.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_transport_errors_abort_with_message() {
    let mut server = mockito::Server::new_async().await;

    // Every name-info lookup 500s → a transport error for every candidate.
    let _name = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+$".to_string()),
        )
        .with_status(500)
        .expect_at_least(1)
        .create_async()
        .await;

    let db = seeded_db(&server.url());
    // More than SYNC_MAX_CONSECUTIVE_ERRORS (5) so the consecutive-error abort
    // triggers; the whole run must still terminate.
    seed_assets(&db, 6);
    let db_path = db.path.to_str().unwrap().to_string();
    let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

    // Must return (not hang); tokio test would otherwise time out.
    repair_step_windowed(&status, &db_path, PROFILE, 150).await;

    // Nothing was stamped — every check errored.
    assert_eq!(
        count_synced(&db),
        0,
        "no candidate stamped when explorer is down"
    );

    let s = status.lock().await;
    assert!(!s.errors.is_empty(), "an error message was recorded");
    assert!(
        s.errors.iter().any(|e| e.contains("Explorer unavailable")),
        "error message mentions explorer unavailability, got {:?}",
        s.errors
    );
}

// ---------------------------------------------------------------------------
// 2b. Persistent explorer FORMAT errors (Task 11 / S1) → aborts with the
//     distinct "degraded" message, not the generic "unavailable" one — and
//     crucially NOT a silent "0 owned" success.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_format_errors_abort_with_degraded_message() {
    let mut server = mockito::Server::new_async().await;

    // Every name-info lookup answers 200 OK but with a shape the client
    // doesn't recognize (no `name` field) — the explorer's contract drifted.
    let _name = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+$".to_string()),
        )
        .with_status(200)
        .with_body(r#"{"domain":"drifted","status":"ok"}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let db = seeded_db(&server.url());
    // More than SYNC_MAX_CONSECUTIVE_ERRORS (5) so the consecutive-error abort
    // triggers; the whole run must still terminate.
    seed_assets(&db, 6);
    let db_path = db.path.to_str().unwrap().to_string();
    let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

    repair_step_windowed(&status, &db_path, PROFILE, 150).await;

    // Nothing was stamped — every check errored (never a silent "0 owned").
    assert_eq!(
        count_synced(&db),
        0,
        "no candidate stamped when the explorer is degraded"
    );

    let s = status.lock().await;
    assert!(!s.errors.is_empty(), "an error message was recorded");
    assert!(
        s.errors
            .iter()
            .any(|e| e.contains("degraded") || e.contains("format")),
        "error message names the format-degradation, got {:?}",
        s.errors
    );
    assert!(
        !s.errors.iter().any(|e| e.contains("Explorer unavailable")),
        "format drift must not be reported as generic unavailability, got {:?}",
        s.errors
    );
}

// ---------------------------------------------------------------------------
// 3. Cancellation → bails out fast, touches neither explorer nor DB.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancellation_before_run_bails_out_without_side_effects() {
    let mut server = mockito::Server::new_async().await;

    // If the loop wrongly proceeded it would hit these; expect(0) proves it did not.
    let _name = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+$".to_string()),
        )
        .expect(0)
        .create_async()
        .await;
    let _hist = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+/history$".to_string()),
        )
        .expect(0)
        .create_async()
        .await;

    let db = seeded_db(&server.url());
    seed_assets(&db, 5);
    let db_path = db.path.to_str().unwrap().to_string();

    // Pre-set the cancel flag, as `cancel_full_sync` would.
    let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
    {
        let mut s = status.lock().await;
        s.cancel_requested = true;
    }

    repair_step_windowed(&status, &db_path, PROFILE, 2).await;

    // No candidate was checked/stamped.
    assert_eq!(count_synced(&db), 0, "cancelled run stamped nothing");

    let s = status.lock().await;
    assert_eq!(s.progress_label, "Sync cancelled");
    // Mock `.expect(0)` assertions are verified on the server guards' drop.
}

// ---------------------------------------------------------------------------
// 4. Tracked-only names (no `assets` row) converge across MULTIPLE windows.
//
// This is the load-bearing regression test for the `attempted` HashSet: since
// `touch_asset_synced`/`mark_asset_finalized_owned` are no-op UPDATEs for a
// name that isn't in `assets`, a tracked-only name's `last_synced_at` NEVER
// gets stamped, so `list_repair_candidates` would return it again in every
// single window forever — UNLESS `attempted` filters it out after the first
// try. With window < candidate count, this test forces multiple windows, so
// if `attempted` were ever removed or broken, this call would loop forever
// and hang. `tokio::time::timeout` turns that hang into a fast, readable test
// failure instead of an actual CI/dev-machine hang.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tracked_only_names_converge_across_multiple_windows() {
    let mut server = mockito::Server::new_async().await;

    const N: usize = 5;

    // Every name-info lookup returns a valid body (same shape as test 1).
    let name_mock = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+$".to_string()),
        )
        .with_body(r#"{"name":"n","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
        .expect(N)
        .create_async()
        .await;
    // Empty history => resolver returns None => "not owned" => touch_asset_synced
    // is called, but since these names have no `assets` row it's a no-op — the
    // point being exercised is that the loop still terminates regardless.
    let hist_mock = server
        .mock(
            "GET",
            mockito::Matcher::Regex(r"^/api/names/[^/]+/history$".to_string()),
        )
        .with_body(r#"{"result":[]}"#)
        .expect(N)
        .create_async()
        .await;

    let db = seeded_db(&server.url());
    seed_tracked_only(&db, N);
    let db_path = db.path.to_str().unwrap().to_string();
    let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

    // Window smaller than N forces multiple windows over the same, never-
    // stamped candidates — exactly the scenario that hangs without `attempted`.
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        repair_step_windowed(&status, &db_path, PROFILE, 2),
    )
    .await;
    assert!(
        outcome.is_ok(),
        "repair_step_windowed did not return within 20s — tracked-only names \
         are looping forever (attempted-set regression)"
    );

    // No `assets` rows exist, so `last_synced_at` is (correctly) never stamped
    // for these names — that's the documented limitation `attempted` works
    // around, not a bug. What must hold is that every tracked-only name was
    // actually attempted exactly once each, which the exact mock call counts
    // above (`.expect(N)`) verify, plus honest convergence in `SyncStatus`.
    let s = status.lock().await;
    assert_eq!(
        s.repair_candidates, N as u32,
        "stable denominator = total backlog"
    );
    assert_eq!(
        s.repair_remaining, 0,
        "remaining converged to 0 — all tracked-only names attempted"
    );
    assert!(s.errors.is_empty(), "clean convergence, no errors");
    drop(s);

    // Belt-and-suspenders: assert the mocks' exact call counts explicitly
    // (in addition to the drop-time assertion mockito performs) so a failure
    // here points straight at "attempted" rather than a generic panic.
    name_mock.assert_async().await;
    hist_mock.assert_async().await;
}
