//! App-managed hsd node lifecycle.
//!
//! Reads are node-free (explorer); a node is needed only to broadcast/sync. The
//! app can start hsd against a user-chosen data directory (`hsd_prefix` setting)
//! so a large chain lands on, say, an external volume instead of `~/.hsd`. The
//! `--api-key` and network are kept in sync with the RPC the app itself uses
//! (`node_rpc_api_key` + the active profile's network), so "the node the app
//! starts" and "the node the app talks to" are the same node.

use crate::db;
use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::AppState;
use std::process::{Command, Stdio};
use tauri::State;
use tokio::time::{sleep, Duration};

/// First usable hsd path given an explicit override and an ordered candidate
/// list. An explicit `hsd_path` is honored verbatim (trust the user — the binary
/// may not exist yet at lookup time); otherwise the first existing candidate
/// wins. Returns `None` if nothing matches (caller falls back to `which`/PATH).
/// Pure (no env reads) so it's unit-testable.
pub(crate) fn pick_hsd_path(override_path: Option<&str>, candidates: &[String]) -> Option<String> {
    if let Some(p) = override_path {
        let p = p.trim();
        if !p.is_empty() {
            return Some(p.to_string());
        }
    }
    candidates
        .iter()
        .find(|c| std::path::Path::new(c).exists())
        .cloned()
}

/// Common hsd install locations to probe (a GUI-launched app has a minimal PATH,
/// so the user's shell PATH isn't available — we must look in the usual dirs).
fn hsd_candidates() -> Vec<String> {
    let mut candidates = vec![
        "/opt/homebrew/bin/hsd".to_string(),
        "/usr/local/bin/hsd".to_string(),
    ];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(format!("{home}/.npm-global/bin/hsd"));
        candidates.push(format!("{home}/.npm/bin/hsd"));
        candidates.push(format!("{home}/.local/bin/hsd"));
        // nvm-managed node installs: ~/.nvm/versions/node/<ver>/bin/hsd
        if let Ok(entries) = std::fs::read_dir(format!("{home}/.nvm/versions/node")) {
            for e in entries.flatten() {
                let cand = e.path().join("bin/hsd");
                if cand.exists() {
                    candidates.push(cand.to_string_lossy().to_string());
                }
            }
        }
    }
    candidates
}

/// Locate the hsd binary: an explicit `hsd_path` override first, then common
/// install dirs, then `which hsd`, then the bare name (resolved via PATH).
fn find_hsd_binary(override_path: Option<&str>) -> String {
    if let Some(found) = pick_hsd_path(override_path, &hsd_candidates()) {
        return found;
    }
    if let Ok(output) = Command::new("which").arg("hsd").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return path;
            }
        }
    }
    "hsd".to_string()
}

/// The explicit `hsd_path` setting, if set and non-empty.
fn configured_hsd_path(state: &AppState) -> Option<String> {
    let db = state.db.lock().ok()?;
    let settings = db::queries::get_settings(&db).ok()?;
    settings
        .get("hsd_path")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `hsd --version`, if the binary runs.
fn get_hsd_version(binary: &str) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;
    if output.status.success() {
        let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!v.is_empty()).then_some(v)
    } else {
        None
    }
}

/// The configured hsd data directory, or hsd's own default (`~/.hsd`) when unset.
fn resolve_data_dir(state: &AppState) -> Result<String, AppError> {
    let configured = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_settings(&db)?
            .get("hsd_prefix")
            .cloned()
            .unwrap_or_default()
    };
    let configured = configured.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Ok(format!("{home}/.hsd"))
}

/// The active profile's network, defaulting to mainnet — matches the network the
/// rest of the app operates on (and the default RPC port).
fn active_profile_network(state: &AppState) -> Network {
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

/// Whether the hsd we started this session is still alive. Reaps a child that has
/// exited (clearing the handle) so the status reflects reality.
fn is_running(state: &AppState) -> Result<bool, AppError> {
    let mut guard = state.hsd_child.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let running = match guard.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    };
    if !running {
        *guard = None;
    }
    Ok(running)
}

