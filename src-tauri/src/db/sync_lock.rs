//! Cross-process sync lock coordination.
//!
//! Both the Tauri app and the background `namehold-syncd` daemon use the same
//! SQLite database. The `sync_locks` table ensures only one process syncs a
//! given profile at a time. A heartbeat mechanism prevents deadlocks after
//! crashes: if a lock's heartbeat is older than [`STALE_THRESHOLD_SECS`], any
//! other process may take it over.

use rusqlite::Connection;

use crate::error::AppError;

/// How long (seconds) before a lock is considered stale and can be taken over.
pub const STALE_THRESHOLD_SECS: i64 = 30;

/// Who holds the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOwnerType {
    App,
    Daemon,
}

impl LockOwnerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            LockOwnerType::App => "app",
            LockOwnerType::Daemon => "daemon",
        }
    }
}

/// Information about a currently-held lock.
#[derive(Debug)]
pub struct SyncLockInfo {
    pub profile_id: String,
    pub owner_pid: u32,
    pub owner_type: String,
    pub acquired_at: i64,
    pub heartbeat_at: i64,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Attempt to acquire the sync lock for a profile.
///
/// Returns `true` if the lock was acquired, `false` if another process holds
/// a fresh (non-stale) lock.
pub fn try_acquire(
    conn: &Connection,
    profile_id: &str,
    owner_type: LockOwnerType,
) -> Result<bool, AppError> {
    let now = now_unix();
    let pid = std::process::id();
    let stale_cutoff = now - STALE_THRESHOLD_SECS;

    // Try to insert a new lock. If one already exists:
    // - If it's stale (heartbeat_at < stale_cutoff), take it over.
    // - If it's ours (same PID), refresh it.
    // - Otherwise, fail (another process holds it).
    let existing: Option<(u32, i64)> = conn
        .query_row(
            "SELECT owner_pid, heartbeat_at FROM sync_locks WHERE profile_id = ?1",
            [profile_id],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, i64>(1)?)),
        )
        .ok();

    match existing {
        None => {
            // No lock exists — acquire it.
            conn.execute(
                "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![profile_id, pid, owner_type.as_str(), now, now],
            )?;
            Ok(true)
        }
        Some((existing_pid, heartbeat)) => {
            if existing_pid == pid {
                // We already hold it (e.g., re-entrant call). Refresh heartbeat.
                conn.execute(
                    "UPDATE sync_locks SET heartbeat_at = ?1 WHERE profile_id = ?2",
                    rusqlite::params![now, profile_id],
                )?;
                Ok(true)
            } else if heartbeat < stale_cutoff {
                // Stale lock — take it over.
                conn.execute(
                    "UPDATE sync_locks
                     SET owner_pid = ?1, owner_type = ?2, acquired_at = ?3, heartbeat_at = ?4
                     WHERE profile_id = ?5",
                    rusqlite::params![pid, owner_type.as_str(), now, now, profile_id],
                )?;
                Ok(true)
            } else {
                // Another process holds a fresh lock.
                Ok(false)
            }
        }
    }
}

/// Refresh the heartbeat for a lock we hold.
///
/// Should be called periodically (every ~10s) while sync is in progress.
/// Returns `true` if the heartbeat was updated, `false` if we no longer hold
/// the lock (e.g., it was taken over by another process).
pub fn refresh_heartbeat(conn: &Connection, profile_id: &str) -> Result<bool, AppError> {
    let now = now_unix();
    let pid = std::process::id();
    let updated = conn.execute(
        "UPDATE sync_locks SET heartbeat_at = ?1 WHERE profile_id = ?2 AND owner_pid = ?3",
        rusqlite::params![now, profile_id, pid],
    )?;
    Ok(updated > 0)
}

