//! Read commands for the non-custodial wallet.
//!
//! Balance + names are read from the HNSFans explorer over the active profile's
//! derived addresses / local inventory (node-free), falling back to the
//! node-synced cache when the explorer is unreachable. Transactions come from
//! the local cache. Writes are never routed through here.

use crate::db::queries;
use crate::error::AppError;
use crate::providers::hnsfans::HnsFansClient;
use crate::AppState;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;
use tauri::State;
use tokio::time::sleep;

/// Delay between explorer requests during discovery. HNSFans rate-limits rapid
/// sequential calls with HTTP 403, so we pace them.
const DISCOVERY_THROTTLE: Duration = Duration::from_millis(150);
/// Max tx pages scanned per address (25 txs/page) — bounds the crawl cost for
/// very busy addresses.
const DISCOVERY_MAX_PAGES_PER_ADDRESS: u32 = 8;
const DISCOVERY_PAGE_SIZE: u32 = 25;

/// The active non-custodial profile id, or `None` if none is selected.
fn active_profile(state: &State<'_, AppState>) -> Result<Option<String>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let id = queries::get_active_profile_id(&conn)?;
    if id.is_empty() {
        return Ok(None);
    }
    if queries::get_wallet_profile(&conn, &id)?.is_some() {
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

/// HNSFans explorer client from settings (`explorer_api_url`).
fn explorer_client(settings: &std::collections::HashMap<String, String>) -> HnsFansClient {
    let url = settings
        .get("explorer_api_url")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("https://e.hnsfans.com");
    HnsFansClient::new(url)
}

/// Balance via the explorer (profile addresses), falling back to the cache.
#[tauri::command]
pub async fn read_balance(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let id = match active_profile(&state)? {
        Some(id) => id,
        None => return Ok(serde_json::json!({"confirmed":0,"unconfirmed":0,"locked_confirmed":0,"locked_unconfirmed":0})),
    };
    let (client, addrs) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (explorer_client(&settings), queries::get_profile_addresses(&conn, &id)?)
    };
    if !addrs.is_empty() {
        if let Ok(balance) = client.get_balance(&addrs).await {
            return Ok(serde_json::to_value(&balance)?);
        }
    }
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    queries::read_cached_balance(&conn, &id)
}

/// Names this wallet actually OWNS on-chain — the union of node-free discovered
/// owners ([`discover_owned_names`]) and node-synced owners. Both are pure DB
/// reads (no explorer/node fan-out), so this is instant and never includes the
/// migration *inventory* (`assets`) — those names live in the Portfolio /
/// Migration views, not "Owned Names".
#[tauri::command]
pub async fn read_names(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let id = match active_profile(&state)? {
        Some(id) => id,
        None => return Ok(serde_json::Value::Array(vec![])),
    };
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    // Discovered (node-free) owners; node-synced owners (gated on an unspent
    // tracked UTXO). De-dup by name.
    let discovered = queries::read_owned_names_explorer(&conn, &id)?;
    let cached = queries::read_cached_names(&conn, &id)?;

    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for v in discovered.into_iter().chain(cached) {
        if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
            if seen.insert(n.to_string()) {
                out.push(v);
            }
        }
    }
    Ok(serde_json::Value::Array(out))
}

/// Discover the names this wallet owns, node-free, by crawling the explorer.
///
/// For each derived address: list the txs it touched, fetch each tx's detail
/// (whose outputs carry `action`+`name`+`address`), and collect names whose
/// output pays one of our addresses. Each candidate is then confirmed by
/// checking the name's *current* owner (via history) is still one of our
/// addresses — so names later transferred away are excluded. Confirmed names
/// are persisted (with live state) so `read_names` serves them instantly.
///
/// Throttled + best-effort: on a rate-limit/transport error mid-crawl we stop
/// and persist whatever was confirmed so far rather than failing the whole pass.
#[tauri::command]
pub async fn discover_owned_names(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let id = match active_profile(&state)? {
        Some(id) => id,
        None => return Ok(serde_json::json!({ "discovered": 0, "names": [] })),
    };
    let (client, addrs) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (explorer_client(&settings), queries::get_profile_addresses(&conn, &id)?)
    };
    if addrs.is_empty() {
        return Ok(serde_json::json!({ "discovered": 0, "names": [] }));
    }
    let addr_set: HashSet<&str> = addrs.iter().map(|s| s.as_str()).collect();

    // 1. Crawl: collect candidate names from name-outputs paying our addresses.
    // `partial` flips true if a request errored (e.g. explorer rate-limit), so
    // the UI can say "explorer busy, try again" rather than implying completeness.
    let mut candidates: HashSet<String> = HashSet::new();
    let mut seen_tx: HashSet<String> = HashSet::new();
    let mut partial = false;
    'crawl: for addr in &addrs {
        let mut offset = 0u32;
        let mut pages = 0u32;
        loop {
            let (txids, total) =
                match client.get_address_txids(addr, DISCOVERY_PAGE_SIZE, offset).await {
                    Ok(v) => v,
                    Err(_) => {
                        partial = true; // rate-limited / transport error: skip this address
                        break;
                    }
                };
            for txid in &txids {
                if !seen_tx.insert(txid.clone()) {
                    continue;
                }
                sleep(DISCOVERY_THROTTLE).await;
                match client.get_tx_named_outputs(txid).await {
                    Ok(outs) => {
                        for o in outs {
                            if addr_set.contains(o.address.as_str()) {
                                candidates.insert(o.name);
                            }
                        }
                    }
                    Err(_) => {
                        partial = true; // likely rate-limited: stop, keep candidates
                        break 'crawl;
                    }
                }
            }
            pages += 1;
            offset += DISCOVERY_PAGE_SIZE;
            if txids.is_empty()
                || (offset as u64) >= total
                || pages >= DISCOVERY_MAX_PAGES_PER_ADDRESS
            {
                break;
            }
            sleep(DISCOVERY_THROTTLE).await;
        }
    }

    // 2 + 3. Confirm current ownership and resolve live state.
    let mut owned: Vec<(crate::hsd::types::HsdName, String, u32)> = Vec::new();
    for name in &candidates {
        sleep(DISCOVERY_THROTTLE).await;
        let (owner_txid, owner_vout) = match client.get_name_current_owner(name).await {
            Ok(Some(o)) => o,
            _ => continue,
        };
        sleep(DISCOVERY_THROTTLE).await;
        let owner_outputs = match client.get_tx_named_outputs(&owner_txid).await {
            Ok(o) => o,
            Err(_) => continue,
        };
        let owned_by_us = owner_outputs
            .iter()
            .find(|o| o.index == owner_vout)
            .map(|o| addr_set.contains(o.address.as_str()))
            .unwrap_or(false);
        if !owned_by_us {
            continue;
        }
        sleep(DISCOVERY_THROTTLE).await;
        if let Ok(info) = client.get_name_info(name).await {
            owned.push((info, owner_txid, owner_vout));
        }
    }

    // 4. Persist confirmed owned names.
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for (info, txid, vout) in &owned {
            queries::upsert_owned_name(&conn, &id, info, txid, *vout)?;
        }
    }

    let names: Vec<&String> = owned.iter().map(|(n, _, _)| &n.name).collect();
    Ok(serde_json::json!({ "discovered": owned.len(), "names": names, "partial": partial }))
}

