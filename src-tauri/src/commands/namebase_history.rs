//! Commands for importing and querying Namebase account-history events.
//!
//! Two import sources:
//! 1. Live fetch — GET /api/account/history/export from Namebase (requires
//!    active session).
//! 2. File upload — user-provided CSV file (works offline).
//!
//! Both sources go through the same parser and upsert logic, so re-importing
//! is idempotent.

use std::fs;

use tauri::State;

use crate::db::namebase_history::{self, ImportHistoryResult, NamebaseHistorySummary};
use crate::error::AppError;
use crate::namebase::history::parse_history_csv;
use crate::AppState;

/// Import account history from a local CSV file. The user provides the path
/// (via Tauri dialog). Returns counts and summary.
#[tauri::command]
pub async fn import_namebase_history_from_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<ImportHistoryResult, AppError> {
    let csv_text = fs::read_to_string(&path)?;
    let events = parse_history_csv(&csv_text)?;

    let mut db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let result = namebase_history::upsert_events(&mut db, &events)?;

    // Audit log.
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_history_import_file', ?1)",
        [serde_json::json!({
            "path": path,
            "inserted": result.inserted,
            "updated": result.updated,
            "total": result.total,
        })
        .to_string()],
    )?;

    Ok(result)
}

/// Import account history from the live Namebase API. Requires an active
/// session (connected Namebase account). Returns counts and summary.
#[tauri::command]
pub async fn import_namebase_history_live(
    state: State<'_, AppState>,
) -> Result<ImportHistoryResult, AppError> {
    // Build the client (existing shared builder).
    let client = crate::commands::namebase::namebase_client(&state)?;
    let before = client.current_cookie();

    // Fetch the CSV export.
    let csv_text = client.get_account_history().await?;
    let events = parse_history_csv(&csv_text)?;

    // Upsert into the DB.
    let mut db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let result = namebase_history::upsert_events(&mut db, &events)?;

    // Persist any rotated cookie.
    crate::commands::namebase::persist_cookie_if_changed(&state, &before, &client)?;

    // Audit log.
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_history_import_live', ?1)",
        [serde_json::json!({
            "inserted": result.inserted,
            "updated": result.updated,
            "total": result.total,
        })
        .to_string()],
    )?;

    Ok(result)
}

/// List imported history rows with optional filters.
#[tauri::command]
pub fn get_namebase_history(
    state: State<'_, AppState>,
    name: Option<String>,
    family: Option<String>,
    search: Option<String>,
) -> Result<Vec<crate::db::namebase_history::NamebaseHistoryRow>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    namebase_history::list_history(&db, name.as_deref(), family.as_deref(), search.as_deref())
}

/// Get summary aggregates for the import card.
#[tauri::command]
pub fn get_namebase_history_summary(
    state: State<'_, AppState>,
) -> Result<NamebaseHistorySummary, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    namebase_history::summary(&db)
}

/// Clear all imported history (user can wipe and re-import).
#[tauri::command]
pub fn clear_namebase_history(state: State<'_, AppState>) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let removed = namebase_history::clear(&db)?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_history_clear', ?1)",
        [serde_json::json!({"removed": removed}).to_string()],
    )?;
    Ok(removed)
}
