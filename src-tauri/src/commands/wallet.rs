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
    let address = client.get_receive_address().await?;
    Ok(address)
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
pub async fn list_wallets(state: State<'_, AppState>) -> Result<Vec<String>, AppError> {
    let client = get_client(&state)?;
    let wallets = client.list_wallets().await?;
    Ok(wallets)
}

#[tauri::command]
pub async fn create_wallet(
    state: State<'_, AppState>,
    id: String,
    passphrase: String,
    mnemonic: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let result = client
        .create_wallet(&id, &passphrase, mnemonic.as_deref())
        .await?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('create_wallet', ?1)",
        [serde_json::json!({"wallet_id": id}).to_string()],
    )?;

    Ok(result)
}

#[tauri::command]
pub async fn get_mnemonic(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let url = format!(
        "{}/wallet/{}/master",
        client.wallet_url_for_master(),
        client.wallet_id_for_master()
    );
    let resp = client.http_get_master(&url).await?;
    Ok(resp)
}

#[tauri::command]
pub async fn delete_wallet(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, AppError> {
    // Soft delete: mark wallet for deletion in local audit log
    // Don't stop hsd or touch wallet database
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('wallet_hidden', ?1)",
        [serde_json::json!({"wallet_id": id}).to_string()],
    )?;

    Ok(format!(
        "Wallet '{}' hidden from list. The wallet still exists in hsd. \
         To fully delete it, stop hsd and remove the wallet database manually.",
        id
    ))
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

#[tauri::command]
pub async fn cancel_transfer(
    state: State<'_, AppState>,
    name: String,
    passphrase: String,
) -> Result<serde_json::Value, AppError> {
    check_write_mode(&state)?;

    let client = get_client(&state)?;
    let result = client.cancel_transfer(&name, &passphrase).await?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('cancel_transfer', ?1)",
        [serde_json::json!({"name": name}).to_string()],
    )?;

    Ok(result)
}

#[tauri::command]
pub async fn get_pending_transactions(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    Ok(client.get_pending_transactions().await?)
}

#[tauri::command]
pub async fn get_transaction(state: State<'_, AppState>, hash: String) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    Ok(client.get_transaction(&hash).await?)
}

#[tauri::command]
pub async fn get_coins(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    Ok(client.get_coins().await?)
}

#[tauri::command]
pub async fn lock_wallet(state: State<'_, AppState>) -> Result<(), AppError> {
    let client = get_client(&state)?;
    client.lock_wallet().await
}

#[tauri::command]
pub async fn unlock_wallet(state: State<'_, AppState>, passphrase: String) -> Result<(), AppError> {
    let client = get_client(&state)?;
    client.unlock_wallet(&passphrase).await
}

#[tauri::command]
pub async fn change_passphrase(state: State<'_, AppState>, old_pass: String, new_pass: String) -> Result<(), AppError> {
    check_write_mode(&state)?;

    let client = get_client(&state)?;
    client.change_passphrase(&old_pass, &new_pass).await
}

#[tauri::command]
pub async fn get_account(state: State<'_, AppState>, account: String) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    Ok(client.get_account(&account).await?)
}

#[tauri::command]
pub async fn validate_address(state: State<'_, AppState>, address: String) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    Ok(client.validate_address(&address).await?)
}

#[tauri::command]
pub async fn estimate_fee(state: State<'_, AppState>, blocks: u32) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    Ok(client.estimate_fee(blocks).await?)
}
