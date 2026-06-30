use crate::db;
use crate::error::AppError;
use crate::hsd::client::HandshakeClient;
use crate::AppState;
use tauri::State;

fn get_client(state: &AppState) -> Result<HandshakeClient, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    Ok(HandshakeClient::from_settings(&settings))
}

fn check_write_mode(state: &AppState) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    let write_mode = settings.get("write_mode").map(|s| s.as_str()).unwrap_or("false");
    if write_mode != "true" {
        return Err(AppError::Other("Write mode is disabled. Enable it in Settings to perform write actions.".to_string()));
    }
    Ok(())
}

#[tauri::command]
pub async fn check_connection(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    match client.check_connection().await {
        Ok(info) => Ok(serde_json::json!({
            "connected": true,
            "info": serde_json::to_value(&info).unwrap_or_default()
        })),
        Err(e) => Ok(serde_json::json!({
            "connected": false,
            "error": e.to_string()
        })),
    }
}

#[tauri::command]
pub async fn get_wallet_info(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let info = client.get_wallet_info().await?;
    Ok(serde_json::to_value(&info)?)
}

#[tauri::command]
pub async fn get_balance(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let balance = client.get_balance().await?;
    Ok(serde_json::to_value(&balance)?)
}

#[tauri::command]
pub async fn get_address(state: State<'_, AppState>) -> Result<String, AppError> {
    let client = get_client(&state)?;
    let addr = client.get_receive_address().await?;
    Ok(addr.address)
}

#[tauri::command]
pub async fn get_names(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let names = client.get_names().await?;
    Ok(serde_json::to_value(&names)?)
}

#[tauri::command]
pub async fn get_name_info(
    state: State<'_, AppState>,
    name: String,
) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let info = client.get_name_info(&name).await?;
    Ok(serde_json::to_value(&info)?)
}

#[tauri::command]
pub async fn get_resource(
    state: State<'_, AppState>,
    name: String,
) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let resource = client.get_resource(&name).await?;
    Ok(resource)
}

#[tauri::command]
pub async fn get_transactions(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let txs = client.get_transactions().await?;
    Ok(txs)
}

#[tauri::command]
pub async fn send_hns(
    state: State<'_, AppState>,
    address: String,
    value: i64,
    passphrase: String,
) -> Result<serde_json::Value, AppError> {
    check_write_mode(&state)?;
    let client = get_client(&state)?;
    let result = client.send_to_address(&address, value, &passphrase).await?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('send_hns', ?1)",
        [serde_json::json!({"address": address, "value": value}).to_string()],
    )?;

    Ok(result)
}

#[tauri::command]
pub async fn transfer_name(
    state: State<'_, AppState>,
    name: String,
    address: String,
    passphrase: String,
) -> Result<serde_json::Value, AppError> {
    check_write_mode(&state)?;
    let client = get_client(&state)?;
    let result = client.send_transfer(&name, &address, &passphrase).await?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('transfer_name', ?1)",
        [serde_json::json!({"name": name, "address": address}).to_string()],
    )?;

    Ok(result)
}
