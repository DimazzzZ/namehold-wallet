//! Managed `hsrd` sidecar lifecycle.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::db;
use crate::error::AppError;
use crate::noncustodial::hsrd::{resolve_authorization, HsrdClient};
use crate::noncustodial::network::Network;
use crate::AppState;
use tauri::State;
use tokio::time::{sleep, Duration};

pub(crate) const HSRD_MIN_VERSION: (u32, u32, u32) = (0, 3, 4);
const DEFAULT_RPC_BIND: &str = "127.0.0.1:12037";
const AUTHORIZATION_FILE: &str = "namehold-wallet.authorization";
const LOG_FILE: &str = "namehold-hsrd.log";

pub(crate) fn pick_hsrd_path(override_path: Option<&str>, candidates: &[String]) -> Option<String> {
    if let Some(path) = override_path.map(str::trim).filter(|path| !path.is_empty()) {
        return Some(path.to_string());
    }
    candidates
        .iter()
        .find(|candidate| Path::new(candidate).exists())
        .cloned()
}

pub(crate) fn hsrd_candidates() -> Vec<String> {
    let mut candidates = vec![
        "/opt/homebrew/bin/hsrd".to_string(),
        "/usr/local/bin/hsrd".to_string(),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".cargo/bin/hsrd").to_string_lossy().to_string());
        candidates.push(home.join(".local/bin/hsrd").to_string_lossy().to_string());
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("hsrd").to_string_lossy().to_string());
        }
    }
    candidates
}

pub(crate) fn find_hsrd_binary(override_path: Option<&str>) -> String {
    if let Some(path) = pick_hsrd_path(override_path, &hsrd_candidates()) {
        return path;
    }
    if let Ok(output) = Command::new("which").arg("hsrd").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path;
            }
        }
    }
    "hsrd".to_string()
}

