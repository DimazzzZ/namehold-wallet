//! Draft-based transaction commands: sync, build, sign, broadcast.
//!
//! The write path is split into three stages so the frontend can show a
//! confirmation before any key material is touched:
//!   1. `build_send_hns_draft` — coin selection + fee/change preview, persisted
//!      as a `draft` row. Requires NO unlock.
//!   2. `sign_tx_draft` — materializes and signs the tx from the unlocked
//!      signer session. Requires unlock.
//!   3. `broadcast_tx_draft` — sends the signed hex via node RPC.
//!
//! Plain HNS sends only (covenant/name actions are a later milestone).

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db;
use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{ChainSource, NodeRpcClient};
use crate::noncustodial::send;
use crate::noncustodial::types::{BroadcastResult, TxDraftSummary, TxSummary};
use crate::noncustodial::{derivation, sync};
use crate::AppState;

/// Build parameters persisted in `signing_inputs_json`, replayed at sign time so
/// the signed transaction matches the previewed intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendBuildParams {
    to_address: String,
    amount_doos: u64,
    change_address: String,
    rate_per_byte: u64,
    account: u32,
    network: String,
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Resolve the active wallet profile or error if none is selected.
fn active_profile(
    conn: &rusqlite::Connection,
) -> Result<crate::noncustodial::types::WalletProfileSummary, AppError> {
    let id = db::queries::get_active_profile_id(conn)?;
    if id.is_empty() {
        return Err(AppError::InvalidInput("no active wallet profile".to_string()));
    }
    db::queries::get_wallet_profile(conn, &id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))
}

/// Derive the change address (branch 1, index 0) for a profile from its xpub.
fn change_address(
    network: Network,
    account_xpub: &str,
) -> Result<String, AppError> {
    let xpub = crate::noncustodial::hd::ExtendedPubKey::from_xpub(network, account_xpub)?;
    let derived = derivation::derive_one(network, &xpub, derivation::BRANCH_CHANGE, 0)?;
    Ok(derived.address)
}

fn session_ttl_ms(settings: &std::collections::HashMap<String, String>) -> u128 {
    let secs = settings
        .get("signer_session_timeout_seconds")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(900);
    (secs as u128) * 1000
}

/// Resolve the fee rate (doos/byte): explicit override, else ask the node's
/// `estimatesmartfee`, else the fixed relay-floor default. Never errors.
async fn resolve_fee_rate(state: &State<'_, AppState>, fee_rate: Option<u64>) -> u64 {
    if let Some(r) = fee_rate {
        return r;
    }
    let settings = {
        match state.db.lock() {
            Ok(conn) => db::queries::get_settings(&conn).ok(),
            Err(_) => None,
        }
    };
    match settings {
        Some(s) => {
            let client = NodeRpcClient::from_settings(&s);
            client
                .estimate_smart_fee(6)
                .await
                .unwrap_or(send::DEFAULT_FEE_RATE_PER_BYTE)
        }
        None => send::DEFAULT_FEE_RATE_PER_BYTE,
    }
}

// --- sync ------------------------------------------------------------------

