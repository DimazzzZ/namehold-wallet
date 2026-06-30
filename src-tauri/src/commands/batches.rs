use crate::db;
use crate::error::AppError;
use crate::models::batch::{Batch, BatchWithAssets};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_batches(state: State<'_, AppState>) -> Result<Vec<Batch>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::list_batches(&db)
}

#[tauri::command]
pub async fn get_batch_with_assets(
    state: State<'_, AppState>,
    id: i64,
) -> Result<BatchWithAssets, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_batch_with_assets(&db, id)
}

#[tauri::command]
pub async fn create_batch(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
    asset_ids: Vec<i64>,
) -> Result<i64, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::create_batch(&db, &name, description.as_deref(), &asset_ids)
}

#[tauri::command]
pub async fn update_batch(
    state: State<'_, AppState>,
    id: i64,
    name: Option<String>,
    description: Option<String>,
    status: Option<String>,
) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::update_batch(
        &db,
        id,
        name.as_deref(),
        description.as_deref(),
        status.as_deref(),
    )
}

#[tauri::command]
pub async fn delete_batch(state: State<'_, AppState>, id: i64) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::delete_batch(&db, id)
}

#[tauri::command]
pub async fn add_to_batch(
    state: State<'_, AppState>,
    batch_id: i64,
    asset_ids: Vec<i64>,
) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::add_to_batch(&db, batch_id, &asset_ids)
}

#[tauri::command]
pub async fn remove_from_batch(
    state: State<'_, AppState>,
    batch_id: i64,
    asset_ids: Vec<i64>,
) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::remove_from_batch(&db, batch_id, &asset_ids)
}
