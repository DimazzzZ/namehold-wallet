use crate::db;
use crate::error::AppError;
use crate::protected_settings::{self, SettingChange};
use crate::security;
use crate::AppState;
use tauri::{AppHandle, Runtime, State};

// COVERAGE: the `#[tauri::command]` wrappers are IPC shells (coverage off);
// their bodies are driven directly in `src/tests/settings_cmd_tests.rs` and,
// for the secure-window flow (`update_protected_settings_confirmed`), in
// `src/tests/protected_settings_tests.rs`.

#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut settings = db::queries::get_settings(&db)?;
    // Redact sensitive values: the renderer never needs the raw secret (it's
    // only consumed by backend commands). Emit a "__has_<key>": "true" marker
    // so the UI can still show "configured" without seeing the value.
    redact(&mut settings);
    Ok(serde_json::to_value(&settings)?)
}

/// Remove every sensitive value (`security::is_sensitive_key`, which also
/// covers the exchange's per-network bitcoind login) and mark it present.
pub(crate) fn redact(settings: &mut std::collections::HashMap<String, String>) {
    let keys: Vec<String> = settings
        .keys()
        .filter(|k| security::is_sensitive_key(k))
        .cloned()
        .collect();
    for key in keys {
        if settings.remove(&key).is_some_and(|v| !v.is_empty()) {
            settings.insert(format!("__has_{key}"), "true".to_string());
        }
    }
}

/// Write one setting the renderer may change ([`security::RENDERER_SETTING_KEYS`]).
/// Every other key is refused before the database is touched: a trust-root
/// key goes through [`update_protected_settings`], and the rest are written
/// only by backend flows. An allowlist, so a key added later is refused until
/// it is listed.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn update_setting(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    if !security::is_renderer_writable(&key) {
        return Err(AppError::InvalidInput(
            if security::is_protected_setting(&key) {
                format!(
                    "setting '{key}' is a connection setting: it is saved only after you confirm \
                 it in the secure window"
                )
            } else {
                format!("setting '{key}' cannot be changed from the UI")
            },
        ));
    }
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::set_setting(&db, &key, &value)?;
    log_setting_change(&db, &key, &value)
}

/// Record a setting change in the audit log, with a sensitive value redacted
/// so secrets are never persisted in plaintext outside the settings row.
pub(crate) fn log_setting_change(
    conn: &rusqlite::Connection,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    let logged_value = if security::is_sensitive_key(key) {
        "***"
    } else {
        value
    };
    conn.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
        [serde_json::json!({"key": key, "value": logged_value}).to_string()],
    )?;
    Ok(())
}

/// Change trust-root settings ([`security::PROTECTED_SETTING_KEYS`]): which
/// program runs as hsd, which node and explorer are trusted, where a node
/// api-key goes. See [`update_protected_settings_confirmed`].
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn update_protected_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    changes: Vec<SettingChange>,
) -> Result<Vec<String>, AppError> {
    update_protected_settings_confirmed(&app, &state, &changes).await
}

/// Validate every requested change ([`protected_settings::plan_changes`]);
/// when one changes the value in effect, show one row per change in the
/// secure window and, once the user confirms, write them all in one
/// transaction with an audit entry each. Returns the keys written: none, and
/// no prompt, when every value equals the one in effect. A declined prompt
/// is `UserRejected` and writes nothing; so is a request with any invalid
/// value. If the stored values change while the window is open, nothing is
/// written, since the rows the user confirmed no longer describe the change.
pub(crate) async fn update_protected_settings_confirmed<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    changes: &[SettingChange],
) -> Result<Vec<String>, AppError> {
    let stored = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_settings(&db)?
    };
    let planned = protected_settings::plan_changes(changes, &stored)?;
    if planned.is_empty() {
        return Ok(Vec::new());
    }
    crate::commands::secure_confirm::confirm_rows(
        app,
        protected_settings::CONFIRM_TITLE,
        protected_settings::CONFIRM_MESSAGE,
        protected_settings::confirmation_rows(&planned, &stored),
    )
    .await?;

    let mut db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let now = db::queries::get_settings(&db)?;
    if protected_settings::plan_changes(changes, &now)? != planned {
        return Err(AppError::InvalidInput(
            "these settings changed while the confirmation was open, so nothing was saved; \
             save again to see the change as it is now"
                .to_string(),
        ));
    }
    let tx = db.transaction()?;
    for c in &planned {
        db::queries::set_setting(&tx, &c.key, &c.new)?;
        log_setting_change(&tx, &c.key, &c.new)?;
    }
    tx.commit()?;
    Ok(planned.into_iter().map(|c| c.key).collect())
}

#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
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
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn get_wallet_snapshots(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let snapshots = db::queries::get_wallet_snapshots(&db, limit.unwrap_or(10))?;
    Ok(serde_json::to_value(&snapshots)?)
}