/// Refresh the local chain cache for a profile from the node: scan derived
/// addresses for coins, upsert UTXOs, reconcile spends, advance the cursor.
#[tauri::command]
pub async fn sync_wallet_state(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    // 1. Snapshot addresses + settings under the lock, then release it before
    //    any network I/O.
    let (profile_id, addresses, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let profile = match wallet_profile_id {
            Some(id) => db::queries::get_wallet_profile(&conn, &id)?
                .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))?,
            None => active_profile(&conn)?,
        };
        let addresses = db::queries::get_profile_addresses(&conn, &profile.id)?;
        let settings = db::queries::get_settings(&conn)?;
        (profile.id, addresses, settings)
    };

    let client = NodeRpcClient::from_settings(&settings);

    // Probe the node first. If it's unreachable, that's expected in explorer /
    // read-only mode: balances + names come from the explorer, so this is NOT an
    // error — we just can't refresh spendable UTXOs. Report it softly.
    let height = match client.get_blockchain_info().await {
        Ok(info) => info.blocks,
        Err(_) => {
            return Ok(serde_json::json!({
                "walletProfileId": profile_id,
                "nodeReachable": false,
                "message": "Node not connected. Balances and names are read from the \
                            explorer; start a local node to sync spendable coins and send.",
            }));
        }
    };

    // 2. Fetch coins per address (network I/O, no lock held).
    let mut all_coins = Vec::new();
    for addr in &addresses {
        match client.get_coins_by_address(addr).await {
            Ok(mut coins) => all_coins.append(&mut coins),
            Err(e) => {
                let url = settings
                    .get("node_rpc_url")
                    .map(|s| s.as_str())
                    .unwrap_or("the configured node");
                // A connection failure (no node listening) is reported by the RPC
                // client as AppError::Http; an actual RPC method error (e.g.
                // address index disabled) comes back as AppError::Rpc.
                return Err(match e {
                    AppError::Http(_) => AppError::Rpc(format!(
                        "Can't reach your local node at {url}. Start hsd (with --index-address) \
                         to sync and send. Reads still work via the explorer."
                    )),
                    other => AppError::Rpc(format!(
                        "getcoinsbyaddress failed for {addr} (is the node's --index-address enabled?): {other}"
                    )),
                });
            }
        }
    }

    // 3. Fetch the full body of each funding tx (network I/O, no lock held) so
    //    the transaction history can be served from cache.
    let mut txs: Vec<(String, Option<i64>, String)> = Vec::new();
    let mut seen_txids = std::collections::HashSet::new();
    for coin in &all_coins {
        if !seen_txids.insert(coin.txid.clone()) {
            continue;
        }
        if let Ok(raw) = client.get_raw_transaction(&coin.txid).await {
            txs.push((coin.txid.clone(), coin.height, raw.to_string()));
        }
    }

    // 4. Persist UTXOs + tx cache under the lock.
    let balances = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for coin in &all_coins {
            sync::upsert_utxo(&conn, &profile_id, coin)?;
            if let Some(addr) = &coin.address {
                sync::mark_address_used(&conn, &profile_id, addr, coin.height)?;
            }
        }
        sync::mark_missing_as_spent(&conn, &profile_id, &all_coins)?;
        for (txid, h, raw) in &txs {
            sync::cache_transaction(&conn, &profile_id, txid, *h, None, raw)?;
        }
        sync::set_sync_cursor(&conn, &profile_id, height)?;
        db::queries::update_profile_sync(&conn, &profile_id, height)?;
        sync::compute_balances(&conn, &profile_id)?
    };

    // 5. Refresh name states for known names (best-effort; never fails the sync).
    let names_synced = refresh_name_states(&state, &profile_id, &client)
        .await
        .unwrap_or(0);

    Ok(serde_json::json!({
        "walletProfileId": profile_id,
        "nodeReachable": true,
        "height": height,
        "utxoCount": all_coins.len(),
        "txsCached": txs.len(),
        "namesSynced": names_synced,
        "liquidDoos": balances.liquid,
        "nameControlDoos": balances.name_control,
        "nameLockupDoos": balances.name_lockup,
        "totalDoos": balances.total(),
    }))
}

/// Refresh `tracked_name_states` for a profile from the node. Candidates are the
/// names the wallet already tracks/owns (node-only RPC can't enumerate owned
/// names by address; node-free discovery + the coin scan find new ones). Returns
/// the number of names refreshed.
async fn refresh_name_states(
    state: &State<'_, AppState>,
    profile_id: &str,
    client: &NodeRpcClient,
) -> Result<usize, AppError> {
    // Only refresh on-chain state for names the wallet already tracks/owns — NOT
    // the whole migration inventory (that could be hundreds of `getnameinfo`
    // calls per sync). Newly-owned names are surfaced by node-free discovery and
    // the coin scan; inventory-vs-chain comparison uses the explorer directly.
    let candidates: Vec<String> = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::list_tracked_name_names(&conn, profile_id)?
            .into_iter()
            .filter(|n| !n.trim().is_empty())
            .collect()
    };

    let mut fetched: Vec<(String, serde_json::Value)> = Vec::new();
    for name in &candidates {
        if let Ok(info) = client.get_name_info(name).await {
            fetched.push((name.clone(), info));
        }
    }

    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for (name, info) in &fetched {
            sync::upsert_name_state(&conn, profile_id, name, info)?;
        }
    }
    Ok(fetched.len())
}

