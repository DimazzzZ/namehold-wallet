use crate::db;
use crate::error::AppError;
use crate::namebase::client::NamebaseClient;
use crate::AppState;
use tauri::State;

fn get_cookie(state: &AppState) -> Result<String, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    Ok(settings.get("namebase_cookie").cloned().unwrap_or_default())
}

#[tauri::command]
pub async fn connect_namebase(
    state: State<'_, AppState>,
    cookie: String,
) -> Result<serde_json::Value, AppError> {
    let client = NamebaseClient::new(&cookie)?;
    let valid = client.check_session().await?;
    if !valid {
        return Err(AppError::Other("Invalid session cookie.".to_string()));
    }

    let account = client.get_account().await?;

    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::set_setting(&db, "namebase_cookie", &cookie)?;
        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('namebase_connect', ?1)",
            [serde_json::json!({"status": "connected"}).to_string()],
        )?;
    }

    Ok(account)
}

#[tauri::command]
pub async fn disconnect_namebase(state: State<'_, AppState>) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::set_setting(&db, "namebase_cookie", "")?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_disconnect', ?1)",
        [serde_json::json!({"status": "disconnected"}).to_string()],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn get_namebase_status(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;

    if cookie.is_empty() {
        return Ok(serde_json::json!({"connected": false, "has_cookie": false}));
    }

    let client = NamebaseClient::new(&cookie)?;
    match client.check_session().await {
        Ok(true) => {
            let account = client.get_account().await.ok();
            Ok(serde_json::json!({"connected": true, "has_cookie": true, "account": account}))
        }
        _ => Ok(serde_json::json!({"connected": false, "has_cookie": true, "error": "Session expired"})),
    }
}

#[tauri::command]
pub async fn fetch_namebase_domains(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;
    let client = NamebaseClient::new(&cookie)?;
    client.get_domains().await
}

#[tauri::command]
pub async fn fetch_namebase_staked(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;
    let client = NamebaseClient::new(&cookie)?;
    client.get_staked_domains().await
}

#[tauri::command]
pub async fn fetch_namebase_renewals(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;
    let client = NamebaseClient::new(&cookie)?;
    client.get_renewals().await
}

#[tauri::command]
pub async fn fetch_namebase_withdrawals(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;
    let client = NamebaseClient::new(&cookie)?;
    client.get_withdrawals().await
}

#[tauri::command]
pub async fn import_from_namebase(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;
    let client = NamebaseClient::new(&cookie)?;

    let domains = client.get_domains().await?;
    let staked_data = client.get_staked_domains().await?;

    let staked_names: std::collections::HashSet<String> = staked_data["stakedDomains"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d["name"].as_str().map(|s| s.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let mut imported = 0;
    let mut skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;

        if let Some(arr) = domains["domains"].as_array() {
            for domain in arr {
                let name = match domain["name"].as_str() {
                    Some(n) => n.to_lowercase().trim().to_string(),
                    None => { skipped += 1; continue; }
                };

                let is_staked = staked_names.contains(&name);
                let status = if is_staked { "do_not_touch_staked" } else { "not_started" };

                match db.execute(
                    "INSERT INTO assets (tld, is_staked, status, category, notes)
                     VALUES (?1, ?2, ?3, 'Namebase', 'Imported from Namebase')
                     ON CONFLICT(tld) DO UPDATE SET
                       is_staked = excluded.is_staked,
                       updated_at = datetime('now')",
                    rusqlite::params![name, if is_staked { 1 } else { 0 }, status],
                ) {
                    Ok(_) => imported += 1,
                    Err(e) => errors.push(format!("{}: {}", name, e)),
                }
            }
        }

        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('namebase_import', ?1)",
            [serde_json::json!({
                "imported": imported,
                "skipped": skipped,
                "errors": errors.len(),
                "staked_count": staked_names.len(),
            }).to_string()],
        )?;
    }

    Ok(serde_json::json!({
        "imported": imported,
        "skipped": skipped,
        "errors": errors,
        "staked_count": staked_names.len(),
    }))
}