/// Release the sync lock for a profile.
///
/// Only releases if we (this PID) hold it.
pub fn release(conn: &Connection, profile_id: &str) -> Result<(), AppError> {
    let pid = std::process::id();
    conn.execute(
        "DELETE FROM sync_locks WHERE profile_id = ?1 AND owner_pid = ?2",
        rusqlite::params![profile_id, pid],
    )?;
    Ok(())
}

/// Check if any process holds the lock for a profile.
pub fn get_lock_info(
    conn: &Connection,
    profile_id: &str,
) -> Result<Option<SyncLockInfo>, AppError> {
    let result = conn
        .query_row(
            "SELECT profile_id, owner_pid, owner_type, acquired_at, heartbeat_at
             FROM sync_locks WHERE profile_id = ?1",
            [profile_id],
            |row| {
                Ok(SyncLockInfo {
                    profile_id: row.get(0)?,
                    owner_pid: row.get(1)?,
                    owner_type: row.get(2)?,
                    acquired_at: row.get(3)?,
                    heartbeat_at: row.get(4)?,
                })
            },
        )
        .ok();
    Ok(result)
}

/// Release all locks held by this process (cleanup on exit).
pub fn release_all_owned(conn: &Connection) -> Result<(), AppError> {
    let pid = std::process::id();
    conn.execute(
        "DELETE FROM sync_locks WHERE owner_pid = ?1",
        rusqlite::params![pid],
    )?;
    Ok(())
}

