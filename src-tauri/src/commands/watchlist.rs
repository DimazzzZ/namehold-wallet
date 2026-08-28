//! Watchlist CRUD commands: track names you don't own for monitoring.

// COVERAGE: #[tauri::command] macro attribute lines are structurally uncoverable.

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

// ---------------------------------------------------------------------------
// Unit tests for private pure helpers (`csv_escape`, `csv_split_row`).
//
// The Tauri-command surface is tested in `src/tests/watchlist_cmd_tests.rs`
// (which cannot reach these private helpers). This inline module pins the
// branch behaviour of each helper directly, exhaustively covering:
//   * every `needs_quote` predicate branch in `csv_escape`
//   * every quoting-state transition in `csv_split_row`, including
//     doubled-quote escapes, embedded commas, empty and trailing fields,
//     bare quotes in the middle of an unquoted field (treated as literal),
//     and an unbalanced trailing quote (graceful fallback: consumed input
//     ends up in the final field).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod pure_helper_tests {
    use super::{csv_escape, csv_split_row};

    // --- csv_escape --------------------------------------------------------

    #[test]
    fn csv_escape_passthrough_when_no_special_chars() {
        // No comma, no quote, no newline, no CR → returned verbatim.
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("with spaces and.dots"), "with spaces and.dots");
        assert_eq!(csv_escape("café-utf8"), "café-utf8");
    }

    #[test]
    fn csv_escape_empty_string_is_passthrough() {
        // Empty string has no special chars → returned as-is (not quoted).
        assert_eq!(csv_escape(""), "");
    }

    #[test]
    fn csv_escape_wraps_comma() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
    }

    #[test]
    fn csv_escape_wraps_and_doubles_quote() {
        // Internal quote must be doubled, then the whole cell wrapped.
        assert_eq!(csv_escape("he said \"hi\""), "\"he said \"\"hi\"\"\"");
        // A cell that is a single quote character.
        assert_eq!(csv_escape("\""), "\"\"\"\"");
    }

    #[test]
    fn csv_escape_wraps_newline() {
        assert_eq!(csv_escape("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn csv_escape_wraps_carriage_return() {
        // The `\r` branch of the `needs_quote` predicate.
        assert_eq!(csv_escape("line1\rline2"), "\"line1\rline2\"");
    }

    #[test]
    fn csv_escape_all_special_chars_at_once() {
        // Combines every triggering branch and the internal-quote doubling.
        let out = csv_escape("a,\"b\"\nc\rd");
        assert_eq!(out, "\"a,\"\"b\"\"\nc\rd\"");
    }

    // --- csv_split_row -----------------------------------------------------

    #[test]
    fn csv_split_row_simple_comma_separated() {
        assert_eq!(
            csv_split_row("a,b,c"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn csv_split_row_empty_input_yields_one_empty_field() {
        // The trailing `out.push(cur)` after the loop guarantees at least
        // one field even for an empty row.
        assert_eq!(csv_split_row(""), vec!["".to_string()]);
    }

    #[test]
    fn csv_split_row_single_field_no_commas() {
        assert_eq!(csv_split_row("solo"), vec!["solo".to_string()]);
    }

    #[test]
    fn csv_split_row_trailing_empty_field() {
        // Trailing comma → a final empty field is pushed.
        assert_eq!(
            csv_split_row("a,b,"),
            vec!["a".to_string(), "b".to_string(), "".to_string()]
        );
    }

    #[test]
    fn csv_split_row_consecutive_commas_yield_empty_middle_fields() {
        assert_eq!(
            csv_split_row("a,,b"),
            vec!["a".to_string(), "".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn csv_split_row_quoted_field_preserves_content() {
        // A quoted field keeps its inner value; the surrounding quotes are
        // stripped by the state machine.
        assert_eq!(
            csv_split_row("\"hello\",world"),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn csv_split_row_embedded_comma_inside_quoted_field() {
        // Commas inside quotes must NOT split the row.
        assert_eq!(
            csv_split_row("\"a,b\",c"),
            vec!["a,b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn csv_split_row_doubled_quote_inside_quoted_field() {
        // `""` inside a quoted field decodes to a single `"`.
        assert_eq!(
            csv_split_row("\"he said \"\"hi\"\"\",tail"),
            vec!["he said \"hi\"".to_string(), "tail".to_string()]
        );
    }

    #[test]
    fn csv_split_row_quote_transitions_out_and_back_in_unquoted_context() {
        // After a closing quote (in_quote flips false), a following char that
        // is not a comma is appended to `cur` via the trailing `else` arm.
        // `"x"y,z` → cur becomes `xy` after the closing quote's peek-branch,
        // then `y` from the unquoted branch (`c == '"' && cur.is_empty()` is
        // false because cur == "x"), then split at comma.
        assert_eq!(
            csv_split_row("\"x\"y,z"),
            vec!["xy".to_string(), "z".to_string()]
        );
    }

    #[test]
    fn csv_split_row_bare_quote_mid_unquoted_field_is_literal() {
        // When cur is non-empty and we hit `"`, neither the "start quote"
        // arm nor the "comma" arm matches → the quote is appended literally
        // via the trailing `else` arm.
        assert_eq!(
            csv_split_row("ab\"cd,ef"),
            vec!["ab\"cd".to_string(), "ef".to_string()]
        );
    }

    #[test]
    fn csv_split_row_unbalanced_quote_falls_back_gracefully() {
        // No terminating quote: the parser stays in_quote until EOF and
        // returns whatever it accumulated as the last field — no panic.
        assert_eq!(
            csv_split_row("\"unterminated,still here"),
            vec!["unterminated,still here".to_string()]
        );
    }

    #[test]
    fn csv_split_row_empty_quoted_field() {
        // A pair of quotes with nothing inside → empty field.
        assert_eq!(
            csv_split_row("\"\",b"),
            vec!["".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn csv_split_row_quoted_field_with_newline_inside() {
        // Newlines are just data inside a quoted field.
        assert_eq!(
            csv_split_row("\"line1\nline2\",tail"),
            vec!["line1\nline2".to_string(), "tail".to_string()]
        );
    }

    // --- round-trip contract: escape then split reverses cleanly -----------

    #[test]
    fn csv_escape_then_split_round_trip_preserves_cells() {
        // Assembling a single-row CSV from escaped cells and re-splitting it
        // must yield the original vector, even when cells contain commas,
        // quotes, and newlines.
        let cells = [
            "plain",
            "",
            "with,comma",
            "with \"quote\"",
            "with\nnewline",
            "café",
        ];
        let row = cells
            .iter()
            .map(|c| csv_escape(c))
            .collect::<Vec<_>>()
            .join(",");
        let parsed = csv_split_row(&row);
        assert_eq!(
            parsed,
            cells.iter().map(|c| c.to_string()).collect::<Vec<_>>()
        );
    }
}
