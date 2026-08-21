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
use tauri::{AppHandle, Runtime, State};

use crate::commands::secure_prompt::{prompt_secure, SecurePromptRequest};
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
    /// "Send Max": ignore `amount_doos`, sweep all coins, output = inputTotal−fee.
    #[serde(default)]
    max: bool,
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
        return Err(AppError::InvalidInput(
            "no active wallet profile".to_string(),
        ));
    }
    db::queries::get_wallet_profile(conn, &id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))
}

/// Derive the change address (branch 1, index 0) for a profile from its xpub.
fn change_address(network: Network, account_xpub: &str) -> Result<String, AppError> {
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

/// Format doos as an HNS decimal string (6 dp) for human display.
fn doos_to_hns_string(doos: i64) -> String {
    let whole = doos / 1_000_000;
    let frac = (doos % 1_000_000).abs();
    format!("{whole}.{frac:06} HNS")
}

/// Compute the TxSummary for a send_hns draft after signing.
/// Extracted for testability and to ensure fee/change calculations are consistent.
///
/// * `plan` — the signed plan (contains inputs, outputs, change_output_index).
/// * `unsigned_txid` — the txid of the unsigned tx (for display).
/// * `to_address` — the recipient address (for display).
///
/// Returns the summary or an error if change_output_index is invalid.
fn compute_send_summary(
    plan: &crate::noncustodial::actions::DraftPlan,
    unsigned_txid: String,
    to_address: String,
) -> Result<TxSummary, AppError> {
    let input_total: u64 = plan.inputs.iter().map(|i| i.value).sum();
    let output_total: u64 = plan.outputs.iter().map(|o| o.value).sum();

    // M2: Use checked_sub to catch corrupted drafts with inverted amounts.
    let fee = input_total.checked_sub(output_total).ok_or_else(|| {
        AppError::Other(
            "send plan: output_total exceeds input_total (corrupted draft?)".to_string(),
        )
    })?;

    // M1: Use plan.change_output_index instead of hardcoded [1].
    // This is critical for finalize-with-payment plans where change is at index 2.
    let change = plan
        .change_output_index
        .and_then(|idx| plan.outputs.get(idx).map(|o| o.value))
        .unwrap_or(0);

    Ok(TxSummary {
        action: "send_hns".to_string(),
        send_total_doos: (output_total - change) as i64,
        fee_doos: fee as i64,
        change_doos: change as i64,
        input_total_doos: input_total as i64,
        num_inputs: plan.inputs.len() as i64,
        recipient_address: Some(to_address),
        txid: Some(unsigned_txid),
        warnings: Vec::new(),
    })
}

/// Build the read-only detail rows shown in the secure confirmation window for
/// a draft. Rows are `{ "label": ..., "value": ... }`; the window renders them
/// verbatim so the user confirms the real on-chain intent, not whatever the
/// (possibly compromised) main webview claims.
fn confirm_details_for_draft(draft: &db::queries::TxDraftRow) -> serde_json::Value {
    let summary: TxSummary = serde_json::from_str(&draft.summary_json).unwrap_or(TxSummary {
        action: draft.action.clone(),
        send_total_doos: 0,
        fee_doos: 0,
        change_doos: 0,
        input_total_doos: 0,
        num_inputs: 0,
        recipient_address: None,
        txid: None,
        warnings: Vec::new(),
    });
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let action_label = match draft.action.as_str() {
        "send_hns" => "Send HNS".to_string(),
        other => format!("Name action: {other}"),
    };
    rows.push(serde_json::json!({ "label": "Action", "value": action_label }));
    if draft.action == "send_hns" {
        if let Some(to) = &summary.recipient_address {
            rows.push(serde_json::json!({ "label": "To", "value": to }));
        }
        rows.push(serde_json::json!({
            "label": "Amount",
            "value": doos_to_hns_string(summary.send_total_doos),
        }));
    }
    rows.push(serde_json::json!({
        "label": "Fee",
        "value": doos_to_hns_string(summary.fee_doos),
    }));
    if let Some(txid) = &summary.txid {
        rows.push(serde_json::json!({ "label": "Txid", "value": txid }));
    }
    for w in &summary.warnings {
        rows.push(serde_json::json!({ "label": "Warning", "value": w }));
    }
    serde_json::json!({ "rows": rows })
}

/// Resolve the fee rate (doos/byte): explicit override, else ask the node's
/// `fee_rate_doos_per_kvb` setting (in doos per 1000 vbytes; divide by 1000
/// to get doos/byte, floored at the relay-minimum), else the node's
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
            // 1) Explicit user setting wins: convert doos/kvB → doos/byte and
            //    floor at the relay minimum. This mirrors names::fee_rate() so
            //    the send flow and the covenant flows use the same knob.
            if let Some(kvb) = s
                .get("fee_rate_doos_per_kvb")
                .and_then(|v| v.parse::<u64>().ok())
            {
                return (kvb / 1000).max(send::MIN_FEE_RATE_PER_BYTE);
            }
            // 2) Ask the node for an estimate — same behavior as before.
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
        let mut addresses = db::queries::get_profile_addresses(&conn, &profile.id)?;
        // Auto-provision derived addresses if none exist yet (e.g., wallet
        // created before the address-derivation step ran, or DB was migrated).
        if addresses.is_empty() {
            if let Ok(network) = derivation::network_from_profile(&profile.network) {
                if let Ok(xpub) = crate::noncustodial::hd::ExtendedPubKey::from_xpub(
                    network,
                    &profile.account_xpub,
                ) {
                    if let Ok(recv) = derivation::ensure_addresses(
                        &conn,
                        &profile.id,
                        0,
                        network,
                        &xpub,
                        derivation::BRANCH_RECEIVE,
                        20,
                    ) {
                        let _ = derivation::ensure_addresses(
                            &conn,
                            &profile.id,
                            0,
                            network,
                            &xpub,
                            derivation::BRANCH_CHANGE,
                            20,
                        );
                        addresses = recv.into_iter().map(|d| d.address).collect();
                    }
                }
            }
        }
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
    let node_url = settings
        .get("node_rpc_url")
        .map(|s| s.as_str())
        .unwrap_or("the configured node");
    let (all_coins, txs) =
        fetch_wallet_coins_and_txs_with_client(&client, &addresses, node_url).await?;

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
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
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
            return Err(AppError::InvalidInput(
                "no active wallet profile".to_string(),
            ));
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
    max: Option<bool>,
) -> Result<TxDraftSummary, AppError> {
    let is_max = max.unwrap_or(false);
    if !is_max && value_doos <= 0 {
        return Err(AppError::InvalidInput(
            "amount must be positive".to_string(),
        ));
    }
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

    let coins = send::load_spendable_coins(&conn, &profile.id, None)?;
    // Send Max sweeps all coins (output = inputTotal − fee, no change); otherwise
    // select to cover the requested amount + fee.
    let selection = if is_max {
        send::select_all_coins(&coins, rate)?
    } else {
        send::select_coins(&coins, value_doos as u64, rate)?
    };
    let amount = if is_max {
        selection.input_total - selection.fee
    } else {
        value_doos as u64
    };
    let change_addr = change_address(network, &profile.account_xpub)?;

    // Materialize the plan now and record its txid in the build-time summary.
    // Handshake txids hash only the non-witness serialization, so this txid is
    // final regardless of signing method. Signing recomputes the plan fresh
    // (coins may need re-fetching) — comparing its txid against this
    // persisted, build-time value is what actually verifies the signed tx
    // matches what the user previewed, instead of comparing a value against
    // itself.
    let build_time_txid = {
        let plan = send::build_send_plan(
            network,
            profile.account_index as u32,
            &coins,
            &to_address,
            amount,
            &change_addr,
            rate,
            is_max,
        )?;
        crate::noncustodial::actions::rebuild_unsigned(&plan, network)?.txid()
    };

    let summary = TxSummary {
        action: "send_hns".to_string(),
        send_total_doos: amount as i64,
        fee_doos: selection.fee as i64,
        change_doos: selection.change as i64,
        input_total_doos: selection.input_total as i64,
        num_inputs: selection.coins.len() as i64,
        recipient_address: Some(to_address.clone()),
        txid: Some(build_time_txid),
        warnings: Vec::new(),
    };
    let params = SendBuildParams {
        to_address,
        amount_doos: amount,
        change_address: change_addr,
        rate_per_byte: rate,
        account: profile.account_index as u32,
        network: profile.network.clone(),
        max: is_max,
    };

    let id = random_id();
    let reserved_inputs: Vec<(String, u32)> = selection
        .coins
        .iter()
        .map(|c| (c.txid.clone(), c.vout))
        .collect();
    db::queries::insert_tx_draft_reserving_coins(
        &conn,
        &id,
        &profile.id,
        "send_hns",
        "", // unsigned hex is materialized at sign time
        &serde_json::to_string(&params)?,
        &serde_json::to_string(&summary)?,
        &reserved_inputs,
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
        return Err(AppError::InvalidInput(
            "amount must be positive".to_string(),
        ));
    }
    let rate = resolve_fee_rate(&state, fee_rate).await;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = active_profile(&conn)?;
    let coins = send::load_spendable_coins(&conn, &profile.id, None)?;
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
    app: AppHandle,
    draft_id: String,
) -> Result<TxDraftSummary, AppError> {
    sign_tx_draft_confirmed(&state, &app, &draft_id).await
}

