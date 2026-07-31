//! Background sync daemon (`namehold-syncd`).
//!
//! Runs continuously, syncing all wallet profiles every 60 seconds.
//! Coordinates with the Tauri app via the `sync_locks` table to avoid
//! concurrent syncs of the same profile.

use std::sync::Arc;
use std::time::Duration;

use crate::commands::sync::{self as sync_cmd, SyncStatus};
use crate::db::sync_lock::{self, LockOwnerType};
use crate::error::AppError;
use tokio::sync::Mutex as AsyncMutex;

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

    eprintln!("namehold-syncd: starting (PID {})", std::process::id());

    // Build a single tokio runtime for the whole daemon lifetime.
    // Each cycle uses `block_on` to drive the async sync steps.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Other(format!("failed to build tokio runtime: {e}")))?;

    loop {
        if let Err(e) = rt.block_on(sync_all_profiles(db_path)) {
            eprintln!("namehold-syncd: sync error: {e}");
        }

        // Sleep 60 seconds before the next sync cycle.
        std::thread::sleep(Duration::from_secs(SYNC_INTERVAL_SECS));
    }
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
            eprintln!(
                "namehold-syncd: skipping profile {} (lock held by another process)",
                profile_id
            );
            continue;
        }

        // Spawn a heartbeat refresher for this profile.
        let hb_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hb_handle = spawn_heartbeat(db_path.to_string(), profile_id.clone(), hb_stop.clone());

        // Run the 3-step sync. If any step fails internally, it's logged;
        // the daemon just moves on. `sync_profile` itself never errors.
        sync_profile(db_path, &profile_id).await;
        eprintln!("namehold-syncd: synced profile {}", profile_id);

        // Stop the heartbeat thread.
        hb_stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = hb_handle.join();

        // Release the lock.
        let c = sync_cmd::open_conn(db_path)?;
        sync_lock::release(&c, &profile_id)?;
    }

    Ok(())
}

/// Sync a single profile (3 steps: node sync, repair, discover).
///
/// Uses a locally-owned `SyncStatus` (never surfaced anywhere) purely as
/// the argument type expected by the shared step functions. The daemon
/// intentionally does not report progress to any UI.
async fn sync_profile(db_path: &str, profile_id: &str) {
    // Local status — never shared, never observed. Just satisfies the API of
    // the shared step functions, which use it internally for progress labels
    // and cancel_requested checks. The daemon never sets cancel_requested,
    // so the steps run to completion.
    let status: Arc<AsyncMutex<SyncStatus>> = Arc::new(AsyncMutex::new(SyncStatus::default()));
    {
        let mut s = status.lock().await;
        s.running = true;
    }

    // Step 1: node UTXO sync (always attempted).
    let _ = sync_cmd::sync_node_step(db_path, profile_id).await;

    // Determine if the node is authoritative (connected + fully synced).
    // If so, we skip the explorer-driven repair/discover in favor of the
    // node-driven equivalents — same policy as `start_full_sync`.
    let node_authoritative = {
        let settings = sync_cmd::open_conn(db_path)
            .ok()
            .and_then(|c| crate::db::queries::get_settings(&c).ok());
        match settings {
            Some(s) => crate::commands::read::node_ready_from_settings(&s).await,
            None => false,
        }
    };

    // Step 2: repair owned names (explorer path only; node path is handled
    // inline by node_discover_step in step 3).
    if !node_authoritative {
        sync_cmd::repair_step(&status, db_path, profile_id).await;
    }

    // Step 3: discover new names (node or explorer path).
    if node_authoritative {
        sync_cmd::node_discover_step(db_path, profile_id).await;
    } else {
        sync_cmd::discover_step(&status, db_path, profile_id).await;
    }

    // Stamp the explorer sync timestamp on a clean run (same policy as
    // start_full_sync).
    sync_cmd::stamp_explorer_sync_if_clean(&status, db_path, profile_id).await;
}

/// Spawn a background thread that refreshes the sync lock's heartbeat every
/// [`HEARTBEAT_INTERVAL_SECS`] until told to stop.
fn spawn_heartbeat(
    db_path: String,
    profile_id: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
            if stop.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            if let Ok(c) = sync_cmd::open_conn(&db_path) {
                if let Err(e) = sync_lock::refresh_heartbeat(&c, &profile_id) {
                    eprintln!(
                        "namehold-syncd: heartbeat refresh failed for {}: {}",
                        profile_id, e
                    );
                }
            }
        }
    })
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
    eprintln!("namehold-syncd: cleaned up locks");
    Ok(())
}