/// Standalone name-state refresh (also run as part of `sync_wallet_state`).
#[tauri::command]
pub async fn sync_tracked_names(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let (profile_id, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let id = match wallet_profile_id {
            Some(id) => id,
            None => db::queries::get_active_profile_id(&conn)?,
        };
        if id.is_empty() {
            return Err(AppError::InvalidInput("no active wallet profile".to_string()));
        }
        (id, db::queries::get_settings(&conn)?)
    };
    let client = NodeRpcClient::from_settings(&settings);
    let n = refresh_name_states(&state, &profile_id, &client).await?;
    Ok(serde_json::json!({ "walletProfileId": profile_id, "namesSynced": n }))
}

// --- build -----------------------------------------------------------------

/// Build (but do not sign) a plain HNS send. Runs coin selection for an accurate
/// fee/change preview and persists a `draft` row.
#[tauri::command]
pub async fn build_send_hns_draft(
    state: State<'_, AppState>,
    to_address: String,
    value_doos: i64,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if value_doos <= 0 {
        return Err(AppError::InvalidInput("amount must be positive".to_string()));
    }
    let amount = value_doos as u64;
    let rate = resolve_fee_rate(&state, fee_rate).await;

    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = active_profile(&conn)?;
    if profile.watch_only {
        return Err(AppError::InvalidInput(
            "active profile is watch-only and cannot send".to_string(),
        ));
    }
    let network = derivation::network_from_profile(&profile.network)?;

    // Validate destination early.
    crate::noncustodial::tx::output_address_from_string(network, &to_address)?;

    let coins = send::load_spendable_coins(&conn, &profile.id)?;
    let selection = send::select_coins(&coins, amount, rate)?;
    let change_addr = change_address(network, &profile.account_xpub)?;

    let summary = TxSummary {
        action: "send_hns".to_string(),
        send_total_doos: amount as i64,
        fee_doos: selection.fee as i64,
        change_doos: selection.change as i64,
        input_total_doos: selection.input_total as i64,
        num_inputs: selection.coins.len() as i64,
        recipient_address: Some(to_address.clone()),
        txid: None,
        warnings: Vec::new(),
    };
    let params = SendBuildParams {
        to_address,
        amount_doos: amount,
        change_address: change_addr,
        rate_per_byte: rate,
        account: profile.account_index as u32,
        network: profile.network.clone(),
    };

    let id = random_id();
    db::queries::insert_tx_draft(
        &conn,
        &id,
        &profile.id,
        "send_hns",
        "", // unsigned hex is materialized at sign time
        &serde_json::to_string(&params)?,
        &serde_json::to_string(&summary)?,
    )?;

    db::queries::get_tx_draft(&conn, &id)?
        .map(|d| d.to_summary())
        .ok_or_else(|| AppError::Other("draft vanished after insert".to_string()))
}

/// Preview the fee/change for a prospective send without persisting a draft.
#[tauri::command]
pub async fn estimate_tx_draft_fee(
    state: State<'_, AppState>,
    value_doos: i64,
    fee_rate: Option<u64>,
) -> Result<serde_json::Value, AppError> {
    if value_doos <= 0 {
        return Err(AppError::InvalidInput("amount must be positive".to_string()));
    }
    let rate = resolve_fee_rate(&state, fee_rate).await;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = active_profile(&conn)?;
    let coins = send::load_spendable_coins(&conn, &profile.id)?;
    let selection = send::select_coins(&coins, value_doos as u64, rate)?;
    Ok(serde_json::json!({
        "feeDoos": selection.fee,
        "changeDoos": selection.change,
        "inputTotalDoos": selection.input_total,
        "numInputs": selection.coins.len(),
    }))
}