/// Single-name lookup with live auction state. Prefers the node (`getnameinfo`
/// is the authoritative source of phase + countdown data, and works on regtest
/// where there's no explorer), falling back to the HNSFans explorer when no node
/// is reachable. Both paths normalize to the frontend `HsdName` shape.
#[tauri::command]
pub async fn read_name_info(
    state: State<'_, AppState>,
    name: String,
) -> Result<serde_json::Value, AppError> {
    let (explorer, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (explorer_client(&settings), settings)
    };

    // Node first: `getnameinfo` returns `{ info: { name, state, stats:{…phase…} } }`
    // (or null `info` for a name that has never been touched on-chain).
    let node = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);
    if let Ok(raw) = node.get_name_info(&name).await {
        if let Some(info) = raw.get("info").filter(|v| !v.is_null()) {
            if let Some(normalized) = crate::providers::hnsfans::normalize_name(info) {
                return Ok(serde_json::to_value(&normalized)?);
            }
        }
    }

    // Fall back to the explorer (node-free / unreachable).
    let info = explorer.get_name_info(&name).await?;
    Ok(serde_json::to_value(&info)?)
}

/// Transaction history from the local (node-synced) cache.
#[tauri::command]
pub async fn read_transactions(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let id = match active_profile(&state)? {
        Some(id) => id,
        None => return Ok(serde_json::Value::Array(vec![])),
    };
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    Ok(serde_json::Value::Array(queries::read_cached_transactions(&conn, &id)?))
}

/// Reconcile the local inventory (asset TLDs) against the names Namebase still
/// lists for the account.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryComparison {
    pub provider_kind: String,
    pub provider_label: String,
    /// In inventory AND still at Namebase.
    pub matched: Vec<String>,
    /// In inventory but NOT at Namebase (transferred out / no longer custodial).
    pub missing_at_provider: Vec<String>,
    /// At Namebase but NOT in the local inventory (not imported/tracked).
    pub extra_at_provider: Vec<String>,
}

#[tauri::command]
pub async fn compare_inventory_with_provider(
    state: State<'_, AppState>,
) -> Result<InventoryComparison, AppError> {
    // Local inventory (tlds are stored lowercased on import).
    let local_names: Vec<String> = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::list_assets(&conn, None, None, None, None, None)?
            .into_iter()
            .map(|a| a.tld)
            .collect()
    };

    // ONE bulk call to Namebase — what the account still holds. Surface failures
    // (not connected / unreachable) instead of swallowing them into a false
    // "everything is missing".
    let client = crate::commands::namebase::namebase_client(&state)?;
    let domains = client.get_domains().await.map_err(|_| {
        AppError::Other(
            "Couldn't reach Namebase — connect your account in the Namebase tab and try again."
                .to_string(),
        )
    })?;

    let nb_set: HashSet<String> = domains["domains"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d["name"].as_str().map(|s| s.trim().to_lowercase()))
                .collect()
        })
        .unwrap_or_default();
    let inv_set: HashSet<String> = local_names.iter().cloned().collect();

    let mut matched = Vec::new();
    let mut missing_at_provider = Vec::new();
    for n in &local_names {
        if nb_set.contains(n) {
            matched.push(n.clone());
        } else {
            missing_at_provider.push(n.clone());
        }
    }
    let mut extra_at_provider: Vec<String> =
        nb_set.into_iter().filter(|n| !inv_set.contains(n)).collect();

    matched.sort();
    missing_at_provider.sort();
    extra_at_provider.sort();

    Ok(InventoryComparison {
        provider_kind: "namebase".to_string(),
        provider_label: "Namebase".to_string(),
        matched,
        missing_at_provider,
        extra_at_provider,
    })
}
