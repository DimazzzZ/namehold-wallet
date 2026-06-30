use crate::db;
use crate::error::AppError;
use crate::models::asset::Asset;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_assets(
    state: State<'_, AppState>,
    status: Option<String>,
    is_staked: Option<bool>,
    search: Option<String>,
    sort_by: Option<String>,
    sort_dir: Option<String>,
) -> Result<Vec<Asset>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::list_assets(
        &db,
        status.as_deref(),
        is_staked,
        search.as_deref(),
        sort_by.as_deref(),
        sort_dir.as_deref(),
    )
}

#[tauri::command]
pub async fn get_asset(state: State<'_, AppState>, id: i64) -> Result<Asset, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_asset(&db, id)
}

#[tauri::command]
pub async fn update_asset(
    state: State<'_, AppState>,
    id: i64,
    status: Option<String>,
    category: Option<String>,
    tags: Option<String>,
    notes: Option<String>,
    hns_received: Option<i64>,
    transfer_tx_hash: Option<String>,
    finalize_tx_hash: Option<String>,
) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::update_asset(
        &db,
        id,
        status.as_deref(),
        category.as_deref(),
        tags.as_deref(),
        notes.as_deref(),
        hns_received,
        transfer_tx_hash.as_deref(),
        finalize_tx_hash.as_deref(),
    )
}

#[tauri::command]
pub async fn bulk_update_status(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    status: String,
) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::bulk_update_status(&db, &ids, &status)
}

#[tauri::command]
pub async fn bulk_update_tags(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    tags: String,
) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::bulk_update_tags(&db, &ids, &tags)
}

#[tauri::command]
pub async fn delete_asset(state: State<'_, AppState>, id: i64) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::delete_asset(&db, id)
}

#[tauri::command]
pub async fn get_dashboard_stats(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_dashboard_stats(&db)
}
