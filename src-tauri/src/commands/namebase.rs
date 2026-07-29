use crate::db;
use crate::error::AppError;
use crate::namebase::client::NamebaseClient;
use crate::noncustodial::cookie_vault;
use crate::AppState;
use tauri::State;

/// Read the Namebase session cookie, preferring the encrypted v1 blob.
/// If `namebase_cookie_v1` is empty but the legacy `namebase_cookie` has a
/// value, migrate it: encrypt → store in v1 → blank the legacy row.
fn read_cookie(state: &AppState) -> Result<String, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;

    let v1_hex = settings
        .get("namebase_cookie_v1")
        .cloned()
        .unwrap_or_default();
    if !v1_hex.is_empty() {
        // Decrypt the v1 blob.
        let plaintext = cookie_vault::decrypt_cookie(&v1_hex)?;
        return Ok(String::from_utf8_lossy(&plaintext).into_owned());
    }

    // Fallback: check the legacy plaintext key.
    let legacy = settings.get("namebase_cookie").cloned().unwrap_or_default();
    if legacy.is_empty() {
        return Ok(String::new());
    }

    // Migrate: encrypt the legacy value and blank the plaintext row.
    match cookie_vault::encrypt_cookie(legacy.as_bytes()) {
        Ok(blob_hex) => {
            db::queries::set_setting(&db, "namebase_cookie_v1", &blob_hex)?;
            db::queries::set_setting(&db, "namebase_cookie", "")?;
            db.execute(
                "INSERT INTO audit_log (action, detail) VALUES ('namebase_cookie_migrated', '{}')",
                [],
            )?;
        }
        Err(_e) => {
            // Keyring unavailable — return the legacy plaintext (don't break
            // existing sessions). The cookie stays in the legacy plaintext key
            // until a keyring becomes available on a later read.
            return Ok(legacy);
        }
    }

    Ok(legacy)
}

/// Encrypt and persist the cookie under the v1 key. Also blanks the legacy key.
fn write_cookie(state: &AppState, cookie: &str) -> Result<(), AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    if cookie.is_empty() {
        // Disconnect: blank both keys.
        db::queries::set_setting(&db, "namebase_cookie_v1", "")?;
        db::queries::set_setting(&db, "namebase_cookie", "")?;
        return Ok(());
    }
    let blob_hex = cookie_vault::encrypt_cookie(cookie.as_bytes())?;
    db::queries::set_setting(&db, "namebase_cookie_v1", &blob_hex)?;
    // Blank legacy key (defense in depth).
    db::queries::set_setting(&db, "namebase_cookie", "")?;
    Ok(())
}

/// Read the `namebase_base_url` test seam, but ONLY in debug builds / tests.
/// In release builds this always returns empty so the client uses the real
/// Namebase host — a poisoned setting can never redirect the session cookie.
fn test_base_url_override(_settings: &crate::models::settings::SettingsMap) -> String {
    #[cfg(any(debug_assertions, test))]
    {
        _settings
            .get("namebase_base_url")
            .cloned()
            .unwrap_or_default()
    }
    #[cfg(not(any(debug_assertions, test)))]
    {
        String::new()
    }
}

/// Build a Namebase client, honoring an optional `namebase_base_url` setting so
/// tests can point the irreversible transfer/withdraw calls at a mock server.
/// Production leaves the setting unset → the real Namebase host.
pub(crate) fn namebase_client(state: &AppState) -> Result<NamebaseClient, AppError> {
    let cookie = read_cookie(state)?;
    let base = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        test_base_url_override(&settings)
    };
    if base.trim().is_empty() {
        NamebaseClient::new(&cookie)
    } else {
        NamebaseClient::with_base_url(&cookie, base.trim())
    }
}

/// Build a Namebase client with an explicit cookie, still honoring the
/// `namebase_base_url` test seam.  Used by `connect_namebase` which receives
/// the cookie as a command parameter rather than reading it from settings.
pub(crate) fn namebase_client_with_cookie(
    state: &AppState,
    cookie: &str,
) -> Result<NamebaseClient, AppError> {
    let base = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        test_base_url_override(&settings)
    };
    if base.trim().is_empty() {
        NamebaseClient::new(cookie)
    } else {
        NamebaseClient::with_base_url(cookie, base.trim())
    }
}

