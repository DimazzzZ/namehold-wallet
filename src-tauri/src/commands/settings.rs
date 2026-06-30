use crate::db;
use crate::error::AppError;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    Ok(serde_json::to_value(&settings)?)
}

#[tauri::command]
pub async fn update_setting(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::set_setting(&db, &key, &value)?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('setting_change', ?1)",
        [serde_json::json!({"key": key, "value": value}).to_string()],
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
    Ok(serde_json::to_value(&entries)?)
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