/// Confirmation-gated signing, generic over the Tauri runtime so tests using
/// the mock runtime can drive it. Requires an explicit per-transaction
/// confirmation in the Rust-owned secure window BEFORE any signing. A
/// compromised/injected main webview can invoke this while the session is
/// unlocked, but it cannot forge the confirmation (a separate window it does
/// not control) and the window shows the real tx details so the user can
/// catch a swapped draft.
pub(crate) async fn sign_tx_draft_confirmed<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    draft_id: &str,
) -> Result<TxDraftSummary, AppError> {
    let draft = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_tx_draft(&conn, draft_id)?
            .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?
    };
    let details = confirm_details_for_draft(&draft);
    let confirm = prompt_secure(
        app,
        SecurePromptRequest {
            mode: "confirm".to_string(),
            title: "Confirm transaction".to_string(),
            message: "Review these details. This will sign and broadcast a transaction."
                .to_string(),
            details: Some(details),
            ..Default::default()
        },
    )
    .await?;
    if !confirm.confirmed {
        return Err(AppError::UserRejected);
    }
    sign_tx_draft_inner(state, draft_id).await
}

/// The signing core, WITHOUT the secure-window confirmation. Not a Tauri
/// command — callers must have already obtained user confirmation (the
/// `sign_tx_draft` command does this). Kept separate so tests can drive signing
/// deterministically without opening a secure window.
pub(crate) async fn sign_tx_draft_inner(
    state: &State<'_, AppState>,
    draft_id: &str,
) -> Result<TxDraftSummary, AppError> {
    // 1. Load the draft + profile kind + session ttl. Also load spendable coins
    //    for the send path.
    let (draft, profile_kind, account_xpub, coins, ttl_ms, covenant_names) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let draft = db::queries::get_tx_draft(&conn, draft_id)?
            .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?;
        let profile = db::queries::get_wallet_profile(&conn, &draft.wallet_profile_id)?
            .ok_or_else(|| {
                AppError::NotFound(format!("wallet profile {}", draft.wallet_profile_id))
            })?;
        let profile_kind = profile.kind.clone();
        let account_xpub = profile.account_xpub.clone();
        let coins = if draft.action == "send_hns" {
            // Prefer the exact coin set this draft reserved at build time (I3):
            // re-selecting over the full pool could drift onto a larger coin
            // that synced in after the build — an input that was never
            // reserved and that another draft could claim before this one
            // broadcasts. Selecting over the reserved set alone is
            // deterministic and reproduces the previewed inputs. Fall back to
            // the general pool (still excluding other drafts' reservations)
            // for drafts holding no reservation rows — e.g. drafts created
            // before migration 015, or whose reservation TTL-expired.
            let reserved = send::load_reserved_coins(&conn, &draft.wallet_profile_id, draft_id)?;
            if reserved.is_empty() {
                send::load_spendable_coins(&conn, &draft.wallet_profile_id, Some(draft_id))?
            } else {
                reserved
            }
        } else {
            Vec::new()
        };
        let settings = db::queries::get_settings(&conn)?;
        // For Ledger covenant actions, resolve names by hash from tracked_name_states.
        let covenant_names = if profile_kind == "ledger_hardware" && draft.action != "send_hns" {
            crate::providers::ledger::resolve_covenant_names(
                &conn,
                &draft.signing_inputs_json,
                &profile.id,
            )?
        } else {
            Vec::new()
        };
        (
            draft,
            profile_kind,
            account_xpub,
            coins,
            session_ttl_ms(&settings),
            covenant_names,
        )
    };

    // 2. Sign, dispatching by profile kind (ledger vs hot) and action.
    let (signed_hex, summary_json) = if profile_kind == "ledger_hardware" {
        sign_via_ledger(&draft, &account_xpub, &coins, &covenant_names).await?
    } else {
        sign_via_hot_session(state, &draft, &coins, ttl_ms)?
    };

    // 3. Persist the signed tx and status.
    persist_signed_draft(state, draft_id, &signed_hex, &summary_json)?;

    load_draft_summary(state, draft_id)
}

/// Persist the signed hex + summary and flip the draft's status to `signed`.
fn persist_signed_draft(
    state: &State<'_, AppState>,
    draft_id: &str,
    signed_hex: &str,
    summary_json: &str,
) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::update_tx_draft_signed(&conn, draft_id, signed_hex, summary_json)?;
    Ok(())
}

/// The hot-wallet signing path — unchanged behaviour from before the Ledger
/// integration. Locks the in-memory signer session and dispatches by action.
fn sign_via_hot_session(
    state: &State<'_, AppState>,
    draft: &db::queries::TxDraftRow,
    coins: &[send::SpendableCoin],
    ttl_ms: u128,
) -> Result<(String, String), AppError> {
    {
        let mut slot = state
            .signer
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
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
            let network = Network::from_str_opt(&params.network).ok_or_else(|| {
                AppError::InvalidInput(format!("bad network '{}'", params.network))
            })?;
            let built = send::build_send(
                session,
                network,
                params.account,
                coins,
                &params.to_address,
                params.amount_doos,
                &params.change_address,
                params.rate_per_byte,
                params.max,
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
            Ok((built.tx_hex, serde_json::to_string(&summary)?))
        } else {
            // Covenant action: sign the persisted plan; keep its build-time summary.
            let plan: crate::noncustodial::actions::DraftPlan =
                serde_json::from_str(&draft.signing_inputs_json)?;
            let (hex, txid) = crate::noncustodial::actions::sign_plan(session, &plan)?;
            // The build-time summary already carries the txid (Handshake txids
            // hash only the non-witness serialization, so signing can't change
            // it) — sign_plan's recomputation must agree, or the stored txid
            // the confirmation tracker polls for would be wrong.
            debug_assert_eq!(
                local_txid_from_summary(&draft.summary_json).as_deref(),
                Some(txid.as_str()),
                "covenant draft summary txid diverged from the signed tx's txid"
            );
            Ok((hex, draft.summary_json.clone()))
        }
    }
}