pub(crate) fn parse_hsrd_version(raw: &str) -> Option<(u32, u32, u32)> {
    let token = raw.split_whitespace().find(|part| {
        part.trim_start_matches(['v', 'V'])
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
    })?;
    let core = token
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn format_version(version: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

fn get_hsrd_version(binary: &str) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn configured_hsrd_path(state: &AppState) -> Option<String> {
    let db = state.db.lock().ok()?;
    db::queries::get_settings(&db)
        .ok()?
        .get("hsrd_path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_data_dir(state: &AppState) -> Result<PathBuf, AppError> {
    let configured = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_settings(&db)?
            .get("hsrd_data_dir")
            .cloned()
            .unwrap_or_default()
    };
    if !configured.trim().is_empty() {
        return Ok(PathBuf::from(configured.trim()));
    }
    dirs::home_dir()
        .map(|home| home.join(".hsrd"))
        .ok_or_else(|| AppError::Other("could not resolve the home directory".into()))
}

fn active_profile_network(state: &AppState) -> Network {
    let Ok(db) = state.db.lock() else {
        return Network::Main;
    };
    let Ok(profile_id) = db::queries::get_active_profile_id(&db) else {
        return Network::Main;
    };
    db::queries::get_wallet_profile(&db, &profile_id)
        .ok()
        .flatten()
        .and_then(|profile| Network::from_str_opt(&profile.network))
        .unwrap_or_default()
}

fn network_argument(network: Network) -> &'static str {
    match network {
        Network::Main => "mainnet",
        Network::Testnet => "testnet",
        Network::Regtest => "regtest",
        Network::Simnet => "simnet",
    }
}

fn configured_rpc_bind(state: &AppState) -> Result<SocketAddr, AppError> {
    let endpoint = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_settings(&db)?
            .get("hsrd_rpc_url")
            .cloned()
            .unwrap_or_else(|| format!("http://{DEFAULT_RPC_BIND}"))
    };
    let url = url::Url::parse(&endpoint)
        .map_err(|e| AppError::InvalidInput(format!("invalid sidecar RPC URL: {e}")))?;
    if url.scheme() != "http" {
        return Err(AppError::InvalidInput(
            "the managed sidecar RPC URL must use loopback HTTP".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::InvalidInput("sidecar RPC URL has no host".into()))?;
    let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1".parse().expect("loopback")
    } else {
        host.parse()
            .map_err(|_| AppError::InvalidInput("managed sidecar host must be an IP".into()))?
    };
    if !ip.is_loopback() {
        return Err(AppError::InvalidInput(
            "the managed sidecar must bind to loopback".into(),
        ));
    }
    Ok(SocketAddr::new(ip, url.port().unwrap_or(12037)))
}

fn is_running(state: &AppState) -> Result<bool, AppError> {
    let mut child = state
        .hsrd_child
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    let running = child
        .as_mut()
        .is_some_and(|process| matches!(process.try_wait(), Ok(None)));
    if !running {
        *child = None;
    }
    Ok(running)
}

struct NodeProbe {
    height: i64,
    verification_progress: Option<f64>,
    headers: Option<i64>,
}

async fn probe_node(state: &AppState) -> Option<NodeProbe> {
    let settings = {
        let db = state.db.lock().ok()?;
        db::queries::get_settings(&db).ok()?
    };
    HsrdClient::from_settings(&settings)
        .get_blockchain_info()
        .await
        .ok()
        .map(|info| NodeProbe {
            height: info.blocks,
            verification_progress: info.verification_progress,
            headers: info.headers,
        })
}

#[tauri::command]
pub async fn node_status(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let binary = find_hsrd_binary(configured_hsrd_path(&state).as_deref());
    let version = get_hsrd_version(&binary);
    let data_dir = resolve_data_dir(&state)?;
    let process_alive = is_running(&state)?;
    let probe = probe_node(&state).await;
    let error = probe
        .is_none()
        .then(|| node_start_error(&data_dir))
        .flatten();
    let synced = probe.as_ref().is_some_and(|value| {
        value.verification_progress.map_or_else(
            || value.headers.is_none_or(|headers| value.height >= headers),
            |progress| progress >= 0.9999,
        )
    });
    Ok(serde_json::json!({
        "binary": binary,
        "binary_found": version.is_some(),
        "version": version,
        "data_dir": data_dir,
        "network": active_profile_network(&state).as_str(),
        "process_alive": process_alive,
        "connected": probe.is_some(),
        "height": probe.as_ref().map(|value| value.height),
        "verification_progress": probe.as_ref().and_then(|value| value.verification_progress),
        "headers": probe.as_ref().and_then(|value| value.headers),
        "last_error": error,
        "index_mismatch": error.as_deref().is_some_and(|message| message.contains("wallet index")),
        "read_source": if synced { "sidecar" } else { "unavailable" }
    }))
}

pub(crate) fn node_start_error(data_dir: &Path) -> Option<String> {
    let body = fs::read_to_string(data_dir.join(LOG_FILE)).ok()?;
    if body.trim().is_empty() || !body.to_ascii_lowercase().contains("error") {
        return None;
    }
    let tail = body
        .trim_end()
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("hsrd failed to start:\n\n{tail}"))
}

