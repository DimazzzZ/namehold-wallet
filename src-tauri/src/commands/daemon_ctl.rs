//! Daemon lifecycle commands: spawn, stop, and check status of `namehold-syncd`.

use crate::db;
use crate::error::AppError;
use crate::AppState;
use std::path::PathBuf;
use tauri::State;

/// The setting key for the background sync toggle.
pub const SETTING_BACKGROUND_SYNC: &str = "background_sync_enabled";

/// Default value for the background sync setting (enabled by default).
pub const BACKGROUND_SYNC_DEFAULT: &str = "1";

/// Name of the daemon binary (without platform extension).
const DAEMON_BIN_NAME: &str = "namehold-syncd";

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Check if background sync is enabled in settings.
#[tauri::command]
pub async fn is_background_sync_enabled(state: State<'_, AppState>) -> Result<bool, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    let enabled = settings
        .get(SETTING_BACKGROUND_SYNC)
        .unwrap_or(&BACKGROUND_SYNC_DEFAULT.to_string())
        == "1";
    Ok(enabled)
}

/// Enable or disable background sync. Spawns or stops the daemon accordingly.
#[tauri::command]
pub async fn set_background_sync_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), AppError> {
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::set_setting(
            &db,
            SETTING_BACKGROUND_SYNC,
            if enabled { "1" } else { "0" },
        )?;
    }

    if enabled {
        spawn_daemon()?;
    } else {
        stop_daemon()?;
    }

    Ok(())
}

/// Check if the daemon process is currently alive.
#[tauri::command]
pub async fn is_daemon_alive() -> Result<bool, AppError> {
    Ok(check_daemon_alive())
}

// ---------------------------------------------------------------------------
// Spawn / stop logic (called from commands and from app startup)
// ---------------------------------------------------------------------------

/// Spawn the daemon as a detached process that outlives the app.
pub fn spawn_daemon() -> Result<(), AppError> {
    // Don't spawn if already running.
    if check_daemon_alive() {
        return Ok(());
    }

    let daemon_path = find_daemon_binary()?;
    spawn_detached(&daemon_path)?;
    eprintln!("daemon_ctl: spawned {}", daemon_path.display());
    Ok(())
}

/// Stop the daemon by sending a signal to the PID in the PID file.
pub fn stop_daemon() -> Result<(), AppError> {
    let pid = match read_pid_file() {
        Some(p) => p,
        None => return Ok(()), // No PID file → daemon not running.
    };

    if !is_process_alive(pid) {
        // Process is already dead; clean up stale PID file.
        let _ = std::fs::remove_file(pid_file_path());
        return Ok(());
    }

    send_terminate(pid);

    // Wait up to 5 seconds for the daemon to exit.
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !is_process_alive(pid) {
            let _ = std::fs::remove_file(pid_file_path());
            eprintln!("daemon_ctl: stopped daemon (PID {})", pid);
            return Ok(());
        }
    }

    // Force kill if still alive.
    send_kill(pid);
    let _ = std::fs::remove_file(pid_file_path());
    eprintln!("daemon_ctl: force-killed daemon (PID {})", pid);
    Ok(())
}

/// Ensure the daemon is running if background sync is enabled.
/// Called on app startup.
pub fn ensure_daemon_if_enabled(settings: &std::collections::HashMap<String, String>) {
    let enabled = settings
        .get(SETTING_BACKGROUND_SYNC)
        .unwrap_or(&BACKGROUND_SYNC_DEFAULT.to_string())
        == "1";

    if enabled && !check_daemon_alive() {
        if let Err(e) = spawn_daemon() {
            eprintln!("daemon_ctl: failed to spawn daemon on startup: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// PID file management
// ---------------------------------------------------------------------------

fn pid_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".namehold")
        .join("syncd.pid")
}

fn read_pid_file() -> Option<u32> {
    let content = std::fs::read_to_string(pid_file_path()).ok()?;
    content.trim().parse().ok()
}

/// Check if the daemon is alive using the PID file.
pub fn check_daemon_alive() -> bool {
    match read_pid_file() {
        Some(pid) => is_process_alive(pid),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Platform-specific process operations
// ---------------------------------------------------------------------------

/// Find the daemon binary. Strategy:
/// 1. Sibling of the current executable (works in dev and bundled).
/// 2. In the same target directory (dev mode).
/// 3. In PATH (fallback).
fn find_daemon_binary() -> Result<PathBuf, AppError> {
    let bin_name = if cfg!(target_os = "windows") {
        format!("{DAEMON_BIN_NAME}.exe")
    } else {
        DAEMON_BIN_NAME.to_string()
    };

    // 1. Sibling of current executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(&bin_name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    // 2. Check if it's in PATH (dev mode: `cargo build` puts both bins in target/debug).
    if let Ok(output) = std::process::Command::new("which")
        .arg(&bin_name)
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    Err(AppError::Other(format!(
        "cannot find daemon binary '{bin_name}' — ensure it's built and in the app bundle"
    )))
}

/// Spawn a detached process that outlives the parent.
#[cfg(unix)]
fn spawn_detached(path: &PathBuf) -> Result<(), AppError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: setsid() creates a new session, making the child immune to
    // the parent's terminal signals (SIGHUP, etc.) and process group signals.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to spawn daemon: {e}")))?;

    // Drop the child handle immediately — we don't wait or reap it.
    // The daemon writes its own PID file.
    drop(child);
    Ok(())
}

/// Spawn a detached process that outlives the parent (Windows).
#[cfg(windows)]
fn spawn_detached(path: &PathBuf) -> Result<(), AppError> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    // CREATE_NEW_PROCESS_GROUP (0x200) + DETACHED_PROCESS (0x08)
    const FLAGS: u32 = 0x00000008 | 0x00000200;

    let child = Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(FLAGS)
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to spawn daemon: {e}")))?;

    drop(child);
    Ok(())
}

/// Check if a process with the given PID is alive.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    // kill(pid, 0) checks if the process exists without sending a signal.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use std::ptr;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == ptr::null_mut() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

/// Send SIGTERM to a process.
#[cfg(unix)]
fn send_terminate(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

#[cfg(windows)]
fn send_terminate(pid: u32) {
    // On Windows, there's no graceful SIGTERM equivalent for non-console apps.
    // We use TerminateProcess as the primary mechanism; the daemon's ctrlc
    // handler won't fire, but the PID file cleanup is best-effort anyway.
    send_kill(pid);
}

/// Force-kill a process.
#[cfg(unix)]
fn send_kill(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn send_kill(pid: u32) {
    use std::ptr;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle != ptr::null_mut() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}