/// Build `ChangeInfo` for a plan's claimed change output, but only after
/// verifying it really is our own change: re-derive the change address
/// (branch 1, index 0) from the account xpub and require it to match
/// `plan.outputs[idx].address`. Never trust `change_output_index` in
/// isolation — a wrong index would tell the device to hide an external
/// payee from the user's on-screen review instead of the actual change.
fn verify_ledger_change_output(
    plan: &crate::noncustodial::actions::DraftPlan,
    network: Network,
    account_xpub: &crate::noncustodial::hd::ExtendedPubKey,
) -> Result<Option<crate::providers::ledger::parse_mode::ChangeInfo>, AppError> {
    use crate::noncustodial::hd::bip44_path;
    use crate::providers::ledger::parse_mode::ChangeInfo;

    match plan.change_output_index {
        Some(idx) => {
            let output = plan.outputs.get(idx).ok_or_else(|| {
                AppError::Other(format!(
                    "ledger plan: change_output_index {idx} is out of bounds ({} outputs)",
                    plan.outputs.len()
                ))
            })?;
            let derived =
                derivation::derive_one(network, account_xpub, derivation::BRANCH_CHANGE, 0)?;
            if derived.address != output.address {
                return Err(AppError::Other(
                    "ledger plan: claimed change output does not match the account's own change address — refusing to sign".to_string(),
                ));
            }
            Ok(Some(ChangeInfo {
                output_index: idx as u8,
                address_version: 0, // p2wpkh
                path: bip44_path(network, plan.account, 1, 0).to_vec(),
            }))
        }
        None => Ok(None),
    }
}

/// Verify the connected Ledger's account xpub matches the wallet profile's
/// stored one. Without this, a different Ledger plugged in would sign with
/// its own keys while witness pubkeys (derived from the stored account_xpub)
/// stay the profile's — the draft is marked signed, coins stay reserved, and
/// broadcast fails on a script mismatch.
fn verify_ledger_device_identity(
    device_pubkey: &[u8; 33],
    device_chain_code: &[u8; 32],
    account_xpub: &crate::noncustodial::hd::ExtendedPubKey,
) -> Result<(), AppError> {
    let device_xpub =
        crate::noncustodial::hd::ExtendedPubKey::from_parts(device_pubkey, device_chain_code)?;
    if device_xpub.public != account_xpub.public
        || device_xpub.chain_code != account_xpub.chain_code
    {
        return Err(AppError::Other(
            "connected Ledger does not match this wallet profile's account xpub — wrong device plugged in? refusing to sign".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod ledger_signing_guards_tests {
    use super::*;
    use crate::noncustodial::actions::{DraftPlan, PlanOutput};
    use crate::noncustodial::hd::ExtendedPubKey;

    // Deterministic test-only "account xpub": derivation semantics (branch/
    // index) are identical regardless of depth, so using the master node
    // directly is fine for these tests (mirrors derivation.rs's test_xpub()).
    fn test_xpub() -> ExtendedPubKey {
        let seed = crate::noncustodial::hd::seed_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
        )
        .unwrap();
        let master = crate::noncustodial::hd::ExtendedPrivKey::from_seed(&seed).unwrap();
        ExtendedPubKey::from_priv(&master)
    }

    fn plan_with_outputs(
        outputs: Vec<PlanOutput>,
        change_output_index: Option<usize>,
    ) -> DraftPlan {
        DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "mainnet".to_string(),
            inputs: Vec::new(),
            outputs,
            change_output_index,
        }
    }

    fn plain_output(value: u64, address: &str) -> PlanOutput {
        PlanOutput {
            value,
            address: address.to_string(),
            covenant_type: 0,
            covenant_items_hex: Vec::new(),
        }
    }

    #[test]
    fn change_output_none_when_plan_has_no_change() {
        let xpub = test_xpub();
        let plan = plan_with_outputs(vec![plain_output(1000, "hs1qrecipient")], None);
        let info = verify_ledger_change_output(&plan, Network::Main, &xpub).unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn change_output_accepted_when_address_matches_derived_change() {
        let xpub = test_xpub();
        let change_addr =
            derivation::derive_one(Network::Main, &xpub, derivation::BRANCH_CHANGE, 0)
                .unwrap()
                .address;
        // Regression for the finalize-with-payment bug: change is NOT always
        // at index 1. Put it at index 2, behind a covenant output and a
        // third-party payment output, and confirm the guard follows the
        // index rather than assuming a fixed position.
        let plan = plan_with_outputs(
            vec![
                plain_output(500, "hs1qcovenant"),
                plain_output(2000, "hs1qthird-party-payment"),
                plain_output(1000, &change_addr),
            ],
            Some(2),
        );
        let info = verify_ledger_change_output(&plan, Network::Main, &xpub)
            .unwrap()
            .expect("change output should be recognized");
        assert_eq!(info.output_index, 2);
    }

    #[test]
    fn change_output_rejected_when_index_points_at_a_third_party_output() {
        // This is exactly the CRITICAL bug: change_output_index hardcoded to
        // 1 while the real change sits at index 2 (finalize-with-payment).
        // outputs[1] is a payment to a third party, not our own change — the
        // guard must refuse to treat it as change.
        let xpub = test_xpub();
        let change_addr =
            derivation::derive_one(Network::Main, &xpub, derivation::BRANCH_CHANGE, 0)
                .unwrap()
                .address;
        let plan = plan_with_outputs(
            vec![
                plain_output(500, "hs1qcovenant"),
                plain_output(2000, "hs1qthird-party-payment"),
                plain_output(1000, &change_addr),
            ],
            Some(1), // wrong: this is the third-party payment, not change
        );
        let err = verify_ledger_change_output(&plan, Network::Main, &xpub).unwrap_err();
        assert!(
            matches!(err, AppError::Other(ref msg) if msg.contains("does not match")),
            "expected a 'does not match' refusal, got {err:?}"
        );
    }

    #[test]
    fn change_output_rejected_when_index_out_of_bounds() {
        let xpub = test_xpub();
        let plan = plan_with_outputs(vec![plain_output(1000, "hs1qrecipient")], Some(5));
        let err = verify_ledger_change_output(&plan, Network::Main, &xpub).unwrap_err();
        assert!(
            matches!(err, AppError::Other(ref msg) if msg.contains("out of bounds")),
            "expected an 'out of bounds' refusal, got {err:?}"
        );
    }

    #[test]
    fn device_identity_accepted_when_pubkey_and_chain_code_match() {
        let xpub = test_xpub();
        let pubkey = xpub.public.serialize();
        let chain_code = xpub.chain_code;
        verify_ledger_device_identity(&pubkey, &chain_code, &xpub)
            .expect("matching device identity should be accepted");
    }

    #[test]
    fn device_identity_rejected_when_a_different_device_is_connected() {
        let profile_xpub = test_xpub();
        // A different device: same derivation machinery, different seed —
        // produces a different pubkey/chain code entirely.
        let other_seed = crate::noncustodial::hd::seed_from_mnemonic(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
            "",
        )
        .unwrap();
        let other_master =
            crate::noncustodial::hd::ExtendedPrivKey::from_seed(&other_seed).unwrap();
        let other_xpub = ExtendedPubKey::from_priv(&other_master);
        let pubkey = other_xpub.public.serialize();
        let chain_code = other_xpub.chain_code;

        let err = verify_ledger_device_identity(&pubkey, &chain_code, &profile_xpub).unwrap_err();
        assert!(
            matches!(err, AppError::Other(ref msg) if msg.contains("does not match this wallet profile")),
            "expected a device-mismatch refusal, got {err:?}"
        );
    }

    #[test]
    fn compute_send_summary_with_change_at_index_1() {
        // Standard send: recipient at [0], change at [1].
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "mainnet".to_string(),
            inputs: vec![crate::noncustodial::actions::PlanInput {
                txid: "aa".repeat(32),
                vout: 0,
                value: 100_000_000,
                branch: 0,
                child_index: 0,
                sighash_type: 1,
            }],
            outputs: vec![
                crate::noncustodial::actions::PlanOutput {
                    value: 90_000_000,
                    address: "hs1qrecipient".to_string(),
                    covenant_type: 0,
                    covenant_items_hex: Vec::new(),
                },
                crate::noncustodial::actions::PlanOutput {
                    value: 9_500_000,
                    address: "hs1qchange".to_string(),
                    covenant_type: 0,
                    covenant_items_hex: Vec::new(),
                },
            ],
            change_output_index: Some(1),
        };
        let summary =
            compute_send_summary(&plan, "abc123".to_string(), "hs1qrecipient".to_string()).unwrap();
        assert_eq!(summary.send_total_doos, 90_000_000);
        assert_eq!(summary.change_doos, 9_500_000);
        assert_eq!(summary.fee_doos, 500_000);
        assert_eq!(summary.input_total_doos, 100_000_000);
    }

    #[test]
    fn compute_send_summary_with_change_at_index_2_finalize_with_payment() {
        // Regression for M1: finalize-with-payment has covenant at [0],
        // payment at [1], change at [2]. The old hardcoded [1] would
        // misreport the payment as change.
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "mainnet".to_string(),
            inputs: vec![crate::noncustodial::actions::PlanInput {
                txid: "aa".repeat(32),
                vout: 0,
                value: 100_000_000,
                branch: 0,
                child_index: 0,
                sighash_type: 1,
            }],
            outputs: vec![
                crate::noncustodial::actions::PlanOutput {
                    value: 1_000_000,
                    address: "hs1qcovenant".to_string(),
                    covenant_type: 2, // REVEAL
                    covenant_items_hex: vec!["deadbeef".to_string()],
                },
                crate::noncustodial::actions::PlanOutput {
                    value: 50_000_000,
                    address: "hs1qpayment".to_string(),
                    covenant_type: 0,
                    covenant_items_hex: Vec::new(),
                },
                crate::noncustodial::actions::PlanOutput {
                    value: 48_500_000,
                    address: "hs1qchange".to_string(),
                    covenant_type: 0,
                    covenant_items_hex: Vec::new(),
                },
            ],
            change_output_index: Some(2),
        };
        let summary =
            compute_send_summary(&plan, "def456".to_string(), "hs1qpayment".to_string()).unwrap();
        // send_total = everything leaving the wallet that is NOT change
        // = covenant(1M) + payment(50M) = 51M. The key regression assertion
        // is that change is correctly read from index 2 (48.5M), NOT the
        // index-1 payment (50M) the old hardcoded code would have used.
        assert_eq!(summary.send_total_doos, 51_000_000);
        assert_eq!(summary.change_doos, 48_500_000);
        assert_eq!(summary.fee_doos, 500_000);
    }

    #[test]
    fn compute_send_summary_no_change() {
        // Sweep: all inputs go to recipient, no change.
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "mainnet".to_string(),
            inputs: vec![crate::noncustodial::actions::PlanInput {
                txid: "aa".repeat(32),
                vout: 0,
                value: 100_000_000,
                branch: 0,
                child_index: 0,
                sighash_type: 1,
            }],
            outputs: vec![crate::noncustodial::actions::PlanOutput {
                value: 99_500_000,
                address: "hs1qrecipient".to_string(),
                covenant_type: 0,
                covenant_items_hex: Vec::new(),
            }],
            change_output_index: None,
        };
        let summary =
            compute_send_summary(&plan, "xyz789".to_string(), "hs1qrecipient".to_string()).unwrap();
        assert_eq!(summary.send_total_doos, 99_500_000);
        assert_eq!(summary.change_doos, 0);
        assert_eq!(summary.fee_doos, 500_000);
    }

    #[test]
    fn compute_send_summary_rejects_corrupted_inverted_amounts() {
        // Regression for M2: output_total > input_total (corrupted draft).
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "mainnet".to_string(),
            inputs: vec![crate::noncustodial::actions::PlanInput {
                txid: "aa".repeat(32),
                vout: 0,
                value: 50_000_000, // small input
                branch: 0,
                child_index: 0,
                sighash_type: 1,
            }],
            outputs: vec![crate::noncustodial::actions::PlanOutput {
                value: 100_000_000, // output > input (impossible)
                address: "hs1qrecipient".to_string(),
                covenant_type: 0,
                covenant_items_hex: Vec::new(),
            }],
            change_output_index: None,
        };
        let err = compute_send_summary(&plan, "bad".to_string(), "hs1q".to_string()).unwrap_err();
        assert!(
            matches!(err, AppError::Other(ref msg) if msg.contains("corrupted draft")),
            "expected a 'corrupted draft' error, got {err:?}"
        );
    }
}