fn ensure_authorization(state: &AppState, data_dir: &Path) -> Result<PathBuf, AppError> {
    let path = data_dir.join(AUTHORIZATION_FILE);
    let settings = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_settings(&db)?
    };
    let mut authorization = resolve_authorization(&settings);
    if authorization.is_empty() {
        authorization = fs::read_to_string(&path)
            .ok()
            .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("Bearer {}", hex::encode(rand::random::<[u8; 32]>())));
    }
    if authorization.len() > 4_096
        || authorization
            .bytes()
            .any(|byte| !(0x20..=0x7e).contains(&byte))
    {
        return Err(AppError::InvalidInput(
            "sidecar Authorization must be 1..=4096 visible ASCII bytes".into(),
        ));
    }
    write_private_file(&path, authorization.as_bytes())?;
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::set_setting(&db, "hsrd_authorization", &authorization)?;
    Ok(path)
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn start_hsrd(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    if is_running(&state)? {
        return Err(AppError::Other("hsrd is already running".into()));
    }
    if let Some(probe) = probe_node(&state).await {
        return Ok(serde_json::json!({
            "connected": true,
            "process_alive": false,
            "height": probe.height,
            "adopted": true
        }));
    }

    let binary = find_hsrd_binary(configured_hsrd_path(&state).as_deref());
    if let Some(raw) = get_hsrd_version(&binary) {
        if let Some(found) = parse_hsrd_version(&raw) {
            if found < HSRD_MIN_VERSION {
                return Err(AppError::Other(format!(
                    "hsrd {} is older than required {}; install a current hsrd release",
                    raw.trim(),
                    format_version(HSRD_MIN_VERSION)
                )));
            }
        }
    }

    let data_dir = resolve_data_dir(&state)?;
    fs::create_dir_all(&data_dir)?;
    let authorization_file = ensure_authorization(&state, &data_dir)?;
    let network = active_profile_network(&state);
    let bind = configured_rpc_bind(&state)?;
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::set_setting(&db, "hsrd_network", network.as_str())?;
    }

    let log_path = data_dir.join(LOG_FILE);
    let log = fs::File::create(&log_path)?;
    let log_error = log.try_clone()?;
    let child = Command::new(&binary)
        .arg("--network")
        .arg(network_argument(network))
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--rpc-bind")
        .arg(bind.to_string())
        .arg("--rpc-authorization-header-file")
        .arg(&authorization_file)
        .arg("--native-sync")
        .arg("--p2p-discovery")
        .arg("--wallet-index")
        .arg("--storage-mode")
        .arg("archive")
        .arg("--mining-engine")
        .arg("--transaction-relay")
        .arg("--acknowledge-incomplete-consensus")
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_error))
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to start hsrd ({binary}): {e}")))?;
    {
        let mut slot = state
            .hsrd_child
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        *slot = Some(child);
    }
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('start_hsrd', ?1)",
            [serde_json::json!({
                "data_dir": data_dir,
                "network": network.as_str(),
                "wallet_rpc_api_version": 1
            })
            .to_string()],
        )?;
    }

    for _ in 0..30 {
        {
            let mut slot = state
                .hsrd_child
                .lock()
                .map_err(|e| AppError::Lock(e.to_string()))?;
            let exited = slot
                .as_mut()
                .is_none_or(|process| !matches!(process.try_wait(), Ok(None)));
            if exited {
                *slot = None;
                return Err(AppError::Other(format!(
                    "hsrd exited during startup.{}",
                    read_log_tail(&log_path)
                )));
            }
        }
        if let Some(probe) = probe_node(&state).await {
            return Ok(serde_json::json!({
                "connected": true,
                "process_alive": true,
                "height": probe.height,
                "data_dir": data_dir,
                "network": network.as_str()
            }));
        }
        sleep(Duration::from_millis(500)).await;
    }
    Ok(serde_json::json!({
        "connected": false,
        "process_alive": true,
        "data_dir": data_dir,
        "network": network.as_str(),
        "message": "hsrd is starting; authenticated wallet RPC will become available after initialization"
    }))
}

pub(crate) fn read_log_tail(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            let tail = value
                .trim_end()
                .lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            format!(" Last log lines:\n{tail}")
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn stop_hsrd(state: State<'_, AppState>) -> Result<(), AppError> {
    let child = state
        .hsrd_child
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?
        .take();
    if let Some(mut process) = child {
        process.kill()?;
        process.wait()?;
    } else if probe_node(&state).await.is_some() {
        return Err(AppError::InvalidInput(
            "the reachable hsrd process is externally managed and was not stopped".into(),
        ));
    }
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('stop_hsrd', 'managed sidecar')",
        [],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn resync_hsrd_chain(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    if let Some(mut process) = state
        .hsrd_child
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?
        .take()
    {
        let _ = process.kill();
        let _ = process.wait();
    }
    let data_dir = resolve_data_dir(&state)?;
    let authorization = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        resolve_authorization(&db::queries::get_settings(&db)?)
    };
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let backup = data_dir.with_file_name(format!(
        "{}-backup-{timestamp}",
        data_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("hsrd-data")
    ));
    if data_dir.exists() {
        fs::rename(&data_dir, &backup)?;
    }
    fs::create_dir_all(&data_dir)?;
    if !authorization.is_empty() {
        write_private_file(&data_dir.join(AUTHORIZATION_FILE), authorization.as_bytes())?;
    }
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('resync_hsrd_chain', ?1)",
            [serde_json::json!({ "backup": backup }).to_string()],
        )?;
    }
    start_hsrd(state).await
}
