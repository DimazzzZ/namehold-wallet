//! Debug-only OS notification simulator.
//!
//! Fires the app's real OS notifications on demand, so both dispatch
//! libraries (`tauri-plugin-notification` for the deadline family, and
//! `notify-rust` for the watchlist family) can be verified visually on
//! the current OS without waiting for real chain / deadline conditions.
//!
//! The command is gated with `#[cfg(debug_assertions)]` and only
//! registered in dev builds (see `lib.rs`), so it does not exist at all
//! in release binaries.
//!
//! The preset title/body strings deliberately mirror the exact format
//! strings the real scanners produce (see `commands/deadlines.rs` and
//! `daemon/watched_names.rs`) so what the developer sees on-screen is
//! what a real user would see.

#![cfg(all(debug_assertions, not(test)))]

use crate::commands::deadlines::{send_os_notification, PendingNotification};
use crate::daemon::watched_names::emit_watchlist_os;
use crate::error::AppError;

/// Fire one preset OS notification for the requested `kind`.
///
/// Returns `Ok(Some(err))` when the OS dispatch reported a delivery
/// error (e.g. denied permission on macOS) — mirroring
/// `ScanOutcome::delivery_error` so the UI can surface it inline. An
/// unknown `kind` is rejected as `InvalidInput` rather than silently
/// no-oping.
#[tauri::command]
pub async fn simulate_notification<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kind: String,
) -> Result<Option<String>, AppError> {
    // Sample names / values chosen to be visually recognisable and to
    // match the shapes the real formatters produce (see referenced
    // source lines).
    match kind.as_str() {
        // --- Deadline family (Tauri plugin) ---------------------------
        // deadlines.rs:189-192
        "reveal" => Ok(send_os_notification(
            &app,
            &PendingNotification {
                key: "sim:reveal:example".into(),
                title: "Reveal window closing".into(),
                body: "example — reveal in 12 blocks or the bid lockup is forfeit".into(),
            },
        )),
        // deadlines.rs:207-208
        "renewal" => Ok(send_os_notification(
            &app,
            &PendingNotification {
                key: "sim:renewal:example".into(),
                title: "Renewal due soon".into(),
                body: "example — expires in 3.0 days".into(),
            },
        )),

        // --- Watchlist family (notify-rust) ---------------------------
        // All watchlist notifications share summary "Watchlist".
        // watched_names.rs:208
        "bidding" => Ok(emit_watchlist_os(
            "Watchlist",
            ".example — bidding is open now",
        )),
        // watched_names.rs:221
        "reopened" => Ok(emit_watchlist_os(
            "Watchlist",
            ".example — is available again for auction",
        )),
        // watched_names.rs:249-250 (100 blocks * 10min ≈ ~17h on mainnet)
        "bidding_soon" => Ok(emit_watchlist_os(
            "Watchlist",
            ".example — bidding opens in 100 blocks (~17h)",
        )),
        // watched_names.rs:272-273
        "highbid" => Ok(emit_watchlist_os(
            "Watchlist",
            ".example — highest bid crossed 5.0000 HNS (now 6.2500 HNS)",
        )),

        other => Err(AppError::InvalidInput(format!(
            "unknown simulate_notification kind: {other}"
        ))),
    }
}