/// The Ledger-hardware signing path. No in-memory signer session; instead
/// connects to the physical device and drives the parse+sign APDU dance.
/// Blocking HID I/O runs on a `spawn_blocking` task.
async fn sign_via_ledger(
    draft: &db::queries::TxDraftRow,
    account_xpub_str: &str,
    coins: &[send::SpendableCoin],
    covenant_names: &[(usize, String)],
) -> Result<(String, String), AppError> {
    use crate::noncustodial::hd::ExtendedPubKey;
    use crate::providers::ledger::{signing::sign_transaction, LedgerSigner};

    // For send_hns, build a plan from the coin selection so both paths share
    // the same signing entry point.
    let (plan, summary_json, txid_expected) = if draft.action == "send_hns" {
        let params: SendBuildParams = serde_json::from_str(&draft.signing_inputs_json)?;
        let network = Network::from_str_opt(&params.network)
            .ok_or_else(|| AppError::InvalidInput(format!("bad network '{}'", params.network)))?;
        let plan = send::build_send_plan(
            network,
            params.account,
            coins,
            &params.to_address,
            params.amount_doos,
            &params.change_address,
            params.rate_per_byte,
            params.max,
        )?;
        // Compute the expected summary + txid from the unsigned tx so the
        // frontend sees the same fee/change it did at build time.
        let unsigned = crate::noncustodial::actions::rebuild_unsigned(&plan, network)?;
        let summary = compute_send_summary(&plan, unsigned.txid(), params.to_address.clone())?;
        // The expected txid must come from build_send_hns_draft's persisted
        // summary, not from this freshly-rebuilt plan: comparing the plan's
        // own txid against itself can never disagree, so it wouldn't catch a
        // rebuild that silently diverges from what the user previewed at
        // build time (different coins, different fee/change, a bug in
        // build_send_plan between build and sign). See local_txid_from_summary.
        let expected = local_txid_from_summary(&draft.summary_json).ok_or_else(|| {
            AppError::Other(
                "send_hns draft summary is missing a txid — refusing to sign".to_string(),
            )
        })?;
        (plan, serde_json::to_string(&summary)?, Some(expected))
    } else {
        // Covenant action: plan is already persisted. The build-time summary
        // carries the txid (Handshake txids hash the non-witness serialization
        // only, so signatures can't change it). Extract it as the expected
        // txid — we compare it against the device-returned txid below to catch
        // any parse-mode blob corruption or divergence between the plan we
        // sent and what the device actually signed.
        let plan: crate::noncustodial::actions::DraftPlan =
            serde_json::from_str(&draft.signing_inputs_json)?;
        let expected = local_txid_from_summary(&draft.summary_json).ok_or_else(|| {
            AppError::Other(
                "covenant draft summary is missing a txid — refusing to sign".to_string(),
            )
        })?;
        (plan, draft.summary_json.clone(), Some(expected))
    };

    let network = Network::from_str_opt(&plan.network)
        .ok_or_else(|| AppError::InvalidInput(format!("bad network '{}'", plan.network)))?;

    // Parse the stored xpub for local pubkey derivation.
    let account_xpub = ExtendedPubKey::from_xpub(network, account_xpub_str)?;

    // Build OutputName entries from the pre-resolved covenant names.
    let names = output_names_from_pairs(covenant_names);

    let change_info = verify_ledger_change_output(&plan, network, &account_xpub)?;

    // Blocking HID I/O runs on the blocking thread pool.
    let (hex, txid) = tokio::task::spawn_blocking(move || {
        let mut signer = LedgerSigner::connect()?;
        // Verify the app is present and reachable.
        let _version = signer.get_app_version()?;
        // No on-device confirmation needed; the xpub disclosure was already
        // approved at import time.
        let (device_pubkey, device_chain_code) =
            signer.get_account_pubkey(network, plan.account, false)?;
        verify_ledger_device_identity(&device_pubkey, &device_chain_code, &account_xpub)?;
        sign_transaction(
            &mut signer,
            &plan,
            &account_xpub,
            network,
            change_info.as_ref(),
            &names,
        )
    })
    .await
    .map_err(|e| AppError::Other(format!("ledger signing task failed: {e}")))??;

    // Verify the device signed the transaction the user previewed.
    // Handshake txids hash the non-witness serialization only, so a mismatch
    // means the device parsed a different tx than we sent — never broadcast it.
    // This check is mandatory for both send_hns and every covenant action.
    let expected = txid_expected.ok_or_else(|| {
        AppError::Other("internal: expected txid was not computed before signing".to_string())
    })?;
    if expected != txid {
        return Err(AppError::Other(format!(
            "ledger signed txid {txid} != expected {expected} — refusing to broadcast"
        )));
    }

    Ok((hex, summary_json))
}

