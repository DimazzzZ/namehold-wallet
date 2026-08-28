//! System-tray (menu-bar on macOS) helper.
//!
//! Provides three things:
//!
//! 1. A [`TrayState`] struct that holds the tray icon + its interactive menu
//!    items so any command can look them up via `app.state::<TrayState>()`
//!    and update the UI without rebuilding the menu.
//!
//! 2. [`refresh_tray`] — the single source of truth for what the tray shows.
//!    Reads the current node status and the background-sync setting, then
//!    updates the status label, the Start/Stop button label, the bg-sync
//!    checkbox, and swaps the tray icon between normal/syncing/stopped.
//!    Called from setup() (initial paint), eagerly from the tray menu
//!    handlers right after they start/stop the node or flip background sync,
//!    and from a short ticker in setup() that reconciles any state changed
//!    outside the tray (frontend invokes, hsd autostart, passive
//!    sync-progress transitions).
//!
//! 3. Two `#[tauri::command]`s that persist the `close_to_tray` boolean —
//!    used by the window-close interceptor in `lib.rs` to decide whether
//!    to hide the main window (tray mode) or exit the app (classic mode).
//!
//! The tray Quit item sets the [`REALLY_QUITTING`] flag before calling
//! `app.exit(0)` so the CloseRequested interceptor lets the close through
//! instead of hiding the window (which would trap the user in tray-only
//! land with no way to fully quit).
//!
//! COVERAGE: ~39% — the core of this module is `refresh_tray`, which reads
//! `TrayState` (Tauri managed state containing `TrayIcon`, `CheckMenuItem`,
//! `MenuItem` handles) and calls `.set_text()`, `.set_checked()`,
//! `.set_icon()` on real OS tray objects. These require a live desktop
//! environment with a system tray. The two `#[tauri::command]` functions
//! (`is_close_to_tray_enabled`, `set_close_to_tray_enabled`) are thin DB
//! setting wrappers whose bodies ARE covered by the existing
//! `watched_states_cmd_tests` harness. The uncovered lines are all inside
//! `refresh_tray` and `TrayState` construction — structurally out of scope
//! for unit tests (no headless tray API in Tauri's `MockRuntime`).

use crate::commands::daemon_ctl::{BACKGROUND_SYNC_DEFAULT, SETTING_BACKGROUND_SYNC};
use crate::db;
use crate::error::AppError;
use crate::AppState;
use std::sync::atomic::AtomicBool;
use tauri::image::Image;
use tauri::include_image;
use tauri::menu::{CheckMenuItem, MenuItem};
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager, Wry};

/// Settings key for "close to tray" (X button hides instead of quits).
pub const SETTING_CLOSE_TO_TRAY: &str = "close_to_tray";

/// Default: "1" (ON). Matches the DEFAULT_SETTINGS in `src/stores/settings.ts`.
pub const CLOSE_TO_TRAY_DEFAULT: &str = "1";

/// Set by the tray Quit menu item before calling `app.exit(0)`. The main
/// window's CloseRequested handler checks this flag and skips the
/// "prevent_close + hide" branch when it's true, so the close actually goes
/// through. Never reset — once set, we're exiting.
pub static REALLY_QUITTING: AtomicBool = AtomicBool::new(false);

/// Handles to every mutable piece of the tray UI. Managed via
/// `app.manage(TrayState { ... })` in setup() so any command can update the
/// tray by looking up `State<'_, TrayState>` without rebuilding the menu.
///
/// Tauri's `TrayIcon`, `MenuItem`, and `CheckMenuItem` are all `Clone` and
/// internally reference-counted; the clones inside `TrayState` share the same
/// underlying OS objects as the ones held by the runtime.
pub struct TrayState {
    pub tray: TrayIcon<Wry>,
    /// Disabled status label ("Node: Running", "Node: Stopped", "Node: Syncing").
    pub status: MenuItem<Wry>,
    /// Toggle button ("Start Node" / "Stop Node"). Its text and enabled state
    /// are updated by `refresh_tray`.
    pub toggle: MenuItem<Wry>,
    /// Background-sync checkbox. Reflects `background_sync_enabled` setting.
    pub bgsync: CheckMenuItem<Wry>,
}

