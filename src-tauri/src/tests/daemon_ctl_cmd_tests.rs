//! Command-layer tests for `commands::daemon_ctl`.
//!
//! Safety note: `stop_daemon()` reads `~/.namehold/syncd.pid` from the real
//! home directory and, if present, signals that PID. If a developer has a
//! live `namehold-syncd` running while running tests, calling `stop_daemon`
//! (directly or via `set_background_sync_enabled(false)`) would SIGTERM
//! their real daemon. This test file deliberately AVOIDS every code path
//! that reaches `stop_daemon()` for that reason.
//!
//! What we DO exercise safely:
//! - `is_background_sync_enabled` — pure DB settings read.
//! - `is_daemon_alive` / `check_daemon_alive` — read the PID file but only
//!   probe liveness (`kill(pid, 0)` on unix); never send a real signal.
//! - `ensure_daemon_if_enabled` — pure branching on the settings hashmap.
//!   The enabled=true branch may call `spawn_daemon`, which in a test env
//!   fails to find the binary and returns Err — no side effects.
//! - `spawn_daemon` direct call — same story: fails to find binary.
//! - `set_background_sync_enabled(true)` — writes settings, then calls
//!   `spawn_daemon` which returns Err (no bundled binary). Safe.

use crate::commands::daemon_ctl::{
    check_daemon_alive, daemon_bin_name, ensure_daemon_if_enabled, find_binary_in_dirs,
    is_background_sync_enabled, is_daemon_alive, set_background_sync_enabled, spawn_daemon,
    BACKGROUND_SYNC_DEFAULT, RESOURCE_REL_DIRS, SETTING_BACKGROUND_SYNC,
};
use crate::db;
use crate::tests::command_helpers::create_test_state;
use crate::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

fn app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(state)
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

// --- is_background_sync_enabled ------------------------------------------

#[tokio::test]
async fn is_background_sync_enabled_uses_default_when_setting_absent() {
    let state = create_test_state();
    let app = app_with(state);

    let enabled = is_background_sync_enabled(app.state::<AppState>())
        .await
        .unwrap();
    // Default is BACKGROUND_SYNC_DEFAULT == "1" → true.
    assert_eq!(enabled, BACKGROUND_SYNC_DEFAULT == "1");
    assert!(enabled, "background sync defaults to on");
}

#[tokio::test]
async fn is_background_sync_enabled_true_when_setting_is_one() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::set_setting(&db, SETTING_BACKGROUND_SYNC, "1").unwrap();
    }
    let app = app_with(state);

    assert!(is_background_sync_enabled(app.state::<AppState>())
        .await
        .unwrap());
}

#[tokio::test]
async fn is_background_sync_enabled_false_when_setting_is_zero() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::set_setting(&db, SETTING_BACKGROUND_SYNC, "0").unwrap();
    }
    let app = app_with(state);

    assert!(!is_background_sync_enabled(app.state::<AppState>())
        .await
        .unwrap());
}

#[tokio::test]
async fn is_background_sync_enabled_false_when_setting_is_gibberish() {
    // Any value other than exactly "1" reads as disabled.
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::set_setting(&db, SETTING_BACKGROUND_SYNC, "yes").unwrap();
    }
    let app = app_with(state);

    assert!(!is_background_sync_enabled(app.state::<AppState>())
        .await
        .unwrap());
}

// --- is_daemon_alive / check_daemon_alive --------------------------------
// These read ~/.namehold/syncd.pid but never send a real signal (kill(pid,0)
// on unix is a permission check, not a signal delivery). Safe in all envs.

#[tokio::test]
async fn is_daemon_alive_returns_bool_without_panic() {
    let alive = is_daemon_alive().await.unwrap();
    // We can't assert a specific value: it depends on whether the developer
    // has a live daemon. We only assert the call itself completes.
    let _: bool = alive;
}

#[test]
fn check_daemon_alive_returns_bool_without_panic() {
    let alive = check_daemon_alive();
    let _: bool = alive;
}

// --- ensure_daemon_if_enabled --------------------------------------------

#[test]
fn ensure_daemon_if_enabled_skips_when_disabled() {
    // enabled=false → early return, never calls spawn_daemon. Purely
    // observable-by-not-panicking; and it never touches the PID file.
    let mut settings = HashMap::new();
    settings.insert(SETTING_BACKGROUND_SYNC.to_string(), "0".to_string());
    ensure_daemon_if_enabled(&settings);
}

