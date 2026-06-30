use crate::db;
use crate::error::AppError;
use crate::hsd::client::HandshakeClient;
use crate::AppState;
use std::process::{Command, Stdio};
use tauri::State;

fn get_client(state: &AppState) -> Result<HandshakeClient, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let settings = db::queries::get_settings(&db)?;
    Ok(HandshakeClient::from_settings(&settings))
}

fn find_hsd_binary() -> String {
    // Check absolute paths first
    let candidates = vec![
        "/opt/homebrew/bin/hsd".to_string(),
        "/usr/local/bin/hsd".to_string(),
        dirs::home_dir()
            .map(|h| h.join(".npm-global/bin/hsd").to_string_lossy().to_string())
            .unwrap_or_default(),
    ];
    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return candidate.clone();
        }
    }
    // Try which to find full path
    if let Ok(output) = Command::new("which").arg("hsd").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return path;
            }
        }
    }
    "hsd".to_string()
}

fn get_hsd_version(binary: &str) -> Option<String> {
    Command::new(binary)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

#[tauri::command]
pub async fn get_node_status(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let client = get_client(&state)?;
    let hsd_bin = find_hsd_binary();
    let bin_exists = std::path::Path::new(&hsd_bin).exists();
    let version = get_hsd_version(&hsd_bin);

    // Try wallet connection
    let wallet_ok = client.check_connection().await.is_ok();

    // Try blockchain info
    let chain_info = client.get_blockchain_info().await.ok();

    let running = wallet_ok || chain_info.is_some();

    Ok(serde_json::json!({
        "running": running,
        "wallet_connected": wallet_ok,
        "hsd_binary": hsd_bin,
        "hsd_binary_found": bin_exists,
        "hsd_version": version,
        "chain": chain_info,
    }))
}

#[tauri::command]
pub async fn stop_hsd(state: State<'_, AppState>) -> Result<String, AppError> {
    let client = get_client(&state)?;
    client.stop_node().await?;
    Ok("hsd stop signal sent".to_string())
}

#[tauri::command]
pub async fn start_hsd(
    state: State<'_, AppState>,
    prefix: Option<String>,
    api_key: Option<String>,
    network: Option<String>,
) -> Result<String, AppError> {
    let (default_prefix, default_api_key, default_network) = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        let prefix = settings.get("hsd_prefix").cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .map(|h| h.join(".hsd").to_string_lossy().to_string())
                    .unwrap_or_else(|| "/root/.hsd".to_string())
            });
        let api_key = settings.get("hsd_api_key").cloned().unwrap_or_default();
        let network = settings.get("hsd_network").cloned().unwrap_or_else(|| "mainnet".to_string());
        (prefix, api_key, network)
    };

    let hsd_prefix = prefix.filter(|s| !s.is_empty()).unwrap_or(default_prefix);
    let hsd_api_key = api_key.filter(|s| !s.is_empty()).unwrap_or(default_api_key);
    let hsd_network = network.filter(|s| !s.is_empty()).unwrap_or(default_network);

    let hsd_bin = find_hsd_binary();
    let mut cmd = Command::new(&hsd_bin);
    cmd.arg(format!("--prefix={}", hsd_prefix));
    if !hsd_api_key.is_empty() {
        cmd.arg(format!("--api-key={}", hsd_api_key));
    }
    match hsd_network.as_str() {
        "testnet" => { cmd.arg("--testnet"); }
        "regtest" => { cmd.arg("--regtest"); }
        _ => {}
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());

    match cmd.spawn() {
        Ok(_) => {
            let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db.execute(
                "INSERT INTO audit_log (action, detail) VALUES ('start_hsd', ?1)",
                [serde_json::json!({"prefix": hsd_prefix, "network": hsd_network}).to_string()],
            )?;
            Ok(format!("hsd started with prefix: {}", hsd_prefix))
        }
        Err(e) => Err(AppError::Other(format!("Failed to start hsd: {}", e))),
    }
}