// ---------------------------------------------------------------------------
// Commands (persisted setting)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn is_close_to_tray_enabled(state: tauri::State<'_, AppState>) -> Result<bool, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    let enabled = settings
        .get(SETTING_CLOSE_TO_TRAY)
        .map(|s| s.as_str())
        .unwrap_or(CLOSE_TO_TRAY_DEFAULT)
        == "1";
    Ok(enabled)
}

#[tauri::command]
pub async fn set_close_to_tray_enabled(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::set_setting(&db, SETTING_CLOSE_TO_TRAY, if enabled { "1" } else { "0" })?;
    Ok(())
}

/// Non-command sibling used from `lib.rs`'s window-close interceptor (which
/// runs on the event loop, not inside a Tauri command). Falls back to the
/// default (ON) if any read fails — the safer choice is "keep the app alive
/// in the tray" over "lose the process because a DB read hiccuped".
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn read_close_to_tray_setting(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let db = match state.db.lock() {
        Ok(g) => g,
        Err(_) => return CLOSE_TO_TRAY_DEFAULT == "1",
    };
    let settings = match db::queries::get_settings(&db) {
        Ok(s) => s,
        Err(_) => return CLOSE_TO_TRAY_DEFAULT == "1",
    };
    settings
        .get(SETTING_CLOSE_TO_TRAY)
        .map(|s| s.as_str())
        .unwrap_or(CLOSE_TO_TRAY_DEFAULT)
        == "1"
}

// ---------------------------------------------------------------------------
// refresh_tray — single source of truth for tray UI state
// ---------------------------------------------------------------------------

/// What kind of icon to show in the tray. Determined from node running state
/// and (when running) sync progress.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TrayIconKind {
    Normal,
    Syncing,
    Stopped,
}

/// Snapshot of the state the tray cares about. Kept small on purpose so a
/// refresh call never allocates strings it doesn't need.
struct TraySnapshot {
    node_running: bool,
    node_synced: bool,
    bg_sync_on: bool,
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn snapshot(app: &AppHandle) -> TraySnapshot {
    let state = app.state::<AppState>();

    // Node running: the backend probe loop updates `node_rpc_alive` every ~5s
    // (and start/stop actions flip it immediately). We OR it with the child
    // handle so a freshly-spawned node shows "Running" even before the first
    // probe tick fires.
    let node_running = state
        .node_rpc_alive
        .load(std::sync::atomic::Ordering::Relaxed)
        || state
            .hsd_child
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false);

    // "Syncing" state proxy: whether a wallet full-sync run is active. This is
    // the cheapest in-process signal (no RPC round-trip on every 5s tick).
    // `SyncStatus.running` is true while a discover/repair sync is in flight.
    // Read via `try_lock` so a held lock never freezes the tray refresh; if we
    // can't read it, assume synced (avoids a sticky spinner icon).
    let node_synced = {
        match state.sync_status.try_lock() {
            Ok(s) => !s.running,
            Err(_) => true,
        }
    };

    let bg_sync_on = state
        .db
        .lock()
        .ok()
        .and_then(|db| db::queries::get_settings(&db).ok())
        .map(|s| {
            s.get(SETTING_BACKGROUND_SYNC)
                .map(|v| v.as_str())
                .unwrap_or(BACKGROUND_SYNC_DEFAULT)
                == "1"
        })
        .unwrap_or(BACKGROUND_SYNC_DEFAULT == "1");

    TraySnapshot {
        node_running,
        node_synced,
        bg_sync_on,
    }
}