/// A successful node RPC probe: the node answered `getblockchaininfo`.
struct NodeProbe {
    height: i64,
    /// Sync progress 0.0..=1.0, when the node reports it.
    verification_progress: Option<f64>,
    /// Peers' best header height (the sync target), when reported.
    headers: Option<i64>,
}

/// The authoritative "is the node actually answering?" check: probe the node RPC
/// (same `getblockchaininfo` call the sync + write-capability paths use). Returns
/// `Some` only when the RPC answers — this is what `connected` is based on,
/// since process liveness is not proof the RPC server is up. Carries sync
/// progress so the UI can show how far the node has caught up.
async fn probe_node(state: &AppState) -> Option<NodeProbe> {
    // Clone the settings map under the lock, then drop it — never hold the db
    // mutex across the await.
    let settings = {
        let db = state.db.lock().ok()?;
        db::queries::get_settings(&db).ok()?
    };
    let client = NodeRpcClient::from_settings(&settings);
    client.get_blockchain_info().await.ok().map(|info| NodeProbe {
        height: info.blocks,
        verification_progress: info.verification_progress,
        headers: info.headers,
    })
}

/// Node status for the Settings UI + status strip. `connected` (RPC answers) is
/// the authoritative signal; `process_alive` only reflects the child we spawned.
#[tauri::command]
pub async fn node_status(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let binary = find_hsd_binary(configured_hsd_path(&state).as_deref());
    let version = get_hsd_version(&binary);
    let data_dir = resolve_data_dir(&state)?;
    let process_alive = is_running(&state)?;
    let probe = probe_node(&state).await;
    // When the RPC isn't answering, surface WHY the last start failed (the hsd log
    // persists past `start_hsd`'s short watch window), so a failed start never
    // shows as a silent "Starting…/Stopped".
    let err = if probe.is_none() {
        node_start_error(&data_dir)
    } else {
        None
    };
    let (last_error, index_mismatch) = match err {
        Some((msg, mismatch)) => (Some(msg), mismatch),
        None => (None, false),
    };
    Ok(serde_json::json!({
        "binary": binary,
        "binary_found": version.is_some(),
        "version": version,
        "data_dir": data_dir,
        "network": active_profile_network(&state).as_str(),
        "process_alive": process_alive,
        "connected": probe.is_some(),
        "height": probe.as_ref().map(|p| p.height),
        "verification_progress": probe.as_ref().and_then(|p| p.verification_progress),
        "headers": probe.as_ref().and_then(|p| p.headers),
        "last_error": last_error,
        // True when the failure is a chain/index-flag mismatch hsd can't fix in
        // place — the UI offers a one-click re-sync for this case.
        "index_mismatch": index_mismatch,
    }))
}

/// If `<data_dir>/namehold-hsd.log` records a startup failure, return
/// `(human_reason, is_index_mismatch)`. `None` when there's no log or it doesn't
/// look like an error. The index-mismatch case (hsd can't retro-enable an index)
/// gets specific guidance and flags the UI to offer a one-click re-sync.
pub(crate) fn node_start_error(data_dir: &str) -> Option<(String, bool)> {
    let log_path = std::path::Path::new(data_dir).join("namehold-hsd.log");
    let body = std::fs::read_to_string(&log_path).ok()?;
    let body = body.trim();
    if body.is_empty() || (!body.contains("Error") && !body.contains("error")) {
        return None;
    }
    let tail: Vec<&str> = body.lines().rev().take(8).collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    if body.contains("retroactively") && body.contains("indexing") {
        Some((
            format!(
                "This chain's indexes don't match what the wallet needs, and hsd can't \
                 change a chain's indexes in place. Re-sync the node data (the old chain \
                 is moved to a backup), or point Node RPC at an already-indexed node.\n\n{tail}"
            ),
            true,
        ))
    } else {
        Some((format!("hsd failed to start:\n\n{tail}"), false))
    }
}