/// Convert pre-resolved `(output_index, name)` pairs into the
/// [`OutputName`](crate::providers::ledger::parse_mode::OutputName) entries
/// that the parse-mode builder expects.
fn output_names_from_pairs(
    pairs: &[(usize, String)],
) -> Vec<crate::providers::ledger::parse_mode::OutputName> {
    pairs
        .iter()
        .map(
            |(idx, name)| crate::providers::ledger::parse_mode::OutputName {
                output_index: *idx,
                name: name.clone(),
            },
        )
        .collect()
}

/// Load the summary of the just-signed draft for the return value.
fn load_draft_summary(
    state: &State<'_, AppState>,
    draft_id: &str,
) -> Result<TxDraftSummary, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let draft = db::queries::get_tx_draft(&conn, draft_id)?
        .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?;
    Ok(draft.to_summary())
}

/// Sign an arbitrary message with the wallet key that owns `name`, reproducing
/// hsd's `signmessagewithname` byte-for-byte (see `noncustodial::message`).
/// Used to satisfy third-party domain-claim verification (e.g. Namebase),
/// which asks the user to sign an exact message with the owning key and paste
/// the resulting signature back.
///
/// Not a spend: builds and broadcasts nothing. Requires the signer to be
/// unlocked for the SAME wallet profile that owns `name` — the private key
/// never leaves this function.
#[tauri::command]
pub async fn sign_name_message(
    state: State<'_, AppState>,
    name: String,
    message: String,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let id = crate::commands::read::resolve_profile(&state, wallet_profile_id)?
        .ok_or_else(|| AppError::InvalidInput("no wallet profile selected".to_string()))?;

    // 1. DB: resolve the account index + the name's owner coin (branch/index),
    // and the signer-session ttl. `get_name_coin` only returns a hit when this
    // profile currently holds the name's owner UTXO (mirrors hsd `isClosed` —
    // no coin, no proof of ownership to sign for).
    let (account, coin, ttl_ms) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let profile = db::queries::get_wallet_profile(&conn, &id)?
            .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))?;
        let coin = db::queries::get_name_coin(&conn, &id, &name)?.ok_or_else(|| {
            AppError::InvalidInput(format!("wallet does not own '{name}' (sync/own it first)"))
        })?;
        let settings = db::queries::get_settings(&conn)?;
        (
            profile.account_index as u32,
            coin,
            session_ttl_ms(&settings),
        )
    };

    // 2. Signer: mirror `sign_tx_draft`'s unlock + per-profile gate — the
    // unlocked session must belong to the SAME profile the coin was resolved
    // under, so one wallet's unlocked signer can never sign on another's
    // behalf.
    let (signature, pubkey) = {
        let mut slot = state
            .signer
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        let session = slot.as_mut().ok_or(AppError::WalletLocked)?;
        if !session.is_unlocked() {
            return Err(AppError::WalletLocked);
        }
        if session.wallet_profile_id() != id {
            return Err(AppError::InvalidInput(
                "the unlocked signer is for a different wallet profile".to_string(),
            ));
        }
        session.touch(ttl_ms);

        let network = session.network();
        let path =
            crate::noncustodial::hd::bip44_path(network, account, coin.branch, coin.child_index);
        let child = session.master()?.derive_path(&path)?;
        let signature =
            crate::noncustodial::message::sign_handshake_message(&child.secret, &message);
        let pubkey = child.compressed_pubkey();
        (signature, pubkey)
    };

    Ok(serde_json::json!({
        "signature": signature,
        "publicKey": hex::encode(pubkey),
        "address": coin.address,
    }))
}

// --- broadcast -------------------------------------------------------------

/// Outcome of a broadcast attempt, as classified by
/// [`classify_broadcast_outcome_with_client`].
#[derive(Debug)]
pub(crate) enum BroadcastOutcome {
    /// Node accepted the tx and returned a txid.
    Success(String),
    /// Node answered with a JSON-RPC error (double-spend, malformed, etc.) —
    /// the tx was definitively rejected and coins are unspent. The wrapped
    /// `AppError` is always the original `AppError::Rpc(_)` from the client.
    RpcError(AppError),
    /// HTTP/transport failure (timeout, connection dropped, DNS, etc.) —
    /// the outcome is ambiguous; the tx may be in the node's mempool. The
    /// wrapped `AppError` preserves the original variant (e.g. `Http`,
    /// `InvalidInput` for read-only sources) so the caller can propagate the
    /// exact error type to the frontend.
    TransportError(AppError),
}

/// Client-injected broadcast outcome classification for [`broadcast_tx_draft`].
/// Calls `send_raw_transaction` and classifies the result into three
/// categories: success (txid), RPC error (definitive rejection), or transport
/// error (ambiguous). Testable against a mock.
pub(crate) async fn classify_broadcast_outcome_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    signed_hex: &str,
) -> BroadcastOutcome {
    match client.send_raw_transaction(signed_hex).await {
        Ok(txid) => BroadcastOutcome::Success(txid),
        Err(e @ AppError::Rpc(_)) => BroadcastOutcome::RpcError(e),
        Err(e) => BroadcastOutcome::TransportError(e),
    }
}

/// Client-injected RPC-fetch phase of [`sync_wallet_state`]. Given the wallet's
/// watch addresses, fetches every coin via `getcoinsbyaddress` per address, then
/// fetches the raw body of each distinct funding tx via `getrawtransaction`.
/// Returns `(all_coins, txs)` where `txs` is `(txid, height, raw_json_string)`.
/// The caller persists both to the DB.
///
/// Errors classify like the original: `AppError::Http` (transport) is mapped to
/// a "start hsd" hint, all other `getcoinsbyaddress` errors carry an
/// "is --index-address enabled?" hint. Failed `getrawtransaction` calls are
/// silently skipped — the tx cache is best-effort, not authoritative.
///
/// Testable against a mock without an AppState.
pub(crate) async fn fetch_wallet_coins_and_txs_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    addresses: &[String],
    node_url: &str,
) -> Result<
    (
        Vec<crate::noncustodial::rpc::NodeCoin>,
        Vec<(String, Option<i64>, String)>,
    ),
    AppError,
