//! Watchlist CRUD commands: track names you don't own for monitoring.

use crate::error::AppError;
use crate::AppState;
use rusqlite::params;
use serde::Serialize;
use tauri::State;

/// A watched name with its metadata.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedName {
    pub name: String,
    pub added_at: String,
    pub notes: String,
}

/// Add a name to the watchlist. Idempotent (silently succeeds if already watched).
#[tauri::command]
pub fn add_to_watchlist(
    state: State<'_, AppState>,
    name: String,
    notes: Option<String>,
) -> Result<(), AppError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::InvalidInput("name cannot be empty".into()));
    }
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO watched_names (name, notes) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET notes = excluded.notes",
        params![name, notes.unwrap_or_default()],
    )?;
    Ok(())
}

/// Remove a name from the watchlist. Idempotent (silently succeeds if not watched).
#[tauri::command]
pub fn remove_from_watchlist(state: State<'_, AppState>, name: String) -> Result<(), AppError> {
    let name = name.trim().to_string();
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute("DELETE FROM watched_names WHERE name = ?1", params![name])?;
    Ok(())
}

/// List all watched names, newest first.
#[tauri::command]
pub fn list_watchlist(state: State<'_, AppState>) -> Result<Vec<WatchedName>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut stmt = db.prepare(
        "SELECT name, added_at, notes FROM watched_names ORDER BY added_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchedName {
            name: row.get(0)?,
            added_at: row.get(1)?,
            notes: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Check if a specific name is in the watchlist.
#[tauri::command]
pub fn is_watched(state: State<'_, AppState>, name: String) -> Result<bool, AppError> {
    let name = name.trim().to_string();
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM watched_names WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