/// Start hsd against the configured data directory. The data dir comes from the
/// `hsd_prefix` setting (default `~/.hsd`); the API key mirrors `node_rpc_api_key`
/// and the network mirrors the active profile, so the app talks to exactly the
/// node it started.
#[tauri::command]
pub async fn start_hsd(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    if is_running(&state)? {
        return Err(AppError::Other("hsd is already running.".to_string()));
    }
    // A node may already be running (e.g. one started in a previous app session,
    // or the user's own). If its RPC already answers, adopt it — never spawn a
    // duplicate, which would only collide on the data-dir lock.
    if let Some(probe) = probe_node(&state).await {
        return Ok(serde_json::json!({
            "connected": true,
            "process_alive": is_running(&state)?,
            "height": probe.height,
        }));
    }

    let data_dir = resolve_data_dir(&state)?;
    // Use the same effective api-key the RPC client uses (explicit setting, else
    // the data dir's hsd.conf), so the node we start and the node we talk to agree.
    let api_key = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&db)?;
        crate::noncustodial::rpc::resolve_node_api_key(&settings)
    };
    let network = active_profile_network(&state);

    std::fs::create_dir_all(&data_dir)
        .map_err(|e| AppError::Other(format!("cannot create data dir {data_dir}: {e}")))?;

    let binary = find_hsd_binary(configured_hsd_path(&state).as_deref());
    let mut cmd = Command::new(&binary);
    cmd.arg(format!("--prefix={data_dir}"));
    if !api_key.trim().is_empty() {
        cmd.arg(format!("--api-key={}", api_key.trim()));
    }
    // The wallet learns its spendable + name-owner coins via `getcoinsbyaddress`
    // (ADDRESS index); `getrawtransaction` (TX index) backs the transaction-history
    // cache AND the pending→confirmed tracking that looks a sent tx up by id.
    //
    // IMPORTANT: hsd cannot retroactively enable an index on an already-synced
    // chain (chaindb.js `verifyFlags` throws "Cannot retroactively enable …
    // indexing"). A chain synced without these must be re-synced from genesis with
    // them — there is no in-place reindex.
    cmd.arg("--index-address");
    cmd.arg("--index-tx");
    match network {
        Network::Testnet => {
            cmd.arg("--testnet");
        }
        Network::Regtest => {
            cmd.arg("--regtest");
        }
        Network::Simnet => {
            cmd.arg("--simnet");
        }
        Network::Main => {}
    }
    // Capture hsd's output to a log file so a failed start has a visible reason
    // (port busy, bad data dir, network mismatch) instead of vanishing into null.
    let log_path = std::path::Path::new(&data_dir).join("namehold-hsd.log");
    let log = std::fs::File::create(&log_path)
        .map_err(|e| AppError::Other(format!("cannot open hsd log {}: {e}", log_path.display())))?;
    let log_err = log
        .try_clone()
        .map_err(|e| AppError::Other(format!("cannot prepare hsd log: {e}")))?;
    cmd.stdout(Stdio::from(log)).stderr(Stdio::from(log_err));

    let child = cmd
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to start hsd ({binary}): {e}")))?;

    {
        let mut guard = state.hsd_child.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        *guard = Some(child);
    }
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('start_hsd', ?1)",
            [serde_json::json!({"data_dir": data_dir, "network": network.as_str()}).to_string()],
        )?;
    }

    // Atomic outcome: wait until the node's RPC actually answers (success), the
    // child exits (failure — surface the log tail), or we time out (still
    // starting — node_status polling flips it green when RPC comes up).
    for _ in 0..30 {
        // Did the child die during startup?
        {
            let mut guard = state.hsd_child.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            let exited = match guard.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(Some(_))),
                None => true,
            };
            if exited {
                *guard = None;
                drop(guard);
                let tail = read_log_tail(&log_path);
                // A data-dir lock means another hsd already owns this directory.
                if tail.contains("LOCK") || tail.contains("Resource temporarily unavailable") {
                    return Err(AppError::Other(format!(
                        "A node is already running on this data directory ({data_dir}). \
                         The app will use it once it can reach its RPC — set the Node RPC \
                         API key in Settings (or it's read from hsd.conf).{tail}"
                    )));
                }
                return Err(AppError::Other(format!("hsd exited on startup.{tail}")));
            }
        }
        if let Some(probe) = probe_node(&state).await {
            return Ok(serde_json::json!({
                "connected": true,
                "process_alive": true,
                "height": probe.height,
                "data_dir": data_dir,
                "network": network.as_str(),
            }));
        }
        sleep(Duration::from_millis(500)).await;
    }

    Ok(serde_json::json!({
        "connected": false,
        "process_alive": true,
        "data_dir": data_dir,
        "network": network.as_str(),
        "message": "hsd is still starting; status will update when its RPC responds.",
    }))
}