> {
    let mut all_coins = Vec::new();
    for addr in addresses {
        match client.get_coins_by_address(addr).await {
            Ok(mut coins) => all_coins.append(&mut coins),
            Err(e) => {
                return Err(match e {
                    AppError::Http(_) => AppError::Rpc(format!(
                        "Can't reach your local node at {node_url}. Start hsd (with --index-address) \
                         to sync and send. Reads still work via the explorer."
                    )),
                    other => AppError::Rpc(format!(
                        "getcoinsbyaddress failed for {addr} (is the node's --index-address enabled?): {other}"
                    )),
                });
            }
        }
    }

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

    Ok((all_coins, txs))
}

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
        let signed = draft
            .signed_tx_hex
            .ok_or_else(|| AppError::InvalidInput("draft is not signed yet".to_string()))?;
        let settings = db::queries::get_settings(&conn)?;
        (signed, settings)
    };

    // Any configured node (local OR remote) can broadcast — configuring a Node
    // RPC URL is the opt-in. The only refusal is a read-only Explorer source,
    // which `send_raw_transaction` rejects internally.
    let client = NodeRpcClient::from_settings(&settings);
    let outcome = classify_broadcast_outcome_with_client(&client, &signed_hex).await;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    match outcome {
        BroadcastOutcome::Success(txid) => {
            db::queries::update_tx_draft_status(
                &conn,
                &draft_id,
                "broadcasted",
                None,
                Some(&txid),
            )?;
            Ok(BroadcastResult {
                draft_id,
                txid,
                status: "broadcasted".to_string(),
            })
        }
        BroadcastOutcome::RpcError(msg) => {
            let msg = msg.to_string();
            db::queries::update_tx_draft_status(&conn, &draft_id, "failed", Some(&msg), None)?;
            db::queries::release_reserved_utxos_for_draft(&conn, &draft_id)?;
            Err(AppError::Rpc(msg))
        }
        BroadcastOutcome::TransportError(e) => {
            let msg = e.to_string();
            db::queries::update_tx_draft_status(
                &conn,
                &draft_id,
                "broadcast_pending",
                Some(&msg),
                None,
            )?;
            Err(e)
        }
    }
}

/// Grace window before a) a broadcast-but-unfound `broadcasted` tx is judged
/// `dropped`, and b) a `broadcast_pending` (transport-ambiguous) draft the
/// node still can't identify is judged `failed` (I5 broadcast_pending
/// auto-resolution). Originally 90s; widened to ~10 minutes because 90s is
/// tight enough for ordinary mempool-relay/node-lag jitter to false-positive
/// as "evicted", especially for the `broadcast_pending` case where the first
/// attempt may not even have reached the node yet. A settings-configurable
/// value was considered (per the task brief) and skipped for now — a single
/// well-named constant is enough, matching the pattern already used for
/// `EXPIRING_SOON_THRESHOLD_DAYS` elsewhere; revisit if node-lag reports come
/// in that need it tuned per-deployment.
const EVICTION_GRACE_SECS: i64 = 600;

/// Confirmation depth beyond which a `confirmed` draft is considered deeply
/// buried and stops being re-polled entirely (I5 core). Below this depth, a
/// reorg that un-mines the tx is still realistic, so the draft is re-verified
/// against the node on every refresh; a reorg past this many blocks is
/// treated as practically impossible, the same "safe" assumption commonly
/// used for finality on Bitcoin-family chains.
const CONFIRMATION_FINALITY_DEPTH: i64 = 12;

