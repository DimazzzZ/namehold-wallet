//! Settings helpers for update notification opt-in.

use crate::db::queries;
use crate::AppState;
use tauri::State;

/// Check if update notifications are enabled.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn is_update_notify_enabled(state: State<'_, AppState>) -> bool {
    match state.db.lock() {
        Ok(db) => match queries::get_settings(&db) {
            Ok(settings) => {
                settings.get("update_notify_enabled").map(String::as_str) == Some("true")
            }
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Enable or disable update notifications.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn set_update_notify_enabled(enabled: bool, state: State<'_, AppState>) -> Result<(), String> {
    match state.db.lock() {
        Ok(db) => {
            let value = if enabled { "true" } else { "false" };
            queries::set_setting(&db, "update_notify_enabled", value)
                .map_err(|e| format!("Failed to set update notification setting: {e}"))
        }
        Err(e) => Err(format!("Database lock failed: {e}")),
    }
}