// --- sign ------------------------------------------------------------------

/// Sign a draft using the unlocked signer session, materializing the signed tx.
#[tauri::command]
pub async fn sign_tx_draft(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<TxDraftSummary, AppError> {
    // 1. Load the draft + session ttl (send_hns also needs spendable coins).
    let (draft, coins, ttl_ms) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let draft = db::queries::get_tx_draft(&conn, &draft_id)?
            .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?;
        let coins = if draft.action == "send_hns" {
            send::load_spendable_coins(&conn, &draft.wallet_profile_id)?
        } else {
            Vec::new()
        };
        let settings = db::queries::get_settings(&conn)?;
        (draft, coins, session_ttl_ms(&settings))
    };

    // 2. Sign under the signer lock, dispatching by action.
    let (signed_hex, summary_json) = {
        let mut slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let session = slot.as_mut().ok_or(AppError::WalletLocked)?;
        if !session.is_unlocked() {
            return Err(AppError::WalletLocked);
        }
        if session.wallet_profile_id() != draft.wallet_profile_id {
            return Err(AppError::InvalidInput(
                "the unlocked signer is for a different wallet profile".to_string(),
            ));
        }
        session.touch(ttl_ms);

        if draft.action == "send_hns" {
            let params: SendBuildParams = serde_json::from_str(&draft.signing_inputs_json)?;
            let network = Network::from_str_opt(&params.network)
                .ok_or_else(|| AppError::InvalidInput(format!("bad network '{}'", params.network)))?;
            let built = send::build_send(
                session,
                network,
                params.account,
                &coins,
                &params.to_address,
                params.amount_doos,
                &params.change_address,
                params.rate_per_byte,
            )?;
            let summary = TxSummary {
                action: "send_hns".to_string(),
                send_total_doos: (built.output_total - built.change) as i64,
                fee_doos: built.fee as i64,
                change_doos: built.change as i64,
                input_total_doos: built.input_total as i64,
                num_inputs: built.num_inputs as i64,
                recipient_address: Some(params.to_address.clone()),
                txid: Some(built.txid.clone()),
                warnings: Vec::new(),
            };
            (built.tx_hex, serde_json::to_string(&summary)?)
        } else {
            // Covenant action: sign the persisted plan; keep its build-time summary.
            let plan: crate::noncustodial::actions::DraftPlan =
                serde_json::from_str(&draft.signing_inputs_json)?;
            let (hex, _txid) = crate::noncustodial::actions::sign_plan(session, &plan)?;
            (hex, draft.summary_json.clone())
        }
    };

    // 3. Persist the signed tx + summary.
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::update_tx_draft_signed(&conn, &draft_id, &signed_hex, &summary_json)?;
        db::queries::get_tx_draft(&conn, &draft_id)?
            .map(|d| d.to_summary())
            .ok_or_else(|| AppError::Other("draft vanished after sign".to_string()))
    }
}

// --- broadcast -------------------------------------------------------------