/// Extract the txid a draft's `summary_json` already carries. Both plain-send
/// [`crate::noncustodial::types::TxSummary`] and the covenant-action
/// `ActionSummary` (`commands::names`) serialize a `"txid"` field, computed
/// via [`crate::noncustodial::tx::Transaction::txid`] — covenant drafts
/// populate it at BUILD time (`persist_with_conn` stores the plan's txid),
/// plain sends at SIGN time (`build_send_hns_draft` writes `txid: None`; the
/// summary is rebuilt with the txid in `sign_tx_draft`). Either way it is
/// present by the time a draft can be `broadcast_pending` (only signed drafts
/// broadcast), and the Handshake txid hashes only the NON-witness
/// serialization, so it is fixed before signing and identical after, meaning
/// this is exactly the txid a successful broadcast would report. Reading it back here (rather than
/// re-parsing `signed_tx_hex` into a `Transaction` and recomputing) reuses
/// the same computation without reimplementing any hashing/parsing, and
/// is the only way to identify a `broadcast_pending` draft on the node: its
/// DB `txid` column is NULL (the ambiguous `sendrawtransaction` call never
/// returned one — see `broadcast_tx_draft`).
fn local_txid_from_summary(summary_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(summary_json)
        .ok()
        .and_then(|v| {
            v.get("txid")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
}

/// Re-poll the node for the on-chain status of this profile's in-flight drafts.
///
/// Three states are tracked (I5):
///   - `broadcasted` → `confirmed` (mined, ≥1 confirmation, recording the
///     block height) or `dropped` (accepted then evicted, or never confirmed
///     past the grace window — e.g. an auction bid that missed its window).
///   - `confirmed` → re-verified against the node until it reaches
///     [`CONFIRMATION_FINALITY_DEPTH`] confirmations (a cheap SQL-side
///     exit stops polling it after that). If the node no longer knows the tx
///     at its recorded height, a reorg un-mined it: revert to `broadcasted`
///     (clearing the height) so it re-enters ordinary mempool tracking — the
///     eviction-grace logic above then decides if it lands again or is
///     eventually `dropped`.
///   - `broadcast_pending` (a transport-ambiguous broadcast whose outcome was
///     never confirmed — see `broadcast_tx_draft`) → the node is queried for
///     the LOCALLY-computed txid ([`local_txid_from_summary`]); known
///     (mempool or mined) promotes it to `broadcasted`/`confirmed` exactly
///     like a normal broadcast, closing the indefinite reservation hold and
///     the "mined-then-retried" mislabel; definitively unknown (an
///     `AppError::Rpc` "not found", not a transport error) past the grace
///     window since the draft's last update is treated like a failed
///     broadcast: `failed`, reservation released.
///
/// Soft-fails to a no-op when the node is unreachable, so reads stay node-free
/// and no draft is ever touched on a transient blip.
#[tauri::command]
pub async fn refresh_tx_confirmations(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    fn empty_result(profile_id: Option<&str>, node_reachable: bool) -> serde_json::Value {
        serde_json::json!({
            "walletProfileId": profile_id,
            "nodeReachable": node_reachable,
            "checked": 0, "confirmed": 0, "dropped": 0,
            "reverted": 0, "promoted": 0, "failed": 0,
        })
    }

    // 1. Resolve the profile + settings. Drafts are NOT fetched yet — the
    //    confirmed-draft depth filter needs the current chain tip, which is
    //    only known after probing the node in step 2.
    let (profile_id, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let id = match wallet_profile_id {
            Some(id) => id,
            None => db::queries::get_active_profile_id(&conn)?,
        };
        if id.is_empty() {
            return Ok(empty_result(None, false));
        }
        (id, db::queries::get_settings(&conn)?)
    };

    // 2. Probe the node. Unreachable → soft no-op (never touch drafts on a blip).
    let client = NodeRpcClient::from_settings(&settings);
    let tip = match client.get_blockchain_info().await {
        Ok(info) => info.blocks,
        Err(_) => return Ok(empty_result(Some(&profile_id), false)),
    };

    // 3. Snapshot the drafts to poll, now that the tip is known.
    let drafts = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::list_drafts_awaiting_confirmation(
            &conn,
            &profile_id,
            tip,
            CONFIRMATION_FINALITY_DEPTH,
        )?
    };
    if drafts.is_empty() {
        return Ok(empty_result(Some(&profile_id), true));
    }

    // 4. Classify each draft from the node (network I/O, no lock held).
    //    `confirmed_updates` carries an optional txid to persist alongside the
    //    height — needed when a `broadcast_pending` draft (no DB txid yet) is
    //    promoted straight to `confirmed` in one step.
    let mut confirmed_updates: Vec<(String, i64, Option<String>)> = Vec::new();
    let mut maybe_dropped: Vec<String> = Vec::new();
    let mut reverted: Vec<String> = Vec::new();
    let mut promoted_broadcasted: Vec<(String, String)> = Vec::new();
    let mut maybe_failed_pending: Vec<String> = Vec::new();

    for d in &drafts {
        if d.status == "broadcast_pending" {
            let txid = match local_txid_from_summary(&d.summary_json) {
                Some(t) => t,
                None => continue, // can't identify the tx; nothing to poll
            };
            match client.get_raw_transaction(&txid).await {
                Ok(tx) => {
                    let confs = tx
                        .get("confirmations")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if confs >= 1 {
                        let height = tx
                            .get("height")
                            .and_then(|v| v.as_i64())
                            .filter(|h| *h >= 0)
                            .unwrap_or_else(|| (tip - confs + 1).max(0));
                        confirmed_updates.push((d.id.clone(), height, Some(txid)));
                    } else {
                        // Known to the node (mempool), just not mined yet.
                        promoted_broadcasted.push((d.id.clone(), txid));
                    }
                }
                // Definitive "the node has never seen this tx" — see the
                // grace-window handling below.
                Err(AppError::Rpc(_)) => maybe_failed_pending.push(d.id.clone()),
                // Transport error: no definitive answer, leave as-is.
                Err(_) => {}
            }
            continue;
        }

        // `confirmed` and `broadcasted` drafts both always carry a txid (set
        // at the point they first became `broadcasted`).
        let txid = match &d.txid {
            Some(t) => t,
            None => continue,
        };
        match client.get_raw_transaction(txid).await {
            Ok(tx) => {
                let confs = tx
                    .get("confirmations")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if confs >= 1 {
                    let height = tx
                        .get("height")
                        .and_then(|v| v.as_i64())
                        .filter(|h| *h >= 0)
                        .unwrap_or_else(|| (tip - confs + 1).max(0));
                    confirmed_updates.push((d.id.clone(), height, None));
                } else if d.status == "confirmed" {
                    // Was confirmed, now back in the mempool with 0 confs — no
                    // longer confirmed at its recorded height (reorg).
                    reverted.push(d.id.clone());
                }
                // confs == 0 && status == "broadcasted" → still in the
                // mempool as expected; leave it `broadcasted`.
            }
            Err(AppError::Rpc(_)) => {
                if d.status == "confirmed" {
                    // The node no longer knows this tx at all: a reorg
                    // un-mined it.
                    reverted.push(d.id.clone());
                } else {
                    // `broadcasted`, never found → candidate for `dropped`
                    // (grace window applied below).
                    maybe_dropped.push(d.id.clone());
                }
            }
            // Transient transport error mid-loop: skip; the next tick retries.
            Err(_) => {}
        }
    }

    // 5. Persist under the lock.
    let (n_conf, n_drop, n_revert, n_promote, n_fail) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for (id, height, txid) in &confirmed_updates {
            db::queries::update_tx_draft_confirmation(&conn, id, *height, txid.as_deref())?;
        }
        let mut n_drop = 0;
        for id in &maybe_dropped {
            if db::queries::draft_updated_age_secs(&conn, id)? >= EVICTION_GRACE_SECS {
                db::queries::update_tx_draft_status(
                    &conn,
                    id,
                    "dropped",
                    Some(
                        "Broadcast but never confirmed — likely evicted from the mempool \
                         (e.g. an auction bid that missed its window). The coins were not moved.",
                    ),
                    None,
                )?;
                // The broadcast never landed — the coins were never actually
                // spent, so free the reservation now instead of leaving it
                // locked for up to the full TTL.
                db::queries::release_reserved_utxos_for_draft(&conn, id)?;
                n_drop += 1;
            }
        }
        for id in &reverted {
            db::queries::revert_tx_draft_to_broadcasted(
                &conn,
                id,
                "No longer found at its recorded confirmation height — likely a chain \
                 reorg un-mined it. Re-tracking as broadcast; it may be re-mined or \
                 eventually judged dropped.",
            )?;
        }
        for (id, txid) in &promoted_broadcasted {
            // Deliberately do NOT touch the coin reservation here — same
            // reasoning as a fresh successful broadcast (see
            // `broadcast_tx_draft`): the node accepting/holding the tx
            // doesn't mean sync has marked the inputs spent yet, and the
            // reservation is what closes that window.
            db::queries::update_tx_draft_status(&conn, id, "broadcasted", None, Some(txid))?;
        }
        let mut n_fail = 0;
        for id in &maybe_failed_pending {
            if db::queries::draft_updated_age_secs(&conn, id)? >= EVICTION_GRACE_SECS {
                db::queries::update_tx_draft_status(
                    &conn,
                    id,
                    "failed",
                    Some(
                        "Broadcast outcome could not be confirmed by the node within the \
                         grace window — treating as failed. The coins were not moved.",
                    ),
                    None,
                )?;
                // Mirrors the `dropped` path: never definitively landed, so
                // the coins were never actually spent — free the reservation.
                db::queries::release_reserved_utxos_for_draft(&conn, id)?;
                n_fail += 1;
            }
        }
        (
            confirmed_updates.len(),
            n_drop,
            reverted.len(),
            promoted_broadcasted.len(),
            n_fail,
        )
    };

    Ok(serde_json::json!({
        "walletProfileId": profile_id,
        "nodeReachable": true,
        "checked": drafts.len(),
        "confirmed": n_conf,
        "dropped": n_drop,
        "reverted": n_revert,
        "promoted": n_promote,
        "failed": n_fail,
    }))
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
        let slot = state
            .signer
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        slot.as_ref().map(|s| s.is_unlocked()).unwrap_or(false)
    };
    let (source, allow_remote, settings, probe_addr) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = db::queries::get_settings(&conn)?;
        let source = ChainSource::from_settings(&settings);
        let allow_remote =
            settings.get("allow_remote_broadcast").map(|s| s.as_str()) == Some("true");
        // One address to probe the node's address index (if a profile exists).
        let probe_addr = active_profile(&conn)
            .ok()
            .and_then(|p| db::queries::get_profile_addresses(&conn, &p.id).ok())
            .and_then(|addrs| addrs.into_iter().next());
        (source, allow_remote, settings, probe_addr)
    };
    let mut cap =
        crate::providers::WriteCapability::evaluate(signer_unlocked, source, allow_remote);

    // SPV mode is explicitly read-only — override whatever the signer evaluated
    // with a clear SPV-specific message. This must run before the generic
    // "broadcaster_available" check so the SPV-specific reason is shown.
    let node_mode = crate::noncustodial::rpc::resolve_node_mode(&settings);
    if node_mode.is_spv() {
        cap.broadcaster_available = false;
        cap.can_write = false;
        cap.reason = Some(
            "SPV mode cannot send transactions. Switch to Full node mode in Settings → Connections to enable sending."
                .to_string(),
        );
        return Ok(cap);
    }

    // Writes also need the node reachable, fully synced, AND address-indexed (the
    // wallet learns its spendable + name-owner coins via getcoinsbyaddress). If
    // any is missing, downgrade to read-only with a precise, actionable reason.
    if cap.can_write {
        let client = NodeRpcClient::from_settings(&settings);
        let node_url = settings
            .get("node_rpc_url")
            .map(|s| s.as_str())
            .unwrap_or("your node");
        apply_node_write_probe_with_client(&client, &mut cap, node_url, probe_addr.as_deref())
            .await;
    }
    Ok(cap)
}

