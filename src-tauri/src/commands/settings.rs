use crate::db;
use crate::error::AppError;
use crate::security;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut settings = db::queries::get_settings(&db)?;
    // Redact sensitive values: the renderer never needs the raw secret (it's
    // only consumed by backend commands). Emit a "__has_<key>": "true" marker
    // so the UI can still show "configured" without seeing the value.
    for &key in security::SENSITIVE_SETTING_KEYS {
        if settings.remove(key).is_some() {
            settings.insert(format!("__has_{key}"), "true".to_string());
        }
    }
    Ok(serde_json::to_value(&settings)?)
}

#[tauri::command]
pub async fn update_setting(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    // Reject renderer writes to security-critical keys (e.g. host overrides
    // that could redirect authenticated requests, or secrets that should only
    // be written by dedicated backend flows).
    if security::is_renderer_write_denied(&key) {
        return Err(AppError::InvalidInput(format!(
            "setting '{key}' cannot be changed from the UI"
        )));
    }
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::set_setting(&db, &key, &value)?;
    // Redact sensitive values in the audit log so secrets are never persisted
    // in plaintext outside the settings row itself.
    let logged_value = if security::is_sensitive_key(&key) {
        "***".to_string()
    } else {
        value
    };
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
        [serde_json::json!({"key": key, "value": logged_value}).to_string()],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let entries = db::queries::get_recent_audit_log(&db, limit.unwrap_or(20))?;
    // Defense-in-depth: redact any pre-existing sensitive values in audit log
    // entries that were written before the redaction was added.
    let redacted: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|mut entry| {
            if let Some(detail) = entry.get("detail").and_then(|d| d.as_str()) {
                if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(detail) {
                    if let Some(key) = parsed.get("key").and_then(|k| k.as_str()).map(String::from)
                    {
                        if security::is_sensitive_key(&key) {
                            parsed["value"] = serde_json::Value::String("***".into());
                            entry["detail"] = serde_json::Value::String(parsed.to_string());
                        }
                    }
                }
            }
            entry
        })
        .collect();
    Ok(serde_json::to_value(&redacted)?)
}

#[tauri::command]
pub async fn get_wallet_snapshots(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let snapshots = db::queries::get_wallet_snapshots(&db, limit.unwrap_or(10))?;
    Ok(serde_json::to_value(&snapshots)?)
}

#[cfg(test)]
mod tests {
    use crate::db;
    use crate::security;
    use rusqlite::Connection;

    /// Fresh in-memory DB with the minimal schema needed to exercise settings
    /// + audit_log — mirrors the app's migrations for those two tables.
    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                action TEXT NOT NULL,
                entity TEXT,
                entity_id INTEGER,
                detail TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );",
        )
        .unwrap();
        conn
    }

    /// Replicates the redaction logic in `get_settings` so it can be exercised
    /// without a Tauri `State` / async harness. If the command changes, this
    /// helper must be kept in lockstep — see the `#[test]` below.
    fn get_settings_redacted(conn: &Connection) -> serde_json::Value {
        let mut settings = db::queries::get_settings(conn).unwrap();
        for &key in security::SENSITIVE_SETTING_KEYS {
            if settings.remove(key).is_some() {
                settings.insert(format!("__has_{key}"), "true".to_string());
            }
        }
        serde_json::to_value(&settings).unwrap()
    }

    #[test]
    fn get_settings_hides_sensitive_values_and_marks_presence() {
        let conn = fresh_db();
        db::queries::set_setting(&conn, "namebase_cookie", "super-secret-session").unwrap();
        db::queries::set_setting(&conn, "node_rpc_api_key", "hunter2").unwrap();
        db::queries::set_setting(&conn, "advanced_mode", "true").unwrap();

        let out = get_settings_redacted(&conn);
        // Raw secrets never surface.
        assert!(out.get("namebase_cookie").is_none());
        assert!(out.get("node_rpc_api_key").is_none());
        // Presence markers are set so the UI can show "configured".
        assert_eq!(out["__has_namebase_cookie"], "true");
        assert_eq!(out["__has_node_rpc_api_key"], "true");
        // Non-sensitive keys pass through untouched.
        assert_eq!(out["advanced_mode"], "true");
    }

    #[test]
    fn get_settings_omits_markers_when_secret_unset() {
        let conn = fresh_db();
        db::queries::set_setting(&conn, "advanced_mode", "true").unwrap();
        let out = get_settings_redacted(&conn);
        assert!(out.get("__has_namebase_cookie").is_none());
        assert!(out.get("__has_node_rpc_api_key").is_none());
    }

    #[test]
    fn audit_log_records_redacted_value_for_sensitive_keys() {
        // Simulate what `update_setting` writes into audit_log for a sensitive
        // key: the persisted `detail` JSON must carry `"value": "***"`, never
        // the raw secret.
        let conn = fresh_db();
        let key = "node_rpc_api_key";
        let logged_value = if security::is_sensitive_key(key) {
            "***".to_string()
        } else {
            "leaked".to_string()
        };
        let detail = serde_json::json!({"key": key, "value": logged_value}).to_string();
        conn.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
            [detail],
        )
        .unwrap();
        let entries = db::queries::get_recent_audit_log(&conn, 10).unwrap();
        let stored = entries[0]["detail"].as_str().unwrap();
        assert!(stored.contains("\"***\""), "must redact: got {stored}");
        assert!(
            !stored.contains("leaked"),
            "raw value must not appear: {stored}"
        );
    }

    #[test]
    fn write_denylist_covers_namebase_base_url() {
        assert!(security::is_renderer_write_denied("namebase_base_url"));
        assert!(security::is_renderer_write_denied("namebase_cookie"));
        assert!(!security::is_renderer_write_denied("node_rpc_api_key"));
    }
}