#[test]
fn ensure_daemon_if_enabled_defaults_to_enabled_when_setting_absent() {
    // No setting present → BACKGROUND_SYNC_DEFAULT="1" fallback → enabled.
    // Will attempt spawn_daemon; find_daemon_binary fails in test env; the
    // Err is caught and logged. Safe — no signals, no state change.
    let settings: HashMap<String, String> = HashMap::new();
    ensure_daemon_if_enabled(&settings);
}

#[test]
fn ensure_daemon_if_enabled_attempts_spawn_when_enabled() {
    // enabled=true and daemon-not-alive → spawn_daemon → find_daemon_binary
    // → Err → logged and swallowed. If the developer HAS a daemon running,
    // check_daemon_alive() short-circuits and spawn isn't attempted. Either
    // way, safe and asserted by not panicking.
    let mut settings = HashMap::new();
    settings.insert(SETTING_BACKGROUND_SYNC.to_string(), "1".to_string());
    ensure_daemon_if_enabled(&settings);
}

// --- spawn_daemon direct -------------------------------------------------
// spawn_daemon() short-circuits with Ok if the daemon is already alive. In
// a normal test env it proceeds to find_daemon_binary() which returns Err
// because the sidecar isn't next to the test binary and (usually) isn't in
// PATH. We accept either outcome.

#[test]
fn spawn_daemon_either_short_circuits_or_errors_cleanly() {
    match spawn_daemon() {
        Ok(()) => {
            // Developer's daemon is alive; check_daemon_alive short-circuited.
        }
        Err(_) => {
            // find_daemon_binary failed as expected in test env. This is the
            // common path and the one that gives us coverage of the full
            // find_daemon_binary fallback chain.
        }
    }
}

// --- set_background_sync_enabled(true) -----------------------------------
// Writes the DB setting, then calls spawn_daemon. spawn_daemon is safe in
// test env (either no-op if a real daemon is running, or Err from missing
// binary). Set to true only — set_background_sync_enabled(false) would
// call stop_daemon which may SIGTERM the developer's real daemon.

#[tokio::test]
async fn set_background_sync_enabled_true_persists_setting() {
    let state = create_test_state();
    // Move the mutex-guarded conn read out of scope before invoking the
    // command (which also locks the DB).
    let app = app_with(state);

    // spawn_daemon may Err in test env; the command surfaces that Err.
    // We only care about the setting write, which happens BEFORE spawn.
    let _ = set_background_sync_enabled(app.state::<AppState>(), true).await;

    let state_handle = app.state::<AppState>();
    let db = state_handle.db.lock().unwrap();
    let settings = db::queries::get_settings(&db).unwrap();
    assert_eq!(
        settings.get(SETTING_BACKGROUND_SYNC).map(String::as_str),
        Some("1"),
        "enabled=true must persist '1' to settings even if spawn errored"
    );
}

// --- find_binary_in_dirs / daemon_bin_name -------------------------------
// These are pure fs / cfg helpers extracted from `find_daemon_binary` so
// the search logic is testable without depending on the ambient exe path
// or a `which` shell-out. The remaining thin orchestration in
// `find_daemon_binary` (env::current_exe + `which`) is annotated
// `coverage(off)` because it is environment-dependent.

/// Create a fresh empty temp dir for a single test. Using `std::env::temp_dir`
/// directly per project convention (no `tempfile` dev-dep).
fn fresh_tmp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("namehold_daemon_ctl_{tag}_{pid}_{nanos}_{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn daemon_bin_name_matches_platform_suffix() {
    let name = daemon_bin_name();
    if cfg!(target_os = "windows") {
        assert!(
            name.ends_with(".exe"),
            "windows binary must have .exe suffix, got {name}"
        );
    } else {
        assert!(
            !name.ends_with(".exe"),
            "non-windows binary must not have .exe suffix, got {name}"
        );
    }
    assert!(
        name.starts_with("namehold-syncd"),
        "binary name must start with 'namehold-syncd', got {name}"
    );
}