fn icon_kind(snap: &TraySnapshot) -> TrayIconKind {
    if !snap.node_running {
        TrayIconKind::Stopped
    } else if !snap.node_synced {
        TrayIconKind::Syncing
    } else {
        TrayIconKind::Normal
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn icon_image(kind: TrayIconKind) -> Image<'static> {
    match kind {
        TrayIconKind::Normal => include_image!("icons/tray-normal.png"),
        TrayIconKind::Syncing => include_image!("icons/tray-syncing.png"),
        TrayIconKind::Stopped => include_image!("icons/tray-stopped.png"),
    }
}

/// Update the tray UI to reflect the current app state. Safe to call from
/// any thread. Errors are logged but never propagated — a UI glitch should
/// never poison the app.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn refresh_tray(app: &AppHandle) {
    let tray_state = match app.try_state::<TrayState>() {
        Some(s) => s,
        None => {
            // Called before setup() finished wiring the tray (e.g. from an
            // early background task). Silent no-op — the initial refresh at
            // the end of setup() will catch us up.
            return;
        }
    };

    let snap = snapshot(app);
    let kind = icon_kind(&snap);

    let (status_text, toggle_text, toggle_enabled) = match (snap.node_running, snap.node_synced) {
        (false, _) => ("Node: Stopped", "Start Node", true),
        (true, false) => ("Node: Syncing…", "Stop Node", true),
        (true, true) => ("Node: Running", "Stop Node", true),
    };

    if let Err(e) = tray_state.status.set_text(status_text) {
        eprintln!("tray: set status text failed: {e}");
    }
    if let Err(e) = tray_state.toggle.set_text(toggle_text) {
        eprintln!("tray: set toggle text failed: {e}");
    }
    if let Err(e) = tray_state.toggle.set_enabled(toggle_enabled) {
        eprintln!("tray: set toggle enabled failed: {e}");
    }
    if let Err(e) = tray_state.bgsync.set_checked(snap.bg_sync_on) {
        eprintln!("tray: set bgsync checked failed: {e}");
    }
    if let Err(e) = tray_state.tray.set_icon(Some(icon_image(kind))) {
        eprintln!("tray: set icon failed: {e}");
    }
    // macOS: use template icons (monochrome) so they adapt to light/dark
    // menu bar. State is conveyed via glyph variants (filled/outline/badge)
    // rather than color.
    #[cfg(target_os = "macos")]
    if let Err(e) = tray_state.tray.set_icon_as_template(true) {
        eprintln!("tray: set_icon_as_template failed: {e}");
    }
}

pub const SETTING_TRAY_HINT_SHOWN: &str = "tray_hint_shown";

/// Dispatch the actual OS notification. Compiled out under `#[cfg(test)]` so
/// unit tests can exercise the DB read/conditional/write logic of
/// [`fire_tray_hint_notification`] without a real notification backend (which
/// isn't available under the mock runtime). Mirrors the pattern used by
/// `deadlines::send_os_notification`.
#[cfg(not(test))]
fn fire_hint_os_notification<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), crate::error::AppError> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title("Namehold is running")
        .body("Namehold is still running in the menu bar. Click the tray icon to reopen.")
        .show()
        .map_err(|e| crate::error::AppError::Other(e.to_string()))
}

