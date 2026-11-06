//! Startup with pending database migrations.
//!
//! A migration can take minutes (0.7.1's dropped a 9 GB table), and running it
//! inside Tauri's setup kept the window from appearing while macOS reported
//! the app as not responding. When migrations are pending, setup now hands
//! them to [`spawn_migrations`] and returns; the window asks
//! [`startup_status`] what is happening and shows the progress, and the rest of
//! setup ([`crate::finish_setup`]) runs on the main thread once they are done.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Manager, State};

use crate::db::read_meter;

/// What the window shows before the app proper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StartupStatus {
    /// Setup has not decided yet (the window asked very early).
    Starting,
    /// Migration `step` of `total` is running.
    Migrating {
        step: usize,
        total: usize,
        version: String,
        /// Seconds since the first migration started.
        elapsed_secs: u64,
        /// How much of the file this step has read, 0–99: the measure of a
        /// long step such as dropping a large table. `None` before any step.
        percent: Option<u8>,
    },
    /// A migration or the setup after it failed; nothing is set up.
    Failed {
        error: String,
    },
    Ready,
}

#[derive(Debug)]
enum Phase {
    Starting,
    Migrating {
        started: Instant,
        step: usize,
        total: usize,
        version: String,
        step_read_from: u64,
        step_estimate: u64,
    },
    Failed(String),
    Ready,
}

/// Managed first thing in setup, so `startup_status` always answers.
#[derive(Debug)]
pub struct StartupState(Mutex<Phase>);

impl Default for StartupState {
    fn default() -> Self {
        Self(Mutex::new(Phase::Starting))
    }
}

impl StartupState {
    fn with<T>(&self, f: impl FnOnce(&mut Phase) -> T) -> T {
        let mut phase = self.0.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut phase)
    }

    pub fn set_ready(&self) {
        self.with(|p| *p = Phase::Ready);
    }

    pub fn set_failed(&self, error: impl Into<String>) {
        let error = error.into();
        eprintln!("startup: {error}");
        self.with(|p| *p = Phase::Failed(error));
    }

    /// Claim the migration run: true from `Starting` or after a failure, false
    /// while one runs or once the app is ready.
    fn begin(&self) -> bool {
        self.with(|p| match p {
            Phase::Starting | Phase::Failed(_) => {
                *p = Phase::Migrating {
                    started: Instant::now(),
                    step: 0,
                    total: 0,
                    version: String::new(),
                    step_read_from: read_meter::bytes_read(),
                    step_estimate: 0,
                };
                true
            }
            Phase::Migrating { .. } | Phase::Ready => false,
        })
    }

    fn step(&self, step: usize, total: usize, version: &str, estimate: u64) {
        self.with(|p| {
            if let Phase::Migrating { started, .. } = p {
                *p = Phase::Migrating {
                    started: *started,
                    step,
                    total,
                    version: version.to_string(),
                    step_read_from: read_meter::bytes_read(),
                    step_estimate: estimate,
                };
            }
        });
    }

    pub fn status(&self) -> StartupStatus {
        self.status_at(read_meter::bytes_read())
    }

    fn status_at(&self, bytes_read: u64) -> StartupStatus {
        self.with(|p| match p {
            Phase::Starting => StartupStatus::Starting,
            Phase::Ready => StartupStatus::Ready,
            Phase::Failed(error) => StartupStatus::Failed {
                error: error.clone(),
            },
            Phase::Migrating {
                started,
                step,
                total,
                version,
                step_read_from,
                step_estimate,
            } => StartupStatus::Migrating {
                step: *step,
                total: *total,
                version: version.clone(),
                elapsed_secs: started.elapsed().as_secs(),
                percent: (*step > 0)
                    .then(|| percent(bytes_read.saturating_sub(*step_read_from), *step_estimate)),
            },
        })
    }
}

/// Share of `estimate` that `read` is, held below 100 until the step ends:
/// the file size is only an estimate of what a step reads.
fn percent(read: u64, estimate: u64) -> u8 {
    if estimate == 0 {
        return 0;
    }
    u8::try_from((read.saturating_mul(100) / estimate).min(99)).unwrap_or(99)
}

/// The database's size in bytes as SQLite sees it (pages in use).
fn used_bytes(conn: &rusqlite::Connection) -> u64 {
    let pragma = |name: &str| -> u64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, i64>(0))
            .ok()
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or(0)
    };
    pragma("page_count")
        .saturating_sub(pragma("freelist_count"))
        .saturating_mul(pragma("page_size"))
}

