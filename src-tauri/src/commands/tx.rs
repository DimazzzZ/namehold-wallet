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
    draft_id: String,
) -> Result<TxDraftSummary, AppError> {
    // 1. Load the draft + session ttl (send_hns also needs spendable coins).
    let (draft, coins, ttl_ms) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let draft = db::queries::get_tx_draft(&conn, &draft_id)?
            .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?;
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
            let reserved = send::load_reserved_coins(&conn, &draft.wallet_profile_id, &draft_id)?;
            if reserved.is_empty() {
                send::load_spendable_coins(&conn, &draft.wallet_profile_id, Some(&draft_id))?
            } else {
                reserved
            }
        } else {
            Vec::new()
        };
        let settings = db::queries::get_settings(&conn)?;
        (draft, coins, session_ttl_ms(&settings))
    };

    // 2. Sign under the signer lock, dispatching by action.
    let (signed_hex, summary_json) = {
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
                &coins,
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
            (built.tx_hex, serde_json::to_string(&summary)?)
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
    match client.send_raw_transaction(&signed_hex).await {
        Ok(txid) => {
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::update_tx_draft_status(
                &conn,
                &draft_id,
                "broadcasted",
                None,
                Some(&txid),
            )?;
            // Deliberately do NOT release this draft's coin reservation here.
            // The node accepting a tx to its mempool does not mean
            // `tracked_utxos.spent_by_txid` gets set for its inputs yet — that
            // only happens the next time `sync_wallet_state` reconciles the
            // chain (see `noncustodial::sync::mark_missing_as_spent`), which
            // can be minutes away. Freeing the reservation now would reopen
            // exactly the double-spend window this feature closes: another
            // draft could re-select the same still-locally-unspent coin
            // before sync catches up. Once sync marks the coin spent,
            // `load_spendable_coins` already excludes it via `spent_by_txid
            // IS NULL` regardless of the (by then harmless, stale)
            // reservation. If the broadcast is later judged `dropped`
            // (evicted / never confirmed), the reservation is released then
            // (see `refresh_tx_confirmations`); otherwise it self-heals via
            // TTL if something goes wrong.
            Ok(BroadcastResult {
                draft_id,
                txid,
                status: "broadcasted".to_string(),
            })
        }
        // `AppError::Rpc(_)` means the node itself answered — with a JSON-RPC
        // error envelope, an unparsable body, or an empty result (see
        // `NodeRpcClient::call`) — so the node definitively did NOT accept
        // this tx (e.g. a double-spend, a stale/evicted input, or a
        // malformed request). The coins were never actually spent: mark the
        // draft `failed` and free the reservation immediately rather than
        // making the user wait out the TTL to retry. This mirrors
        // `refresh_tx_confirmations`, which treats `Err(AppError::Rpc(_))`
        // from `getrawtransaction` as the definitive "not found" signal.
        Err(e @ AppError::Rpc(_)) => {
            let msg = e.to_string();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::update_tx_draft_status(&conn, &draft_id, "failed", Some(&msg), None)?;
            db::queries::release_reserved_utxos_for_draft(&conn, &draft_id)?;
            Err(e)
        }
        // Any other error (HTTP/transport failure: timeout, dropped
        // connection, DNS, TCP reset, ...) means we never got a definitive
        // answer from the node — `sendrawtransaction` may have reached it
        // and been accepted before the connection died. Treating this the
        // same as an outright rejection would free the coin for re-selection
        // by another draft while this one might already be sitting in the
        // node's mempool: exactly the double-select window Finding 1 closes
        // for the TTL sweep, reopened here via the broadcast error path.
        // Instead: keep the reservation, and record the ambiguous outcome as
        // `broadcast_pending` (a status already reserved for this in the
        // schema's CHECK constraint and the frontend's `TxDraftSummary`
        // union, previously unused) rather than `failed` — `failed` would
        // read as "definitely did not happen", which we cannot claim.
        // `refresh_tx_confirmations` DOES resolve this automatically: it also
        // polls `broadcast_pending` drafts, computing the txid locally from
        // `signed_tx_hex` (via `local_txid_from_summary`) since none was
        // returned by the failed broadcast. If the node knows the tx, the
        // draft is promoted to `broadcasted`/`confirmed`; if it definitively
        // doesn't (past the eviction grace), the draft is marked `failed`
        // and the reservation released. A manual retry is also always safe:
        // if the node never got the first attempt, `sendrawtransaction`
        // sends it now; if it already has the tx (mempool or mined), hsd
        // accepts the retry and returns the same txid rather than erroring.
        Err(e) => {
            let msg = e.to_string();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
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
        let source = ChainSource::from_setting(
            settings
                .get("chain_source")
                .map(|s| s.as_str())
                .unwrap_or("local_node"),
        );
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
