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
    use crate::security;

    #[test]
    fn write_denylist_covers_namebase_base_url() {
        assert!(security::is_renderer_write_denied("namebase_base_url"));
        assert!(security::is_renderer_write_denied("namebase_cookie"));
        assert!(!security::is_renderer_write_denied("node_rpc_api_key"));
    }
}