/// Last few lines of the hsd log, for surfacing a startup failure reason.
fn read_log_tail(path: &std::path::Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => {
            let tail: Vec<&str> = s.trim_end().lines().rev().take(8).collect();
            let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            format!(" Last log lines:\n{tail}")
        }
        _ => String::new(),
    }
}

/// Stop the app-managed hsd (no-op if we didn't start one this session).
#[tauri::command]
pub async fn stop_hsd(state: State<'_, AppState>) -> Result<(), AppError> {
    let child = {
        let mut guard = state.hsd_child.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        guard.take()
    };
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('stop_hsd', ?1)",
        ["{}"],
    )?;
    Ok(())
}

/// The on-disk chain artifacts to move aside for a re-sync, by network. Mainnet
/// stores them at the prefix root; other networks under a per-network subdir.
/// Pure (path computation only) so it's unit-testable.
pub(crate) fn chain_paths_for_network(data_dir: &str, network: Network) -> Vec<std::path::PathBuf> {
    let base = std::path::Path::new(data_dir);
    match network {
        Network::Main => ["blocks", "chain", "tree"].iter().map(|p| base.join(p)).collect(),
        Network::Testnet => vec![base.join("testnet")],
        Network::Regtest => vec![base.join("regtest")],
        Network::Simnet => vec![base.join("simnet")],
    }
}

/// One-click recovery for an index/flag mismatch: stop the managed node, move the
/// current chain data to a timestamped backup under the data dir, then start hsd
/// fresh so it re-syncs with the wallet's required indexes. The backup is
/// reversible (the wallet/key/conf are left in place); reads stay node-free.
#[tauri::command]
pub async fn resync_hsd_chain(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    // 1. Stop any node we manage so the chain files aren't locked.
    {
        let mut guard = state.hsd_child.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    let data_dir = resolve_data_dir(&state)?;
    let network = active_profile_network(&state);

    // 2. Move existing chain artifacts into a timestamped backup dir.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = std::path::Path::new(&data_dir).join(format!("_noindex-backup-{ts}"));
    std::fs::create_dir_all(&backup)
        .map_err(|e| AppError::Other(format!("cannot create backup dir: {e}")))?;
    let mut moved = Vec::new();
    for p in chain_paths_for_network(&data_dir, network) {
        if p.exists() {
            let name = p
                .file_name()
                .ok_or_else(|| AppError::Other("bad chain path".into()))?;
            std::fs::rename(&p, backup.join(name))
                .map_err(|e| AppError::Other(format!("cannot move {}: {e}", p.display())))?;
            moved.push(p.display().to_string());
        }
    }
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db.execute(
            "INSERT INTO audit_log (action, detail) VALUES ('resync_hsd_chain', ?1)",
            [serde_json::json!({
                "backup": backup.display().to_string(),
                "moved": moved,
            })
            .to_string()],
        )?;
    }

    // 3. Start hsd fresh — it re-syncs with the required indexes.
    start_hsd(state).await
}
