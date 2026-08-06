//! Read-only command to retrieve daemon-populated watched_name_states.
//! The Watchlist page uses this to seed columns (Countdown, Highest bid,
//! Expiry) without waiting for per-name RPC round-trips.

use crate::error::AppError;
use crate::AppState;
use serde::Serialize;
use tauri::State;

/// A row from `watched_name_states` — the daemon-written cache.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedNameStateRow {
    pub name: String,
    pub last_phase: Option<String>,
    pub last_state_json: Option<String>,
    pub last_highest_doos: Option<i64>,
    pub blocks_until_next: Option<i64>,
    pub polled_at: String,
}

/// Return all rows from `watched_name_states`. Lightweight — the daemon keeps
/// this table small (one row per watched name). The frontend uses
/// `last_state_json` to hydrate the full HsdName without an RPC call.
#[tauri::command]
pub fn get_watched_states(
    state: State<'_, AppState>,
) -> Result<Vec<WatchedNameStateRow>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut stmt = db.prepare(
        "SELECT name, last_phase, last_state_json, last_highest_doos, blocks_until_next, polled_at
         FROM watched_name_states
         ORDER BY polled_at DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WatchedNameStateRow {
                name: row.get(0)?,
                last_phase: row.get(1)?,
                last_state_json: row.get(2)?,
                last_highest_doos: row.get(3)?,
                blocks_until_next: row.get(4)?,
                polled_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