/// Run the pending migrations on `db_path`, reporting each step to `state`.
pub(crate) fn migrate(state: &StartupState, db_path: &Path) -> Result<(), String> {
    let conn = crate::db::connection::open_metered(db_path).map_err(|e| e.to_string())?;
    crate::db::migrations::run_with_progress(&conn, |step, total, version| {
        state.step(step, total, version, used_bytes(&conn));
    })
    .map_err(|e| format!("database update {} failed: {e}", current_version(state)))
}

fn current_version(state: &StartupState) -> String {
    state.with(|p| match p {
        Phase::Migrating { version, .. } => version.clone(),
        _ => String::new(),
    })
}

/// Run the pending migrations on a thread of their own, then finish setup on
/// the main thread. Does nothing while a run is going or once the app is
/// ready, so a second call (a retry clicked twice) is harmless.
pub fn spawn_migrations(app: AppHandle, db_path: PathBuf) {
    if !app.state::<StartupState>().begin() {
        return;
    }
    let thread_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("migrations".into())
        .spawn(move || {
            let app = thread_app;
            let state = app.state::<StartupState>();
            if let Err(e) = migrate(&state, &db_path) {
                state.set_failed(e);
                return;
            }
            let handle = app.clone();
            let queued = app.run_on_main_thread(move || {
                let state = handle.state::<StartupState>();
                let finished = crate::db::connection::open(&db_path)
                    .map_err(|e| e.to_string())
                    .and_then(|conn| {
                        crate::finish_setup(&handle, conn, &db_path).map_err(|e| e.to_string())
                    });
                match finished {
                    Ok(()) => state.set_ready(),
                    Err(e) => {
                        state.set_failed(format!("startup after the database update failed: {e}"))
                    }
                }
            });
            if let Err(e) = queued {
                state.set_failed(format!("startup after the database update failed: {e}"));
            }
        });
    if let Err(e) = spawned {
        app.state::<StartupState>()
            .set_failed(format!("could not start the database update: {e}"));
    }
}

/// What the window should show: progress, an error, or the app.
#[tauri::command]
pub fn startup_status(state: State<'_, StartupState>) -> StartupStatus {
    state.status()
}

/// Try the database update again after it failed.
#[tauri::command]
pub fn retry_startup(app: AppHandle) {
    let path = crate::db_file_path(&app);
    spawn_migrations(app, path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_stays_below_100_and_handles_no_estimate() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(50, 100), 50);
        assert_eq!(percent(500, 100), 99);
        assert_eq!(percent(u64::MAX, 3), 99);
    }

    #[test]
    fn a_run_reports_its_step_and_the_share_read() {
        let state = StartupState::default();
        assert_eq!(state.status(), StartupStatus::Starting);
        assert!(state.begin());
        assert!(!state.begin(), "a second run must not start while one runs");

        state.step(1, 2, "042", 1000);
        let from = state.with(|p| match p {
            Phase::Migrating { step_read_from, .. } => *step_read_from,
            _ => unreachable!(),
        });
        match state.status_at(from + 250) {
            StartupStatus::Migrating {
                step,
                total,
                version,
                percent,
                ..
            } => {
                assert_eq!(
                    (step, total, version.as_str(), percent),
                    (1, 2, "042", Some(25))
                );
            }
            other => panic!("unexpected {other:?}"),
        }

        state.set_failed("disk full");
        assert_eq!(
            state.status(),
            StartupStatus::Failed {
                error: "disk full".into()
            }
        );
        assert!(state.begin(), "a failed run can be retried");
        state.set_ready();
        assert!(!state.begin());
        assert_eq!(state.status(), StartupStatus::Ready);
    }

    #[test]
    fn migrate_runs_every_pending_step_on_a_file() {
        let dir = std::env::temp_dir().join(format!("namehold_startup_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("portfolio.db");

        let state = StartupState::default();
        assert!(state.begin());
        migrate(&state, &path).unwrap();
        let conn = crate::db::connection::open(&path).unwrap();
        assert!(crate::db::migrations::pending(&conn).unwrap().is_empty());
        assert!(matches!(
            state.status(),
            StartupStatus::Migrating { step, total, .. } if step == total && total > 0
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
