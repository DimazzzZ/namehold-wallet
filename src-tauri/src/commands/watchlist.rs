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
    pub tags: String,
}

/// Add a name to the watchlist. Idempotent (silently succeeds if already watched).
#[tauri::command]
pub fn add_to_watchlist(
    state: State<'_, AppState>,
    name: String,
    notes: Option<String>,
    tags: Option<String>,
) -> Result<(), AppError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::InvalidInput("name cannot be empty".into()));
    }
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO watched_names (name, notes, tags) VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET notes = excluded.notes, tags = excluded.tags",
        params![name, notes.unwrap_or_default(), tags.unwrap_or_default()],
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
    let mut stmt =
        db.prepare("SELECT name, added_at, notes, tags FROM watched_names ORDER BY added_at DESC")?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchedName {
            name: row.get(0)?,
            added_at: row.get(1)?,
            notes: row.get(2)?,
            tags: row.get(3)?,
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

// ---------------------------------------------------------------------------
// Bulk status + tag management
// ---------------------------------------------------------------------------

/// Per-name watchlist status (bulk response).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistStatus {
    pub name: String,
    pub watched: bool,
    pub tags: String,
    pub state: Option<String>,
    pub expiry: Option<u32>,
}

/// Bulk lookup: for each name in `names`, return whether it's watched and its
/// tags. The on-chain state fields (`state`, `expiry`) are populated from the
/// local `tracked_name_states` cache when available (i.e. when the name was
/// previously synced by this wallet); otherwise they're `None` and the
/// frontend should fall back to `read_name_info` for those names.
#[tauri::command]
pub fn get_watchlist_status(
    state: State<'_, AppState>,
    names: Vec<String>,
) -> Result<Vec<WatchlistStatus>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut out = Vec::with_capacity(names.len());
    for name in &names {
        let name = name.trim();
        // Check watchlist membership + tags.
        let row: Option<String> = db
            .query_row(
                "SELECT tags FROM watched_names WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .ok();
        let watched = row.is_some();
        let tags = row.unwrap_or_default();

        // Best-effort: check if we have cached state for this name from any
        // profile's sync. Watched names are usually NOT owned, so this will
        // often be None — the frontend fetches live state via read_name_info.
        let (ns_state, ns_renewal): (Option<String>, Option<u32>) = db
            .query_row(
                "SELECT state, renewal_height FROM tracked_name_states WHERE name = ?1 LIMIT 1",
                params![name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((None, None));

        out.push(WatchlistStatus {
            name: name.to_string(),
            watched,
            tags,
            state: ns_state,
            expiry: ns_renewal,
        });
    }
    Ok(out)
}

/// Update the tags for a watched name.
#[tauri::command]
pub fn update_watchlist_tags(
    state: State<'_, AppState>,
    name: String,
    tags: String,
) -> Result<(), AppError> {
    let name = name.trim().to_string();
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let updated = db.execute(
        "UPDATE watched_names SET tags = ?2 WHERE name = ?1",
        params![name, tags.trim()],
    )?;
    if updated == 0 {
        return Err(AppError::NotFound(format!("'{}' not in watchlist", name)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CSV import/export
// ---------------------------------------------------------------------------

/// Import result summary for CSV imports.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Escape a CSV cell: wrap in quotes if it contains a quote, comma, or newline;
/// double any internal quotes (standard CSV rules).
fn csv_escape(s: &str) -> String {
    let needs_quote = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
    if !needs_quote {
        return s.to_string();
    }
    let escaped = s.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

/// Split a single CSV row into fields, honoring quoted fields with embedded
/// commas and doubled-quote escaping.
fn csv_split_row(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = row.chars().peekable();
    let mut in_quote = false;
    while let Some(c) = chars.next() {
        if in_quote {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quote = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' && cur.is_empty() {
            in_quote = true;
        } else if c == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Export the current watchlist to a CSV file. Columns:
/// `name,tags,notes,added_at,state,expiry`. State/expiry are snapshotted from
/// the local cache at export time — they may be stale.
#[tauri::command]
pub fn export_watchlist_csv(state: State<'_, AppState>, path: String) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    // Join watched_names to tracked_name_states (best-effort — LEFT JOIN so
    // rows without cached state still export). We pick any profile's cached
    // row (LIMIT 1 per name via the subquery pattern below).
    let mut stmt = db.prepare(
        "SELECT w.name, w.tags, w.notes, w.added_at,
                (SELECT state FROM tracked_name_states t WHERE t.name = w.name LIMIT 1) AS state,
                (SELECT renewal_height FROM tracked_name_states t WHERE t.name = w.name LIMIT 1) AS expiry
         FROM watched_names w
         ORDER BY w.added_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<u32>>(5)?,
        ))
    })?;

    let mut out = String::from("name,tags,notes,added_at,state,expiry\n");
    let mut count = 0usize;
    for row in rows {
        let (name, tags, notes, added_at, state, expiry) = row?;
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_escape(&name),
            csv_escape(&tags),
            csv_escape(&notes),
            csv_escape(&added_at),
            csv_escape(state.as_deref().unwrap_or("")),
            expiry.map(|e| e.to_string()).unwrap_or_default(),
        ));
        count += 1;
    }
    std::fs::write(&path, out)?;
    Ok(count)
}

/// Import a CSV file into the watchlist. Accepts the header row emitted by
/// [`export_watchlist_csv`] (`name,tags,notes,added_at,state,expiry`). Only
/// `name`, `tags`, and `notes` are used on import — `added_at` is set to
/// `now`, `state`/`expiry` are ignored (re-fetched live). Existing rows are
/// preserved (INSERT OR IGNORE); duplicates count as `skipped`.
#[tauri::command]
pub fn import_watchlist_csv(
    state: State<'_, AppState>,
    path: String,
) -> Result<WatchlistImportResult, AppError> {
    let content = std::fs::read_to_string(&path)?;
    let mut lines = content.lines();
    // Skip header row if present.
    let header = lines.next().unwrap_or("");
    let header_fields = csv_split_row(header);
    let has_header = header_fields
        .first()
        .map(|f| f.eq_ignore_ascii_case("name"))
        .unwrap_or(false);
    let mut result = WatchlistImportResult {
        imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };
    // If the first line wasn't a header, treat it as data.
    let data_iter: Box<dyn Iterator<Item = &str>> = if has_header {
        Box::new(lines)
    } else {
        Box::new(std::iter::once(header).chain(lines))
    };

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    for (idx, line) in data_iter.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = csv_split_row(line);
        let name = fields.first().map(|s| s.trim()).unwrap_or("");
        if name.is_empty() {
            result.errors.push(format!("row {}: empty name", idx + 1));
            continue;
        }
        let tags = fields.get(1).map(|s| s.trim()).unwrap_or("");
        let notes = fields.get(2).map(|s| s.trim()).unwrap_or("");
        // INSERT OR IGNORE preserves existing rows; check rows-affected to
        // count imported vs skipped.
        let inserted = db.execute(
            "INSERT OR IGNORE INTO watched_names (name, notes, tags) VALUES (?1, ?2, ?3)",
            params![name, notes, tags],
        )?;
        if inserted == 1 {
            result.imported += 1;
        } else {
            result.skipped += 1;
        }
    }
    Ok(result)
}