/// Fire a native macOS notification on the first time the user closes the
/// window to tray. Checks the `tray_hint_shown` setting; if already "1",
/// returns early. Otherwise, shows the notification, persists the flag, and
/// returns Ok.
pub async fn fire_tray_hint_notification<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), crate::error::AppError> {
    let state = app.state::<crate::AppState>();
    let db = state
        .db
        .lock()
        .map_err(|e| crate::error::AppError::Lock(e.to_string()))?;

    let settings = crate::db::queries::get_settings(&db)?;
    if settings.get(SETTING_TRAY_HINT_SHOWN).map(|s| s.as_str()) == Some("1") {
        return Ok(());
    }

    #[cfg(not(test))]
    fire_hint_os_notification(app)?;

    crate::db::queries::set_setting(&db, SETTING_TRAY_HINT_SHOWN, "1")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Manager;

    /// Fresh in-memory DB with all migrations applied.
    fn migrated_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    /// Mock Tauri app managing an `AppState` backed by the given connection.
    fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
        mock_builder()
            .manage(AppState {
                db: std::sync::Mutex::new(conn),
                signer: std::sync::Mutex::new(None),
                secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
                hsd_child: std::sync::Mutex::new(None),
                node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
                sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                    crate::commands::sync::SyncStatus::default(),
                )),
            })
            .build(mock_context(noop_assets()))
            .expect("mock app")
    }

    /// First close-to-tray: the flag starts unset, so the function should run
    /// its body (OS notification is compiled out under cfg(test)) and persist
    /// `tray_hint_shown = "1"`.
    #[tokio::test]
    async fn fire_tray_hint_first_time_persists_flag() {
        let app = app_with(migrated_conn());

        fire_tray_hint_notification(app.handle())
            .await
            .expect("first call should succeed");

        let state = app.state::<AppState>();
        let db = state.db.lock().expect("db lock");
        let settings = crate::db::queries::get_settings(&db).expect("get_settings");
        assert_eq!(
            settings.get(SETTING_TRAY_HINT_SHOWN).map(|s| s.as_str()),
            Some("1"),
            "tray_hint_shown should be set to '1' after the first close-to-tray"
        );
    }

    /// Second (and later) close-to-tray: the flag is already "1", so the
    /// function should short-circuit and return Ok without touching the DB.
    #[tokio::test]
    async fn fire_tray_hint_second_time_is_noop() {
        let conn = migrated_conn();
        crate::db::queries::set_setting(&conn, SETTING_TRAY_HINT_SHOWN, "1").expect("pre-set flag");
        let app = app_with(conn);

        fire_tray_hint_notification(app.handle())
            .await
            .expect("second call should return Ok (no-op)");

        let state = app.state::<AppState>();
        let db = state.db.lock().expect("db lock");
        let settings = crate::db::queries::get_settings(&db).expect("get_settings");
        assert_eq!(
            settings.get(SETTING_TRAY_HINT_SHOWN).map(|s| s.as_str()),
            Some("1"),
            "tray_hint_shown should remain '1' on subsequent calls"
        );
    }

    // --- is_close_to_tray_enabled / set_close_to_tray_enabled ----------------

    #[tokio::test]
    async fn is_close_to_tray_enabled_defaults_to_true() {
        let app = app_with(migrated_conn());
        let state: tauri::State<'_, AppState> = app.state();
        let result = is_close_to_tray_enabled(state).await.unwrap();
        assert!(result, "default should be enabled (CLOSE_TO_TRAY_DEFAULT = '1')");
    }

    #[tokio::test]
    async fn is_close_to_tray_enabled_reads_explicit_false() {
        let conn = migrated_conn();
        crate::db::queries::set_setting(&conn, SETTING_CLOSE_TO_TRAY, "0").unwrap();
        let app = app_with(conn);
        let state: tauri::State<'_, AppState> = app.state();
        let result = is_close_to_tray_enabled(state).await.unwrap();
        assert!(!result, "should be disabled when setting is '0'");
    }

    #[tokio::test]
    async fn set_close_to_tray_enabled_persists_true() {
        let app = app_with(migrated_conn());
        let state: tauri::State<'_, AppState> = app.state();
        set_close_to_tray_enabled(state, true).await.unwrap();

        let st = app.state::<AppState>();
        let db = st.db.lock().unwrap();
        let settings = crate::db::queries::get_settings(&db).unwrap();
        assert_eq!(settings.get(SETTING_CLOSE_TO_TRAY).map(|s| s.as_str()), Some("1"));
    }

    #[tokio::test]
    async fn set_close_to_tray_enabled_persists_false() {
        let app = app_with(migrated_conn());
        let state: tauri::State<'_, AppState> = app.state();
        set_close_to_tray_enabled(state, false).await.unwrap();

        let st = app.state::<AppState>();
        let db = st.db.lock().unwrap();
        let settings = crate::db::queries::get_settings(&db).unwrap();
        assert_eq!(settings.get(SETTING_CLOSE_TO_TRAY).map(|s| s.as_str()), Some("0"));
    }

    // --- icon_kind (pure function) -------------------------------------------

    #[test]
    fn icon_kind_stopped_when_node_not_running() {
        let snap = TraySnapshot { node_running: false, node_synced: true, bg_sync_on: true };
        assert!(matches!(icon_kind(&snap), TrayIconKind::Stopped));
    }

    #[test]
    fn icon_kind_syncing_when_not_synced() {
        let snap = TraySnapshot { node_running: true, node_synced: false, bg_sync_on: true };
        assert!(matches!(icon_kind(&snap), TrayIconKind::Syncing));
    }

    #[test]
    fn icon_kind_normal_when_running_and_synced() {
        let snap = TraySnapshot { node_running: true, node_synced: true, bg_sync_on: true };
        assert!(matches!(icon_kind(&snap), TrayIconKind::Normal));
    }
}