/// Broadcast a signed draft via node RPC.
#[tauri::command]
pub async fn broadcast_tx_draft(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<BroadcastResult, AppError> {
    let (signed_hex, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let draft = db::queries::get_tx_draft(&conn, &draft_id)?
            .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?;
        let signed = draft.signed_tx_hex.ok_or_else(|| {
            AppError::InvalidInput("draft is not signed yet".to_string())
        })?;
        let settings = db::queries::get_settings(&conn)?;
        (signed, settings)
    };

    // Any configured node (local OR remote) can broadcast — configuring a Node
    // RPC URL is the opt-in. The only refusal is a read-only Explorer source,
    // which `send_raw_transaction` rejects internally.
    let client = NodeRpcClient::from_settings(&settings);
    match client.send_raw_transaction(&signed_hex).await {
        Ok(txid) => {
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::update_tx_draft_status(&conn, &draft_id, "broadcasted", None, Some(&txid))?;
            Ok(BroadcastResult {
                draft_id,
                txid,
                status: "broadcasted".to_string(),
            })
        }
        Err(e) => {
            let msg = e.to_string();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::update_tx_draft_status(&conn, &draft_id, "failed", Some(&msg), None)?;
            Err(e)
        }
    }
}

/// Read cached balances for a profile (or the active profile) without touching
/// the node. Returns zeros when no profile is active.
#[tauri::command]
pub async fn get_wallet_balances(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let id = match wallet_profile_id {
        Some(id) => id,
        None => db::queries::get_active_profile_id(&conn)?,
    };
    if id.is_empty() {
        return Ok(serde_json::json!({
            "liquidDoos": 0, "nameControlDoos": 0, "nameLockupDoos": 0, "totalDoos": 0
        }));
    }
    let b = sync::compute_balances(&conn, &id)?;
    Ok(serde_json::json!({
        "liquidDoos": b.liquid,
        "nameControlDoos": b.name_control,
        "nameLockupDoos": b.name_lockup,
        "totalDoos": b.total(),
    }))
}

/// Report non-custodial write capability: writes require an unlocked signer AND
/// a broadcaster-capable node source. The frontend gates spend actions on this.
#[tauri::command]
pub async fn get_write_capability(
    state: State<'_, AppState>,
) -> Result<crate::providers::WriteCapability, AppError> {
    let signer_unlocked = {
        let slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        slot.as_ref().map(|s| s.is_unlocked()).unwrap_or(false)
    };
    let (source, allow_remote, settings, probe_addr) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&conn)?;
        let source = ChainSource::from_setting(
            settings.get("chain_source").map(|s| s.as_str()).unwrap_or("local_node"),
        );
        let allow_remote = settings.get("allow_remote_broadcast").map(|s| s.as_str()) == Some("true");
        // One address to probe the node's address index (if a profile exists).
        let probe_addr = active_profile(&conn)
            .ok()
            .and_then(|p| db::queries::get_profile_addresses(&conn, &p.id).ok())
            .and_then(|addrs| addrs.into_iter().next());
        (source, allow_remote, settings, probe_addr)
    };
    let mut cap =
        crate::providers::WriteCapability::evaluate(signer_unlocked, source, allow_remote);

    // Writes also need the node reachable, fully synced, AND address-indexed (the
    // wallet learns its spendable + name-owner coins via getcoinsbyaddress). If
    // any is missing, downgrade to read-only with a precise, actionable reason.
    if cap.can_write {
        let client = NodeRpcClient::from_settings(&settings);
        match client.get_blockchain_info().await {
            Err(_) => {
                let url = settings
                    .get("node_rpc_url")
                    .map(|s| s.as_str())
                    .unwrap_or("your node");
                cap.broadcaster_available = false;
                cap.can_write = false;
                cap.reason = Some(format!("Start your local node ({url}) to send."));
            }
            Ok(info) => {
                let synced = info.verification_progress.map(|p| p >= 0.9999).unwrap_or(true);
                if !synced {
                    let pct = (info.verification_progress.unwrap_or(0.0) * 100.0).floor() as i64;
                    cap.can_write = false;
                    cap.reason = Some(format!(
                        "Your local node is still syncing ({pct}%). On-chain sends and transfers need a fully-synced node."
                    ));
                } else if let Some(addr) = &probe_addr {
                    if client.get_coins_by_address(addr).await.is_err() {
                        cap.can_write = false;
                        cap.reason = Some(
                            "Your node isn't address-indexed — restart hsd with address indexing (Settings → Start hsd) and let it finish syncing."
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    Ok(cap)
}

/// List drafts for a profile (or the active profile).
#[tauri::command]
pub async fn list_tx_drafts(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<Vec<TxDraftSummary>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let id = match wallet_profile_id {
        Some(id) => id,
        None => db::queries::get_active_profile_id(&conn)?,
    };
    if id.is_empty() {
        return Ok(Vec::new());
    }
    db::queries::list_tx_drafts(&conn, &id)
}
