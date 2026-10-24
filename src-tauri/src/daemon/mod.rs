//! Background sync daemon (`namehold-syncd`).
//!
//! Runs continuously, syncing all wallet profiles every 60 seconds.
//! Coordinates with the Tauri app via the `sync_locks` table to avoid
//! concurrent syncs of the same profile.
//!
//! COVERAGE: 0% — every function in this module is an IO shell driven by a
//! standalone process context: `run()` is an infinite tokio loop with signal
//! handling that never returns; `sync_all_profiles`/`sync_profile` open the
//! real on-disk DB by path and call sync commands that require live node RPC;
//! `write_pid_file`/`cleanup` mutate `~/.namehold/syncd.pid` on the developer's
//! actual home directory (unit-testing them would risk deleting or overwriting
//! a live daemon's PID file). The sync logic itself is fully covered inside
//! `commands::sync` (which this module thinly wraps); the DB-lock coordination
//! is covered inside `db::sync_lock`. What remains here is pure orchestration
//! against a real filesystem + tokio runtime + hsd node, structurally out of
//! scope for unit tests. The one exception is `send_finalize_reminders_with`,
//! which takes a connection and an emitter and is tested in
//! `tests::deadlines_cmd_tests`.

/// One line of the daemon's log, stamped with the UTC time. The log is the
/// daemon's stderr, which the app sends to `~/.namehold/syncd.log`
/// (`commands::daemon_ctl::spawn_detached`).
macro_rules! dlog {
    ($($arg:tt)*) => {
        eprintln!("{} {}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"), format_args!($($arg)*))
    };
}

use std::sync::Arc;
use std::time::Duration;

use crate::commands::sync::{self as sync_cmd, SyncStatus};
use crate::db::sync_lock::{self, LockOwnerType};
use crate::error::AppError;
use tokio::sync::Mutex as AsyncMutex;

pub mod watched_names;

/// How often the daemon runs a full sync cycle.
pub const SYNC_INTERVAL_SECS: u64 = 60;

/// How often the heartbeat is refreshed while syncing a profile.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// Main daemon loop: sync all profiles every 60 seconds.
pub fn run(db_path: &str) -> Result<(), AppError> {
    let conn = crate::db::connection::open(std::path::Path::new(db_path))?;
    crate::db::migrations::run(&conn)?;
    drop(conn); // Close this handle — each cycle re-opens fresh connections.

    // Write PID to a file so the app can detect if we're alive.
    write_pid_file()?;

    dlog!("namehold-syncd: starting (PID {})", std::process::id());

    // Build a single tokio runtime for the whole daemon lifetime.
    // Each cycle uses `block_on` to drive the async sync steps.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Other(format!("failed to build tokio runtime: {e}")))?;

    loop {
        if let Err(e) = rt.block_on(sync_all_profiles(db_path)) {
            dlog!("namehold-syncd: sync error: {e}");
        }

        // After syncing owned profiles, poll watched (non-owned) names and
        // emit OS notifications on phase transitions / re-opens / bidding-soon
        // / highest-bid threshold. No-op unless the user opted in via Settings.
        rt.block_on(watched_names::run_watched_scan(db_path));

        // Then the Shakedex finalize reminders (purchase, listing, cancel),
        // so they reach the user while the app window is closed. The app
        // sends none of them while this step's heartbeat is fresh.
        run_finalize_reminders(db_path);

        // Sleep 60 seconds before the next sync cycle.
        std::thread::sleep(Duration::from_secs(SYNC_INTERVAL_SECS));
    }
}

/// One cycle's finalize reminders, sent as OS notifications through
/// `notify-rust`. Errors are logged; the loop goes on.
fn run_finalize_reminders(db_path: &str) {
    let now = chrono::Utc::now().timestamp();
    let conn = match sync_cmd::open_conn(db_path) {
        Ok(conn) => conn,
        Err(e) => return dlog!("namehold-syncd: finalize reminders: {e}"),
    };
    let result = send_finalize_reminders_with(&conn, now, &mut |summary, body| {
        watched_names::emit_os_notification(summary, body)
    });
    match result {
        Ok(outcome) => {
            if !outcome.notified.is_empty() {
                let keys: Vec<&str> = outcome.notified.iter().map(|n| n.key.as_str()).collect();
                dlog!(
                    "namehold-syncd: sent {} finalize reminder(s): {}",
                    outcome.notified.len(),
                    keys.join(", ")
                );
            }
            if let Some(e) = outcome.delivery_error {
                dlog!("namehold-syncd: finalize reminder not shown: {e}");
            }
        }
        Err(e) => dlog!("namehold-syncd: finalize reminders: {e}"),
    }
}