/// Client-injected node write-capability probe for [`get_write_capability`].
/// Given a capability that is currently `can_write == true`, verifies the node
/// is reachable, fully synced, and address-indexed — downgrading `cap` to
/// read-only with a precise, actionable reason if any check fails. No-op when
/// `cap.can_write` is already false. Testable against a mock.
pub(crate) async fn apply_node_write_probe_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    cap: &mut crate::providers::WriteCapability,
    node_url: &str,
    probe_addr: Option<&str>,
) {
    if !cap.can_write {
        return;
    }
    match client.get_blockchain_info().await {
        Err(_) => {
            cap.broadcaster_available = false;
            cap.can_write = false;
            cap.reason = Some(format!("Start your local node ({node_url}) to send."));
        }
        Ok(info) => {
            // "Synced" means the chain tip is reached (applied blocks caught up
            // to the best known header). When `verification_progress` is
            // available it is the most reliable signal — a node can report
            // height == headers while still only ~8% verified if it is far
            // behind the real chain tip. Always gate on progress when present.
            let synced = match info.verification_progress {
                Some(p) => p >= 0.9999,
                None => match info.headers {
                    Some(h) if h > 0 => info.blocks >= h,
                    _ => true,
                },
            };
            if !synced {
                let pct = match info.verification_progress {
                    Some(p) => (p * 100.0).floor() as i64,
                    None => match info.headers {
                        Some(h) if h > 0 => {
                            ((info.blocks as f64 / h as f64) * 100.0).floor() as i64
                        }
                        _ => 0,
                    },
                };
                cap.can_write = false;
                cap.reason = Some(format!(
                    "Your local node is still syncing ({pct}%). On-chain sends and transfers need a fully-synced node."
                ));
            } else if let Some(addr) = probe_addr {
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

/// Discard a draft that hasn't been broadcast, releasing any coins it had
/// reserved (I3) so a later draft can spend them. Refuses to delete a draft
/// that already reached the chain (`broadcasted`/`confirmed`).
#[tauri::command]
pub async fn delete_tx_draft(state: State<'_, AppState>, draft_id: String) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::delete_tx_draft(&conn, &draft_id)
}

/// Explicitly release a draft's coin reservation (I3) WITHOUT deleting the
/// draft row — e.g. the user wants to abandon a stuck/failed build and let
/// its coins be picked up by a new one, while keeping the old draft around
/// for history. A no-op if the draft holds no reservation. Unlike
/// [`delete_tx_draft`] this never refuses based on status: releasing a
/// reservation is harmless even for a `broadcasted`/`confirmed` draft (by
/// then the coin is normally already excluded from selection via
/// `spent_by_txid`, so this is inert in practice).
#[tauri::command]
pub async fn release_tx_draft_reservation(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<serde_json::Value, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let released = db::queries::release_reserved_utxos_for_draft(&conn, &draft_id)?;
    Ok(serde_json::json!({ "draftId": draft_id, "coinsReleased": released }))
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

#[cfg(test)]
mod confirm_tests {
    use super::*;

    /// Build a `TxDraftRow` with the given action + summary; other fields are
    /// placeholder values the confirmation payload never reads.
    fn draft(action: &str, summary_json: &str) -> db::queries::TxDraftRow {
        db::queries::TxDraftRow {
            id: "d1".to_string(),
            wallet_profile_id: "p1".to_string(),
            action: action.to_string(),
            unsigned_tx_hex: String::new(),
            signed_tx_hex: None,
            signing_inputs_json: "{}".to_string(),
            summary_json: summary_json.to_string(),
            status: "draft".to_string(),
            error_message: None,
            txid: None,
            confirmation_height: None,
            created_at: "now".to_string(),
        }
    }

    fn summary_json(s: &TxSummary) -> String {
        serde_json::to_string(s).unwrap()
    }

    fn base_summary(action: &str) -> TxSummary {
        TxSummary {
            action: action.to_string(),
            send_total_doos: 0,
            fee_doos: 0,
            change_doos: 0,
            input_total_doos: 0,
            num_inputs: 0,
            recipient_address: None,
            txid: None,
            warnings: Vec::new(),
        }
    }

    /// Find the value string for a given label in the rows array.
    fn value_for<'a>(details: &'a serde_json::Value, label: &str) -> Option<&'a str> {
        details["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["label"] == label)
            .and_then(|r| r["value"].as_str())
    }

    #[test]
    fn doos_to_hns_string_formats_whole_and_fractional_amounts() {
        assert_eq!(doos_to_hns_string(0), "0.000000 HNS");
        assert_eq!(doos_to_hns_string(1_000_000), "1.000000 HNS");
        assert_eq!(doos_to_hns_string(1_500_000), "1.500000 HNS");
        assert_eq!(doos_to_hns_string(2_000_123), "2.000123 HNS");
        // Negative shouldn't occur, but must not panic and keeps a sane form.
        assert_eq!(doos_to_hns_string(-1_500_000), "-1.500000 HNS");
    }

    #[test]
    fn confirm_details_for_send_hns_shows_to_amount_fee_txid() {
        let mut s = base_summary("send_hns");
        s.send_total_doos = 2_500_000;
        s.fee_doos = 10_000;
        s.recipient_address = Some("hs1qexampleaddr".to_string());
        s.txid = Some("abc123".to_string());
        let details = confirm_details_for_draft(&draft("send_hns", &summary_json(&s)));

        assert_eq!(value_for(&details, "Action"), Some("Send HNS"));
        assert_eq!(value_for(&details, "To"), Some("hs1qexampleaddr"));
        assert_eq!(value_for(&details, "Amount"), Some("2.500000 HNS"));
        assert_eq!(value_for(&details, "Fee"), Some("0.010000 HNS"));
        assert_eq!(value_for(&details, "Txid"), Some("abc123"));
    }

    #[test]
    fn confirm_details_for_covenant_action_labels_action_and_omits_recipient() {
        let mut s = base_summary("register");
        s.fee_doos = 5_000;
        let details = confirm_details_for_draft(&draft("register", &summary_json(&s)));

        assert_eq!(value_for(&details, "Action"), Some("Name action: register"));
        // Covenant actions don't show a "To" or "Amount" row.
        assert_eq!(value_for(&details, "To"), None);
        assert_eq!(value_for(&details, "Amount"), None);
        assert_eq!(value_for(&details, "Fee"), Some("0.005000 HNS"));
    }

    #[test]
    fn confirm_details_survives_malformed_summary_json() {
        // Forces the `unwrap_or` fallback: still yields an Action + Fee row
        // (fee 0) built from the draft's own `action`, no panic.
        let details = confirm_details_for_draft(&draft("send_hns", "this is not json"));
        assert_eq!(value_for(&details, "Action"), Some("Send HNS"));
        assert_eq!(value_for(&details, "Fee"), Some("0.000000 HNS"));
    }

    #[test]
    fn confirm_details_includes_warnings_as_rows() {
        let mut s = base_summary("send_hns");
        s.warnings = vec!["dust output".to_string(), "high fee".to_string()];
        let details = confirm_details_for_draft(&draft("send_hns", &summary_json(&s)));
        let warnings: Vec<&str> = details["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["label"] == "Warning")
            .map(|r| r["value"].as_str().unwrap())
            .collect();
        assert_eq!(warnings, vec!["dust output", "high fee"]);
    }
}