/// Write the client's current cookie back to settings if it differs from
/// `before` — i.e. the server rotated it via `Set-Cookie` during this
/// command's calls. A no-op write is skipped so an unchanged session doesn't
/// churn the `settings` table on every dashboard poll.
pub(crate) fn persist_cookie_if_changed(
    state: &AppState,
    before: &str,
    client: &NamebaseClient,
) -> Result<(), AppError> {
    let after = client.current_cookie();
    if after != before {
        write_cookie(state, &after)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn connect_namebase(
    state: State<'_, AppState>,
    cookie: String,
) -> Result<serde_json::Value, AppError> {
    let client = namebase_client_with_cookie(&state, &cookie)?;

    let account = client.get_account().await.map_err(|e| {
        AppError::Other(format!(
            "Namebase rejected the session ({}). Make sure you copied the full cookie header from sunset.namebase.io",
            e
        ))
    })?;

    // Store whatever the client's jar ends up holding (not the raw paste) —
    // if Namebase rotated the cookie on this very first request, we want the
    // rotated value on disk, not the one the user pasted.
    let final_cookie = client.current_cookie();
    write_cookie(&state, &final_cookie)?;
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_connect', ?1)",
        [serde_json::json!({"status": "connected"}).to_string()],
    )?;

    Ok(account)
}

#[tauri::command]
pub async fn disconnect_namebase(state: State<'_, AppState>) -> Result<(), AppError> {
    write_cookie(&state, "")?;
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_disconnect', ?1)",
        [serde_json::json!({"status": "disconnected"}).to_string()],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn get_namebase_status(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let cookie = read_cookie(&state)?;

    if cookie.is_empty() {
        return Ok(serde_json::json!({"connected": false, "has_cookie": false}));
    }

    let client = namebase_client(&state)?;
    let before = client.current_cookie();
    let result = match client.check_session().await {
        Ok(true) => {
            let account = client.get_account().await.ok();
            serde_json::json!({"connected": true, "has_cookie": true, "account": account})
        }
        _ => {
            serde_json::json!({"connected": false, "has_cookie": true, "error": "Session expired"})
        }
    };
    persist_cookie_if_changed(&state, &before, &client)?;
    Ok(result)
}

#[tauri::command]
pub async fn fetch_namebase_domains(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = namebase_client(&state)?;
    let before = client.current_cookie();
    let result = client.get_domains().await?;
    persist_cookie_if_changed(&state, &before, &client)?;
    Ok(result)
}

#[tauri::command]
pub async fn fetch_namebase_staked(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = namebase_client(&state)?;
    let before = client.current_cookie();
    let result = client.get_staked_domains().await?;
    persist_cookie_if_changed(&state, &before, &client)?;
    Ok(result)
}

#[tauri::command]
pub async fn fetch_namebase_renewals(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    // Use the shared client builder so the `namebase_base_url` test seam applies
    // (and so this honors any future base-url override), unlike the other fetch_*
    // commands which hard-code the real host.
    let client = namebase_client(&state)?;
    let before = client.current_cookie();
    let result = client.get_renewals().await?;
    persist_cookie_if_changed(&state, &before, &client)?;
    Ok(result)
}

#[tauri::command]
pub async fn fetch_namebase_withdrawals(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = namebase_client(&state)?;
    let before = client.current_cookie();
    let result = client.get_withdrawals().await?;
    persist_cookie_if_changed(&state, &before, &client)?;
    Ok(result)
}

#[tauri::command]
pub async fn import_from_namebase(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = namebase_client(&state)?;
    let before = client.current_cookie();

    let domains = client.get_domains().await?;
    let staked_data = client.get_staked_domains().await?;
    persist_cookie_if_changed(&state, &before, &client)?;

    let staked_names: std::collections::HashSet<String> = staked_data["stakedDomains"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d["name"].as_str().map(|s| s.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let mut imported = 0;
    let mut staked_imported = 0;
    let mut skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    // Collect names already seen from the transferable list.
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;

        // First pass: import all transferable domains.
        if let Some(arr) = domains["domains"].as_array() {
            for domain in arr {
                let name = match domain["name"].as_str() {
                    Some(n) => n.to_lowercase().trim().to_string(),
                    None => {
                        skipped += 1;
                        continue;
                    }
                };

                seen_names.insert(name.clone());
                let is_staked = staked_names.contains(&name);
                let status = if is_staked {
                    "do_not_touch_staked"
                } else {
                    "not_started"
                };

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

        // Second pass: import staked-only domains that weren't in the transferable list.
        for name in &staked_names {
            if seen_names.contains(name) {
                continue; // already imported in first pass
            }
            staked_imported += 1;
            let _ = db.execute(
                "INSERT INTO assets (tld, is_staked, status, category, notes)
                 VALUES (?1, 1, 'do_not_touch_staked', 'Namebase', 'Imported from Namebase (staked)')
                 ON CONFLICT(tld) DO UPDATE SET
                   is_staked = 1,
                   status = 'do_not_touch_staked',
                   updated_at = datetime('now')",
                rusqlite::params![name],
            );
        }

        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('namebase_import', ?1)",
            [serde_json::json!({
                "imported": imported,
                "staked_imported": staked_imported,
                "skipped": skipped,
                "errors": errors.len(),
                "staked_count": staked_names.len(),
            })
            .to_string()],
        )?;
    }

    Ok(serde_json::json!({
        "imported": imported,
        "staked_imported": staked_imported,
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
    let before = client.current_cookie();
    client.transfer_domain(&name, &address).await?;
    persist_cookie_if_changed(&state, &before, &client)?;

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
        Ok(Some(p)) => crate::noncustodial::derivation::network_from_profile(&p.network)
            .unwrap_or(Network::Main),
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
    let before = client.current_cookie();
    client.withdraw_hns(&address, amount.trim()).await?;
    persist_cookie_if_changed(&state, &before, &client)?;

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('namebase_withdraw_hns', ?1)",
        [serde_json::json!({"address": address, "amount": amount}).to_string()],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn fetch_namebase_domain_withdrawals(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let client = namebase_client(&state)?;
    let before = client.current_cookie();
    let result = client.get_domain_withdrawals().await?;
    persist_cookie_if_changed(&state, &before, &client)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::test_base_url_override;
    use crate::models::settings::SettingsMap;

    #[test]
    fn test_base_url_override_returns_setting_value_in_test_build() {
        // Under `cfg(test)` (and debug builds) the seam echoes the setting so
        // integration tests can point the client at a mock server.
        let mut settings = SettingsMap::new();
        settings.insert(
            "namebase_base_url".to_string(),
            "http://127.0.0.1:8080".to_string(),
        );
        assert_eq!(test_base_url_override(&settings), "http://127.0.0.1:8080");
    }

    #[test]
    fn test_base_url_override_returns_empty_when_setting_absent() {
        let settings = SettingsMap::new();
        assert_eq!(test_base_url_override(&settings), "");
    }

    // Note: the release-only branch (`#[cfg(not(any(debug_assertions,
    // test)))]`) always returns "" regardless of settings. It is compiled out
    // under a test binary, so it cannot be exercised here directly — the
    // build-config guards enforce that invariant at compile time.
}