/// Force-acquire the lock for the app, preempting any daemon-held lock.
///
/// The app takes priority over the daemon: when the user clicks Sync, the app
/// grabs the lock unconditionally, stealing it from the daemon if necessary.
/// The daemon detects the theft on its next [`refresh_heartbeat`] (returns
/// `false`) and abandons its in-flight sync of that profile cleanly.
///
/// The only lock this will NOT steal is one held by another *app* process
/// with a fresh heartbeat — but there is only ever one app instance (single
/// window), so in practice this always succeeds. Returns `false` only if a
/// fresh lock is held by a different PID that is also of type `app`.
pub fn acquire_for_app(conn: &Connection, profile_id: &str) -> Result<bool, AppError> {
    let now = now_unix();
    let pid = std::process::id();
    let stale_cutoff = now - STALE_THRESHOLD_SECS;

    let existing: Option<(u32, String, i64)> = conn
        .query_row(
            "SELECT owner_pid, owner_type, heartbeat_at FROM sync_locks WHERE profile_id = ?1",
            [profile_id],
            |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .ok();

    match existing {
        None => {
            conn.execute(
                "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
                 VALUES (?1, ?2, 'app', ?3, ?4)",
                rusqlite::params![profile_id, pid, now, now],
            )?;
            Ok(true)
        }
        Some((existing_pid, owner_type, heartbeat)) => {
            // Refuse only if a *different* app instance holds a fresh lock.
            if existing_pid != pid && owner_type == "app" && heartbeat >= stale_cutoff {
                return Ok(false);
            }
            // Otherwise (ours, daemon-held, or stale) — take it over for the app.
            conn.execute(
                "UPDATE sync_locks
                 SET owner_pid = ?1, owner_type = 'app', acquired_at = ?2, heartbeat_at = ?3
                 WHERE profile_id = ?4",
                rusqlite::params![pid, now, now, profile_id],
            )?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn acquire_and_release() {
        let conn = setup_db();
        assert!(try_acquire(&conn, "p1", LockOwnerType::App).unwrap());
        assert!(get_lock_info(&conn, "p1").unwrap().is_some());
        release(&conn, "p1").unwrap();
        assert!(get_lock_info(&conn, "p1").unwrap().is_none());
    }

    #[test]
    fn reentrant_acquire_succeeds() {
        let conn = setup_db();
        assert!(try_acquire(&conn, "p1", LockOwnerType::App).unwrap());
        // Same PID acquiring again should succeed (refresh heartbeat).
        assert!(try_acquire(&conn, "p1", LockOwnerType::App).unwrap());
    }

    #[test]
    fn fresh_lock_blocks_other_pid() {
        let conn = setup_db();
        let now = now_unix();
        // Simulate another PID holding the lock with a fresh heartbeat.
        conn.execute(
            "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
             VALUES ('p1', 99999, 'daemon', ?1, ?2)",
            rusqlite::params![now, now],
        )
        .unwrap();

        // Our PID should fail to acquire.
        assert!(!try_acquire(&conn, "p1", LockOwnerType::App).unwrap());
    }

    #[test]
    fn stale_lock_is_taken_over() {
        let conn = setup_db();
        let stale_time = now_unix() - STALE_THRESHOLD_SECS - 5;
        // Simulate a stale lock from another PID.
        conn.execute(
            "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
             VALUES ('p1', 99999, 'daemon', ?1, ?2)",
            rusqlite::params![stale_time, stale_time],
        )
        .unwrap();

        // We should be able to take it over.
        assert!(try_acquire(&conn, "p1", LockOwnerType::App).unwrap());
        let info = get_lock_info(&conn, "p1").unwrap().unwrap();
        assert_eq!(info.owner_pid, std::process::id());
    }

    #[test]
    fn refresh_heartbeat_works() {
        let conn = setup_db();
        assert!(try_acquire(&conn, "p1", LockOwnerType::Daemon).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(refresh_heartbeat(&conn, "p1").unwrap());
        let info = get_lock_info(&conn, "p1").unwrap().unwrap();
        assert!(info.heartbeat_at >= info.acquired_at);
    }

    #[test]
    fn release_all_owned_clears_our_locks() {
        let conn = setup_db();
        assert!(try_acquire(&conn, "p1", LockOwnerType::App).unwrap());
        assert!(try_acquire(&conn, "p2", LockOwnerType::App).unwrap());
        release_all_owned(&conn).unwrap();
        assert!(get_lock_info(&conn, "p1").unwrap().is_none());
        assert!(get_lock_info(&conn, "p2").unwrap().is_none());
    }

    #[test]
    fn acquire_for_app_preempts_daemon_lock() {
        let conn = setup_db();
        let now = now_unix();
        // Simulate a fresh daemon lock from another PID.
        conn.execute(
            "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
             VALUES ('p1', 99999, 'daemon', ?1, ?2)",
            rusqlite::params![now, now],
        )
        .unwrap();

        // The app should steal the lock even though it's fresh.
        assert!(acquire_for_app(&conn, "p1").unwrap());
        let info = get_lock_info(&conn, "p1").unwrap().unwrap();
        assert_eq!(info.owner_pid, std::process::id());
        assert_eq!(info.owner_type, "app");
    }

    #[test]
    fn acquire_for_app_refuses_fresh_app_lock_from_other_pid() {
        let conn = setup_db();
        let now = now_unix();
        // Another app instance holds the lock.
        conn.execute(
            "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
             VALUES ('p1', 99999, 'app', ?1, ?2)",
            rusqlite::params![now, now],
        )
        .unwrap();

        assert!(!acquire_for_app(&conn, "p1").unwrap());
    }

    #[test]
    fn daemon_detects_lock_theft_via_heartbeat() {
        let conn = setup_db();
        // Simulate daemon lock.
        let now = now_unix();
        conn.execute(
            "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
             VALUES ('p1', 99999, 'daemon', ?1, ?2)",
            rusqlite::params![now, now],
        )
        .unwrap();

        // App steals it.
        acquire_for_app(&conn, "p1").unwrap();

        // Daemon's refresh (using PID 99999) should fail — no rows updated.
        // We can't easily mock a different PID here, so verify the row's
        // owner_pid is now the current PID and owner_type is 'app'.
        let info = get_lock_info(&conn, "p1").unwrap().unwrap();
        assert_ne!(info.owner_pid, 99999);
        assert_eq!(info.owner_type, "app");
    }
}