/// The daemon's finalize-reminder step at unix time `now`, with its emitter
/// given: the shared delivery shell
/// (`commands::deadlines::deliver_finalize_reminders`, the same scanners,
/// setting and dedup state as the app) hands each reminder's title and body
/// to `emit`, whose error becomes the outcome's `delivery_error` as the app's
/// own does (the reminder stays claimed: at most once). Then the daemon's
/// heartbeat is written, with the reminders off too, so the app leaves them
/// to this process while it keeps running this step; a step that failed
/// writes none, and the app takes over once the last one is stale.
pub(crate) fn send_finalize_reminders_with(
    conn: &rusqlite::Connection,
    now: i64,
    emit: &mut dyn FnMut(&str, &str) -> Result<(), String>,
) -> Result<crate::commands::deadlines::ScanOutcome, AppError> {
    use crate::commands::deadlines::{
        deliver_finalize_reminders, record_daemon_heartbeat, PendingNotification,
    };
    let day = now.div_euclid(86_400);
    let mut notify = |n: &PendingNotification| emit(&n.title, &n.body).err();
    let outcome = deliver_finalize_reminders(conn, day, &mut notify)?;
    record_daemon_heartbeat(conn, now)?;
    Ok(outcome)
}

/// Sync all wallet profiles.
async fn sync_all_profiles(db_path: &str) -> Result<(), AppError> {
    // Open a short-lived connection just to list profiles.
    let conn = sync_cmd::open_conn(db_path)?;
    let profiles = crate::db::queries::list_wallet_profiles(&conn)?;
    drop(conn);

    for profile in profiles {
        let profile_id = profile.id.clone();

        // Try to acquire the lock for this profile (short-lived conn).
        let acquired = {
            let c = sync_cmd::open_conn(db_path)?;
            sync_lock::try_acquire(&c, &profile_id, LockOwnerType::Daemon)?
        };
        if !acquired {
            // Another process (app or another daemon instance) is syncing this profile.
            dlog!(
                "namehold-syncd: skipping profile {} (lock held by another process)",
                profile_id
            );
            continue;
        }

        // Spawn a heartbeat refresher for this profile. It raises
        // `lock_lost` when the app (or another process) takes the lock over,
        // and the run then stops before its next step.
        let hb_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let lock_lost = sync_lock::LockLost::default();
        let hb_handle = sync_cmd::spawn_lock_heartbeat(
            db_path.to_string(),
            profile_id.clone(),
            hb_stop.clone(),
            lock_lost.clone(),
        );

        // Run the sync steps. If any step fails internally, it's logged;
        // the daemon just moves on. `sync_profile` itself never errors.
        sync_profile(db_path, &profile_id, &lock_lost).await;
        if lock_lost.is_lost() {
            dlog!(
                "namehold-syncd: stopped syncing profile {profile_id}: its sync lock was taken over"
            );
        } else {
            dlog!("namehold-syncd: synced profile {}", profile_id);
        }

        // Stop the heartbeat thread.
        hb_stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = hb_handle.join();

        // Release the lock (a no-op when it was taken over: only our own
        // row is deleted).
        let c = sync_cmd::open_conn(db_path)?;
        sync_lock::release(&c, &profile_id)?;
    }

    Ok(())
}

/// Sync a single profile (3 steps: node sync, repair, discover).
///
/// Uses a locally-owned `SyncStatus` (never surfaced anywhere) purely as
/// the argument type expected by the shared step functions, and to carry
/// `lock_lost` (raised by the profile's heartbeat) to them. The daemon
/// intentionally does not report progress to any UI.
pub(crate) async fn sync_profile(db_path: &str, profile_id: &str, lock_lost: &sync_lock::LockLost) {
    // Local status — never shared, never observed. Just satisfies the API of
    // the shared step functions, which use it internally for progress labels
    // and the stop checks. The daemon never sets cancel_requested; a lost
    // lock stops the steps as it does the app's.
    let status: Arc<AsyncMutex<SyncStatus>> = Arc::new(AsyncMutex::new(SyncStatus::default()));
    {
        let mut s = status.lock().await;
        s.running = true;
        s.lock_lost = lock_lost.clone();
    }

    // Run the shared sync steps (the single source of truth, also
    // used by the app's `start_full_sync`). The daemon caller: it has no UI
    // to poll the progress labels, and it never broadcasts.
    sync_cmd::run_sync_steps(&status, db_path, profile_id, sync_cmd::SyncCaller::Daemon).await;

    // Stamp the explorer sync timestamp on a clean run (same policy as
    // start_full_sync).
    sync_cmd::stamp_explorer_sync_if_clean(&status, db_path, profile_id).await;
}

/// Write the daemon's PID to `~/.namehold/syncd.pid`.
fn write_pid_file() -> Result<(), AppError> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".to_string()))?;
    let pid_path = home.join(".namehold").join("syncd.pid");

    std::fs::create_dir_all(pid_path.parent().unwrap())?;
    std::fs::write(&pid_path, format!("{}\n", std::process::id()))?;

    Ok(())
}

/// Clean up on daemon exit.
pub fn cleanup(db_path: &str) -> Result<(), AppError> {
    let conn = sync_cmd::open_conn(db_path)?;
    sync_lock::release_all_owned(&conn)?;
    // Best-effort: remove the PID file (harmless if it fails).
    if let Some(home) = dirs::home_dir() {
        let pid_path = home.join(".namehold").join("syncd.pid");
        let _ = std::fs::remove_file(&pid_path);
    }
    dlog!("namehold-syncd: cleaned up locks");
    Ok(())
}
