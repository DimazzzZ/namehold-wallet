use crate::db;
use crate::error::AppError;
use crate::hsd::client::HandshakeClient;
use crate::AppState;
use std::collections::HashSet;
use tauri::State;

#[tauri::command]
pub async fn sync_names(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let (client, settings, assets) = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        let assets = db::queries::list_assets(&db, None, None, None, None, None)?;
        (HandshakeClient::from_settings(&settings), settings, assets)
    };

    let wallet_names = client.get_names().await?;
    let balance = client.get_balance().await.ok();
    let address = client.get_receive_address().await.ok();

    let tld_set: HashSet<String> = assets.iter().map(|a| a.tld.clone()).collect();
    let mut matched = 0usize;
    let mut errors: Vec<String> = Vec::new();

    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for name in &wallet_names {
            let tld = name.name.trim_start_matches('.').to_lowercase();
            if tld_set.contains(&tld) {
                let expires = name
                    .stats
                    .as_ref()
                    .and_then(|s| s.renewal_period_end)
                    .map(|v| v as i64);
                let days = name
                    .stats
                    .as_ref()
                    .and_then(|s| s.days_until_expire);
                let state_str = name.state.as_deref().unwrap_or("unknown");

                if let Err(e) = db.execute(
                    "UPDATE assets SET
                        status = 'finalized_owned',
                        name_state = ?1,
                        expires_at_height = ?2,
                        days_until_expire = ?3,
                        last_synced_at = datetime('now'),
                        updated_at = datetime('now')
                     WHERE tld = ?4",
                    rusqlite::params![state_str, expires, days, tld],
                ) {
                    errors.push(format!("{}: {}", tld, e));
                } else {
                    matched += 1;
                }
            }
        }

        let wallet_id = settings
            .get("hsd_wallet_id")
            .cloned()
            .unwrap_or_else(|| "primary".to_string());

        if let Some(bal) = balance {
            db::queries::insert_wallet_snapshot(
                &db,
                &wallet_id,
                bal.confirmed,
                address.as_deref(),
                wallet_names.len() as i64,
                None,
            )?;
        }

        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('sync', ?1)",
            [serde_json::json!({
                "matched": matched,
                "wallet_names": wallet_names.len(),
                "errors": errors
            })
            .to_string()],
        )?;
    }

    let extra_names: Vec<String> = wallet_names
        .iter()
        .map(|n| n.name.trim_start_matches('.').to_lowercase())
        .filter(|n| !tld_set.contains(n))
        .collect();

    let wallet_tlds: HashSet<String> = wallet_names
        .iter()
        .map(|n| n.name.trim_start_matches('.').to_lowercase())
        .collect();
    let missing_names: Vec<String> = tld_set
        .difference(&wallet_tlds)
        .cloned()
        .collect();

    Ok(serde_json::json!({
        "matched": matched,
        "wallet_count": wallet_names.len(),
        "extra_count": extra_names.len(),
        "extra_names": extra_names,
        "missing_count": missing_names.len(),
        "missing_names": missing_names,
        "errors": errors,
    }))
}

#[tauri::command]
pub async fn get_sync_report(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let (client, assets) = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        let assets = db::queries::list_assets(&db, None, None, None, None, None)?;
        (HandshakeClient::from_settings(&settings), assets)
    };

    let wallet_names = client.get_names().await?;
    let wallet_tlds: HashSet<String> = wallet_names
        .iter()
        .map(|n| n.name.trim_start_matches('.').to_lowercase())
        .collect();
    let inventory_tlds: HashSet<String> = assets.iter().map(|a| a.tld.clone()).collect();

    let matched: Vec<String> = inventory_tlds
        .intersection(&wallet_tlds)
        .cloned()
        .collect();
    let missing: Vec<String> = inventory_tlds
        .difference(&wallet_tlds)
        .cloned()
        .collect();
    let extra: Vec<String> = wallet_tlds
        .difference(&inventory_tlds)
        .cloned()
        .collect();

    Ok(serde_json::json!({
        "matched": matched,
        "missing": missing,
        "extra": extra,
    }))
}
