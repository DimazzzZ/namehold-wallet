//! Regression tests for `last_explorer_sync_at` (Task 11 review, Finding 2).
//!
//! `WalletView.tsx`'s "Last successful sync" line used to read only
//! `wallet_profiles.last_synced_at`, which advances exclusively via the
//! node-RPC sync step (`apply_node_sync_batch` -> `update_profile_sync`) —
//! never via the explorer-driven `repair_step_windowed`/`discover_step`. In
//! explorer-only mode (no local node) it stayed NULL forever, even after a
//! fully successful explorer sync.
//!
//! `last_explorer_sync_at` is a separate column, stamped by
//! [`stamp_explorer_sync_if_clean`] — called exactly ONCE from the "Done"
//! block at the end of `start_full_sync`'s background thread, entirely
//! outside the per-name repair/discover loops — and ONLY when the run
//! reached the end with no cancellation and no `SYNC_MAX_CONSECUTIVE_ERRORS`
//! abort (the only thing that ever pushes into `SyncStatus.errors`).
//!
//! These tests drive `stamp_explorer_sync_if_clean` directly (it's
//! `pub(crate)`, same test-seam pattern as `repair_step_windowed` in
//! `repair_convergence_tests.rs`) against a real `SyncStatus` and a
//! file-backed DB — deterministic and fast, with no background thread, no
//! mocked explorer, and no interaction with the `TEST_PANIC_HOOK` seam used
//! by `sync_race_tests.rs` (spawning additional real `start_full_sync`
//! background threads was tried first and made that panic test flaky under
//! `cargo test`'s parallel runner, since `TEST_PANIC_HOOK` is a single
//! process-global flag any concurrently-spawned sync thread consumes —
//! testing the extracted function directly avoids that shared state
//! entirely while still exercising the exact code the "Done" block calls).

use crate::commands::sync::{stamp_explorer_sync_if_clean, SyncStatus};
use crate::db;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

const PROFILE: &str = "expsync1";

static COUNTER: AtomicUsize = AtomicUsize::new(0);

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

fn seeded_db() -> TempDb {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("namehold_expsync_test_{pid}_{n}.db"));
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "ExpSync",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    drop(conn);
    TempDb { path }
}

fn stamped_at(db: &TempDb) -> Option<String> {
    let conn = rusqlite::Connection::open(&db.path).unwrap();
    db::queries::get_wallet_profile(&conn, PROFILE)
        .unwrap()
        .unwrap()
        .last_explorer_sync_at
}

// ---------------------------------------------------------------------------
// 1. Clean status (no cancellation, no errors) -> stamps the column.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn clean_status_stamps_last_explorer_sync_at() {
    let db = seeded_db();
    let db_path = db.path.to_str().unwrap().to_string();
    assert!(stamped_at(&db).is_none(), "must be unset before the run");

    let status = Arc::new(Mutex::new(SyncStatus::default()));
    stamp_explorer_sync_if_clean(&status, &db_path, PROFILE).await;

    assert!(
        stamped_at(&db).is_some(),
        "a clean SyncStatus (no cancel, no errors) must stamp last_explorer_sync_at"
    );
}

// ---------------------------------------------------------------------------
// 2. Non-empty `errors` (exactly what an aborted repair/discover step
//    leaves behind, per `record_error_and_clear_waiting`) -> does NOT stamp,
//    and does not disturb a pre-existing stamp either.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn status_with_errors_does_not_advance_last_explorer_sync_at() {
    let db = seeded_db();
    let db_path = db.path.to_str().unwrap().to_string();

    // Seed a pre-existing stamp so the test proves the aborted run leaves it
    // exactly as-is, not merely "still None".
    {
        let conn = rusqlite::Connection::open(&db.path).unwrap();
        db::queries::stamp_explorer_sync(&conn, PROFILE).unwrap();
    }
    let sentinel = stamped_at(&db).expect("sentinel stamp must be set before the run");

    let status = Arc::new(Mutex::new(SyncStatus::default()));
    {
        let mut s = status.lock().await;
        s.errors
            .push("Explorer degraded — response format changed unexpectedly.".to_string());
    }
    stamp_explorer_sync_if_clean(&status, &db_path, PROFILE).await;

    assert_eq!(
        stamped_at(&db),
        Some(sentinel),
        "a run with a recorded error must not advance last_explorer_sync_at"
    );
}

// ---------------------------------------------------------------------------
// 3. `cancel_requested` (even with `errors` empty) -> does NOT stamp. A
//    user-cancelled run didn't finish repair+discover, so it isn't "clean".
// ---------------------------------------------------------------------------
#[tokio::test]
async fn cancelled_status_does_not_stamp_even_with_no_errors() {
    let db = seeded_db();
    let db_path = db.path.to_str().unwrap().to_string();

    let status = Arc::new(Mutex::new(SyncStatus::default()));
    {
        let mut s = status.lock().await;
        s.cancel_requested = true;
    }
    stamp_explorer_sync_if_clean(&status, &db_path, PROFILE).await;

    assert!(
        stamped_at(&db).is_none(),
        "a cancelled run must not stamp last_explorer_sync_at"
    );
}