#[test]
fn resource_rel_dirs_are_non_empty() {
    // Guardrail: the resource-dir list is what the Linux-bundle fallback
    // walks. If someone accidentally empties it, coverage on step 3 of
    // `find_daemon_binary` silently drops to zero. Assert it stays populated.
    assert!(
        !RESOURCE_REL_DIRS.is_empty(),
        "RESOURCE_REL_DIRS must list at least one bundle-relative resource dir"
    );
    for rel in RESOURCE_REL_DIRS {
        assert!(!rel.is_empty(), "empty entry in RESOURCE_REL_DIRS");
    }
}

#[test]
fn find_binary_in_dirs_returns_none_when_no_dirs() {
    // Empty search list → never finds anything.
    let hit = find_binary_in_dirs("namehold-syncd", &[]);
    assert!(hit.is_none());
}

#[test]
fn find_binary_in_dirs_returns_none_when_binary_absent() {
    // Existing dir, but binary not present.
    let dir = fresh_tmp_dir("absent");
    let hit = find_binary_in_dirs("namehold-syncd", std::slice::from_ref(&dir));
    assert!(
        hit.is_none(),
        "empty dir must produce no hit; got {hit:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_binary_in_dirs_finds_binary_in_first_dir() {
    // Place a fake binary in a temp dir; the search returns it.
    let dir = fresh_tmp_dir("hit_first");
    let bin = dir.join("namehold-syncd");
    std::fs::write(&bin, b"#!/bin/sh\nexit 0\n").expect("write fake bin");

    let hit = find_binary_in_dirs("namehold-syncd", std::slice::from_ref(&dir))
        .expect("expected to find binary");
    assert_eq!(hit, bin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_binary_in_dirs_skips_missing_dirs_and_finds_later_match() {
    // First dir doesn't exist; second is empty; third holds the binary.
    // Exercises the loop's "keep going after miss" behavior.
    let nonexistent = std::env::temp_dir().join(format!(
        "namehold_daemon_ctl_ghost_{}",
        std::process::id()
    ));
    // Ensure it really doesn't exist.
    let _ = std::fs::remove_dir_all(&nonexistent);

    let empty = fresh_tmp_dir("skip_empty");
    let with_bin = fresh_tmp_dir("skip_hit");
    let bin = with_bin.join("namehold-syncd");
    std::fs::write(&bin, b"stub").expect("write fake bin");

    let dirs = vec![nonexistent.clone(), empty.clone(), with_bin.clone()];
    let hit = find_binary_in_dirs("namehold-syncd", &dirs).expect("expected match in third dir");
    assert_eq!(hit, bin);

    let _ = std::fs::remove_dir_all(&empty);
    let _ = std::fs::remove_dir_all(&with_bin);
}

#[test]
fn find_binary_in_dirs_returns_first_match_when_multiple_present() {
    // Priority: the earlier dir wins even if a later dir also has the binary.
    let first = fresh_tmp_dir("first");
    let second = fresh_tmp_dir("second");
    let first_bin = first.join("namehold-syncd");
    let second_bin = second.join("namehold-syncd");
    std::fs::write(&first_bin, b"first").expect("write first");
    std::fs::write(&second_bin, b"second").expect("write second");

    let hit = find_binary_in_dirs("namehold-syncd", &[first.clone(), second.clone()])
        .expect("expected first-dir hit");
    assert_eq!(hit, first_bin);

    let _ = std::fs::remove_dir_all(&first);
    let _ = std::fs::remove_dir_all(&second);
}

#[test]
fn find_binary_in_dirs_honors_bin_name_arg() {
    // Same dir, different filename asked for → no hit.
    let dir = fresh_tmp_dir("wrong_name");
    std::fs::write(dir.join("some-other-binary"), b"stub").expect("write stub");

    let hit = find_binary_in_dirs("namehold-syncd", std::slice::from_ref(&dir));
    assert!(
        hit.is_none(),
        "wrong filename must not match; got {hit:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_binary_in_dirs_finds_platform_suffixed_name() {
    // Round-trip: the actual name used by find_daemon_binary resolves
    // to a file we place under a temp dir.
    let dir = fresh_tmp_dir("suffix");
    let name = daemon_bin_name();
    let bin = dir.join(&name);
    std::fs::write(&bin, b"stub").expect("write fake bin");

    let hit = find_binary_in_dirs(&name, std::slice::from_ref(&dir)).expect("expected match");
    assert_eq!(hit, bin);
    let _ = std::fs::remove_dir_all(&dir);
}
