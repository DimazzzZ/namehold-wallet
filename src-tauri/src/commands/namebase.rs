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

/// Build a Namebase client, honoring an optional `namebase_base_url` setting so
/// tests can point the irreversible transfer/withdraw calls at a mock server.
/// Production leaves the setting unset → the real Namebase host.
pub(crate) fn namebase_client(state: &AppState) -> Result<NamebaseClient, AppError> {
    let (cookie, base) = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        (
            settings.get("namebase_cookie").cloned().unwrap_or_default(),
            settings.get("namebase_base_url").cloned().unwrap_or_default(),
        )
    };
    if base.trim().is_empty() {
        NamebaseClient::new(&cookie)
    } else {
        NamebaseClient::with_base_url(&cookie, base.trim())
    }
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
    // Use the shared client builder so the `namebase_base_url` test seam applies
    // (and so this honors any future base-url override), unlike the other fetch_*
    // commands which hard-code the real host.
    let client = namebase_client(&state)?;
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

#[tauri::command]
pub async fn namebase_transfer_domain(
    state: State<'_, AppState>,
    name: String,
    address: String,
) -> Result<(), AppError> {
    // Validate the destination FIRST — a Namebase withdrawal is irreversible, so a
    // malformed or wrong-network address would lose the domain. Reuse the same
    // address validator the Send flow uses. This fails fast (no cookie needed).
    let address = address.trim().to_string();
    let network = active_profile_network(&state);
    crate::noncustodial::address::decode(network, &address).map_err(|_| {
        AppError::InvalidInput(format!(
            "destination is not a valid {} HNS address",
            network.as_str()
        ))
    })?;

    let client = namebase_client(&state)?;
    client.transfer_domain(&name, &address).await?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_transfer', ?1)",
        [serde_json::json!({"name": name, "address": address}).to_string()],
    )?;
    // Reflect the initiated transfer in the inventory so the domain shows
    // "transfer requested" (and the Transfers view / inventory badge light up).
    db::queries::set_asset_status_by_tld(&db, &name, "namebase_transfer_requested")?;

    Ok(())
}

/// The active profile's network, defaulting to mainnet (Namebase domains are
/// mainnet HNS) when there is no active profile or it can't be parsed.
fn active_profile_network(state: &AppState) -> crate::noncustodial::network::Network {
    use crate::noncustodial::network::Network;
    let conn = match state.db.lock() {
        Ok(c) => c,
        Err(_) => return Network::Main,
    };
    let id = db::queries::get_active_profile_id(&conn).unwrap_or_default();
    if id.is_empty() {
        return Network::Main;
    }
    match db::queries::get_wallet_profile(&conn, &id) {
        Ok(Some(p)) => {
            crate::noncustodial::derivation::network_from_profile(&p.network).unwrap_or(Network::Main)
        }
        _ => Network::Main,
    }
}

#[tauri::command]
pub async fn namebase_withdraw_hns(
    state: State<'_, AppState>,
    address: String,
    amount: String,
) -> Result<(), AppError> {
    // Validate FIRST — a Namebase withdrawal is irreversible. Reuse the Send
    // flow's address validator; require a positive integer amount (doos).
    let address = address.trim().to_string();
    let network = active_profile_network(&state);
    crate::noncustodial::address::decode(network, &address).map_err(|_| {
        AppError::InvalidInput(format!(
            "destination is not a valid {} HNS address",
            network.as_str()
        ))
    })?;
    // Namebase amounts are in HNS (decimal), e.g. "1" or "1.5".
    match amount.trim().parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => {}
        _ => {
            return Err(AppError::InvalidInput(
                "amount must be a positive number of HNS".to_string(),
            ))
        }
    }

    let client = namebase_client(&state)?;
    client.withdraw_hns(&address, amount.trim()).await?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_withdraw_hns', ?1)",
        [serde_json::json!({"address": address, "amount": amount}).to_string()],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn fetch_namebase_domain_withdrawals(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let cookie = get_cookie(&state)?;
    let client = NamebaseClient::new(&cookie)?;
    client.get_domain_withdrawals().await
}
