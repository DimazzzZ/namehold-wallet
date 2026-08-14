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
use rusqlite::OptionalExtension;
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

/// Resolve which profile a read targets. An explicit `wallet_profile_id` (passed
/// by the frontend, keyed to the query's wallet) wins so a read can NEVER return
/// another profile's data — critical when the active profile changes mid-switch.
/// Falls back to the active profile when no id is given or the id doesn't exist.
pub(crate) fn resolve_profile(
    state: &State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<Option<String>, AppError> {
    if let Some(id) = wallet_profile_id {
        let id = id.trim().to_string();
        if !id.is_empty() {
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            if queries::get_wallet_profile(&conn, &id)?.is_some() {
                return Ok(Some(id));
            }
        }
    }
    active_profile(state)
}

/// Check if the local hsd node is connected AND fully synced, making local
/// cached data the preferred read source. Returns `true` when the node RPC
/// answers and the chain is caught up (height ≥ headers, or progress ≥ 0.9999).
///
/// In SPV mode, always returns `false` — SPV nodes don't have `--index-address`
/// and can't serve UTXO queries, so all reads must go through the explorer.
pub(crate) async fn is_node_ready_for_local_reads(state: &State<'_, AppState>) -> bool {
    // SPV mode: node is never authoritative for reads.
    let node_mode = {
        let db = match state.db.lock() {
            Ok(db) => db,
            Err(_) => return false,
        };
        match crate::db::queries::get_settings(&db) {
            Ok(settings) => crate::noncustodial::rpc::resolve_node_mode(&settings),
            Err(_) => crate::noncustodial::rpc::NodeMode::Full,
        }
    };
    if node_mode.is_spv() {
        return false;
    }
    node_tip_height_if_synced(state).await.is_some()
}

/// The live node tip height, but ONLY when the node is connected AND fully
/// synced (same gate as [`is_node_ready_for_local_reads`] — this is its
/// height-carrying form). `None` when the node is unreachable or catching up.
pub(crate) async fn node_tip_height_if_synced(state: &State<'_, AppState>) -> Option<i64> {
    let settings = {
        let db = state.db.lock().ok()?;
        crate::db::queries::get_settings(&db).ok()?
    };
    node_tip_height_if_synced_from_settings(&settings).await
}

/// Settings-based form of [`node_tip_height_if_synced`], usable outside a
/// `State<AppState>` context (e.g. the background sync thread, which holds only
/// a bare DB connection). Returns the node tip height iff the node RPC answers
/// AND the chain is fully synced. This is the single source of truth for the
/// "is the node authoritative?" gate — the `State`-based helper delegates here.
pub(crate) async fn node_tip_height_if_synced_from_settings(
    settings: &std::collections::HashMap<String, String>,
) -> Option<i64> {
    let client = crate::noncustodial::rpc::NodeRpcClient::from_settings(settings);
    let info = client.get_blockchain_info().await.ok()?;
    // Connected — now check if synced.
    // When verification_progress is available it is the most reliable signal —
    // a node can report height == headers while still only ~8% verified if it
    // is far behind the real chain tip. Always gate on progress when present.
    let synced = if let Some(progress) = info.verification_progress {
        progress >= 0.9999
    } else if let Some(headers) = info.headers {
        headers > 0 && info.blocks >= headers
    } else {
        // No sync metadata: assume synced (e.g. regtest with a single miner).
        true
    };
    synced.then_some(info.blocks)
}

/// Settings-based readiness gate: `true` when the local node is connected AND
/// fully synced, making node/local data the authoritative read source. Mirrors
/// [`is_node_ready_for_local_reads`] for callers that only have settings/a DB
/// connection (the background sync thread).
pub async fn node_ready_from_settings(
    settings: &std::collections::HashMap<String, String>,
) -> bool {
    node_tip_height_if_synced_from_settings(settings)
        .await
        .is_some()
}

/// HNSFans explorer client from settings (`explorer_api_url`). Thin wrapper
/// kept for call-site brevity — the actual construction is centralized in
/// [`crate::providers::explorer_client_from_settings`] (Task 11 / S1).
fn explorer_client(settings: &std::collections::HashMap<String, String>) -> HnsFansClient {
    crate::providers::explorer_client_from_settings(settings)
}

/// Node-only owned-name discovery for [`discover_owned_names`]. Resolves the
/// wallet's name-covenant coins to names via the node (`getnamebyhash` with a
/// rawName fallback), fetches each name's `getnameinfo`, and upserts an
/// authoritative `tracked_name_states` row — no explorer calls. Returns the
/// same `{ discovered, names }` shape as the explorer path.
async fn discover_owned_names_via_node(
    state: &State<'_, AppState>,
    profile_id: &str,
) -> Result<serde_json::Value, AppError> {
    let (settings, hashes) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        let hashes = queries::list_unspent_wallet_name_hashes(&conn, profile_id)?;
        (settings, hashes)
    };

    let client = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);

    // Resolve each hash → name (node's getnamebyhash, else the coin's rawName).
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for h in &hashes {
        let resolved = match client.get_name_by_hash(&h.name_hash_hex).await {
            Ok(Some(n)) => Some(n),
            Ok(None) | Err(_) => h
                .raw_name_hex
                .as_deref()
                .and_then(|hex| hex::decode(hex).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok()),
        };
        if let Some(name) = resolved {
            let trimmed = name.trim().to_ascii_lowercase();
            if !trimmed.is_empty() {
                names.insert(trimmed);
            }
        }
    }

    let mut fetched: Vec<(String, serde_json::Value)> = Vec::new();
    for name in &names {
        if let Ok(info) = client.get_name_info(name).await {
            fetched.push((name.clone(), info));
        }
    }

    let discovered_names: Vec<String> = fetched.iter().map(|(n, _)| n.clone()).collect();
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for (name, info) in &fetched {
            crate::noncustodial::sync::upsert_name_state(&conn, profile_id, name, info)?;
        }
    }

    Ok(serde_json::json!({
        "discovered": discovered_names.len(),
        "names": discovered_names,
    }))
}

/// Node-only owned-name reconciliation for [`repair_owned_names`]. For each
/// candidate name, ask the node for authoritative state via `getnameinfo` and
/// resolve the owner outpoint's address via `gettxout` — the same information
/// the explorer path derives from `/history`, without HNSFans.
async fn repair_owned_names_via_node(
    state: &State<'_, AppState>,
    profile_id: &str,
) -> Result<serde_json::Value, AppError> {
    let (settings, inventory_tlds, tracked, all_addresses) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        (
            queries::get_settings(&conn)?,
            queries::get_inventory_tlds(&conn)?,
            queries::list_tracked_name_names(&conn, profile_id)?,
            queries::get_profile_addresses(&conn, profile_id)?,
        )
    };
    let addr_set: HashSet<String> = all_addresses.iter().cloned().collect();

    let mut candidates: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for name in inventory_tlds.iter().chain(tracked.iter()) {
        let n = name.trim().to_lowercase();
        if n.is_empty() || !seen.insert(n.clone()) {
            continue;
        }
        candidates.push(n);
    }

    let client = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);

    let mut errors: Vec<String> = Vec::new();
    let mut repaired = 0u32;

    for name in &candidates {
        // Authoritative name state from the node.
        let info_result = match client.get_name_info(name).await {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{name}: getnameinfo failed — {e}"));
                continue;
            }
        };
        let info_opt = info_result.get("info").cloned().filter(|v| !v.is_null());

        // Resolve the current owner outpoint's address:
        //   info.owner.{hash,index} IS the current owner UTXO (or null when the
        //   name has no live owner — e.g. never OPENed, or fully released). We
        //   ask the node for that specific output via `gettxout` and read its
        //   address; if it belongs to one of our derived addresses, we own it.
        let owner = info_opt.as_ref().and_then(|i| i.get("owner"));
        let owner_txid = owner
            .and_then(|o| o.get("hash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let owner_vout = owner.and_then(|o| o.get("index")).and_then(|v| v.as_u64());

        let (owner_addr, owner_txid_str, owner_vout_u32) = match (owner_txid, owner_vout) {
            (Some(t), Some(v)) if !t.is_empty() && t != "0".repeat(t.len()) => {
                match client.get_tx_out(&t, v as u32).await {
                    Ok(Some(txo)) => {
                        let addr = txo
                            .get("address")
                            .and_then(|a| a.get("string").or_else(|| a.get("hash")))
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        (addr, Some(t), Some(v as u32))
                    }
                    _ => (None, Some(t), Some(v as u32)),
                }
            }
            _ => (None, None, None),
        };

        let owned_by_wallet = owner_addr
            .as_deref()
            .map(|a| addr_set.contains(a))
            .unwrap_or(false);

        match (
            owned_by_wallet,
            info_opt,
            owner_txid_str,
            owner_vout_u32,
            owner_addr,
        ) {
            (true, Some(info), Some(txid), Some(vout), Some(addr)) => {
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                // Reuse the existing upsert — it accepts a `getnameinfo.info`
                // payload (the same shape the explorer's `get_name_info_optional`
                // returns) plus (owner_txid, owner_vout, owner_address).
                let info_shaped: crate::hsd::types::HsdName = serde_json::from_value(info)
                    .map_err(|e| {
                        AppError::Rpc(format!("malformed node getnameinfo for {name}: {e}"))
                    })?;
                queries::upsert_owned_name(&conn, profile_id, &info_shaped, &txid, vout, &addr)?;
                queries::mark_asset_finalized_owned(&conn, name, info_shaped.state.as_deref())?;
                repaired += 1;
            }
            _ => {
                // Not owned per node (or unresolvable owner): stamp so repeated
                // runs converge instead of re-checking forever.
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                queries::touch_asset_synced(&conn, name)?;
            }
        }
    }

    Ok(serde_json::json!({
        "repaired": repaired,
        "errors": errors,
        "candidates": candidates.len(),
    }))
}

/// Balance read with automatic source selection:
///   1. If the local hsd node is connected AND fully synced → use the local
///      chain cache (authoritative, no network hop).
///   2. Otherwise → fall back to the HNSFans explorer over the profile's
///      watch addresses.
///   3. If the explorer also fails → fall back to the local cache (last resort).
///
/// `wallet_profile_id` pins the read to a specific wallet (defaults to active).
#[tauri::command]
pub async fn read_balance(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => {
            return Ok(
                serde_json::json!({"confirmed":0,"unconfirmed":0,"locked_confirmed":0,"locked_unconfirmed":0}),
            )
        }
    };

    // Prefer local cache when the node is connected and synced.
    if is_node_ready_for_local_reads(&state).await {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        return queries::read_cached_balance(&conn, &id);
    }

    // Explorer fallback.
    let (client, mut addrs) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (
            explorer_client(&settings),
            queries::get_profile_addresses(&conn, &id)?,
        )
    };
    // Auto-provision derived addresses if none exist yet, so the explorer
    // can look up the wallet's balance even if sync hasn't run.
    if addrs.is_empty() {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        if let Ok(Some(profile)) = queries::get_wallet_profile(&conn, &id) {
            if let Ok(network) =
                crate::noncustodial::derivation::network_from_profile(&profile.network)
            {
                if let Ok(xpub) = crate::noncustodial::hd::ExtendedPubKey::from_xpub(
                    network,
                    &profile.account_xpub,
                ) {
                    if let Ok(recv) = crate::noncustodial::derivation::ensure_addresses(
                        &conn,
                        &id,
                        0,
                        network,
                        &xpub,
                        crate::noncustodial::derivation::BRANCH_RECEIVE,
                        20,
                    ) {
                        let _ = crate::noncustodial::derivation::ensure_addresses(
                            &conn,
                            &id,
                            0,
                            network,
                            &xpub,
                            crate::noncustodial::derivation::BRANCH_CHANGE,
                            20,
                        );
                        addrs = recv.into_iter().map(|d| d.address).collect();
                    }
                }
            }
        }
    }
    if !addrs.is_empty() {
        if let Ok(balance) = client.get_balance(&addrs).await {
            // `HsdBalance` deserializes from the hsd node's camelCase RPC, so its
            // Serialize impl also emits camelCase (`lockedConfirmed`). The frontend
            // contract for `read_balance` is snake_case (see the two json! paths
            // above, src/types HsdBalance, and src/lib/zod.ts), so map explicitly
            // here — returning the struct verbatim would silently drop the locked
            // fields on the FE. Covered by read_balance_serializes_snake_case.
            return Ok(serde_json::json!({
                "confirmed": balance.confirmed,
                "unconfirmed": balance.unconfirmed,
                "locked_confirmed": balance.locked_confirmed.unwrap_or(0),
                "locked_unconfirmed": balance.locked_unconfirmed.unwrap_or(0),
            }));
        }
    }
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    queries::read_cached_balance(&conn, &id)
}

/// Names this wallet actually OWNS on-chain — the union of node-free discovered
/// owners ([`discover_owned_names`]) and node-synced owners. Both are pure DB
/// reads, so this is instant and never includes the migration *inventory*
/// (`assets`) — those names live in the Portfolio / Migration views, not
/// "Owned Names".
///
/// This command is strictly DB-only: it never constructs an explorer client
/// or makes network calls. Ownership reconciliation (including surfacing
/// transferred/migrated inventory names) happens exclusively through the Sync
/// flow (`repair_owned_names` / `discover_owned_names`), which writes results
/// into `tracked_name_states` — `read_names` just reads that table back.
///
/// When the local node is connected and synced, the node-synced cache is the
/// primary source (authoritative); otherwise discovered (explorer-crawled,
/// but previously persisted) names are primary. Either way both sources come
/// from the DB.
#[tauri::command]
pub async fn read_names(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(serde_json::Value::Array(vec![])),
    };
    // Check node readiness BEFORE acquiring the DB lock so we don't hold
    // MutexGuard across the async RPC probe (MutexGuard is !Send).
    let local_ready = is_node_ready_for_local_reads(&state).await;

    let out = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        collect_read_names_data(&conn, &id, local_ready)?
    };

    Ok(serde_json::Value::Array(out))
}

/// Synchronous helper that collects all DB data for `read_names`.
/// The MutexGuard is dropped when this returns, so no !Send value crosses
/// an async boundary.
fn collect_read_names_data(
    conn: &rusqlite::Connection,
    id: &str,
    local_ready: bool,
) -> Result<Vec<serde_json::Value>, AppError> {
    let discovered = queries::read_owned_names_explorer(conn, id)?;
    let cached = queries::read_cached_names(conn, id)?;

    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let (primary, secondary) = if local_ready {
        (cached, discovered)
    } else {
        (discovered, cached)
    };
    for v in primary.into_iter().chain(secondary) {
        if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
            if seen.insert(n.to_string()) {
                out.push(v);
            }
        }
    }

    Ok(out)
}

/// Names this wallet currently holds an *auction position* in — opened, bid,
/// or revealed, but not yet owned (e.g. an in-progress `.namehold` open that
/// hasn't won its auction). Complements [`read_names`], which only returns
/// names already OWNED; this is what lets the Auctions view surface a name
/// the wallet has a stake in before ownership lands.
///
/// Strictly DB-only, like `read_names` — no RPC/network calls. The frontend
/// layers live auction phase / capabilities on top via the existing
/// capability batch.
#[tauri::command]
pub async fn read_auction_position_names(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(serde_json::Value::Array(vec![])),
    };

    let names = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::auction_position_names(&conn, &id)?
    };

    Ok(serde_json::Value::Array(
        names.into_iter().map(serde_json::Value::String).collect(),
    ))
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

    // Node-only path: when the local node is authoritative (connected + fully
    // synced) we ignore the HNSFans explorer entirely. Owned names are derived
    // from the wallet's own name-covenant coins (already refreshed into
    // `tracked_utxos` by `sync_node_step`) — the coin's covenant items[0] gives
    // us the nameHash, resolved to a name via `getnamebyhash` (or the paired
    // OPEN/BID/FINALIZE covenant's rawName). This is the "post-sync workaround
    // retired" path described in the Feature 3 plan.
    if is_node_ready_for_local_reads(&state).await {
        return discover_owned_names_via_node(&state, &id).await;
    }

    let (client, addrs) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (
            explorer_client(&settings),
            queries::get_profile_addresses(&conn, &id)?,
        )
    };
    if addrs.is_empty() {
        return Ok(serde_json::json!({ "discovered": 0, "names": [] }));
    }
    let addr_set: HashSet<String> = addrs.iter().cloned().collect();

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
            let (txids, total) = match client
                .get_address_txids(addr, DISCOVERY_PAGE_SIZE, offset)
                .await
            {
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

    // 2 + 3. Confirm current ownership (via explorer history, NOT the dead
    // `owner.hash` field) and resolve live state.
    let mut owned: Vec<(crate::hsd::types::HsdName, String, u32, String)> = Vec::new();
    for name in &candidates {
        sleep(DISCOVERY_THROTTLE).await;
        let resolution = match crate::commands::sync::resolve_owner_via_history(
            &client, name, &addr_set,
        )
        .await
        {
            Ok(Some(r)) => r,
            _ => continue,
        };
        if !resolution.owned_by_wallet {
            continue;
        }
        sleep(DISCOVERY_THROTTLE).await;
        if let Ok(info) = client.get_name_info(name).await {
            owned.push((
                info,
                resolution.owner_txid,
                resolution.owner_vout,
                resolution.owner_address,
            ));
        }
    }

    // 4. Persist confirmed owned names.
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        for (info, txid, vout, owner_address) in &owned {
            queries::upsert_owned_name(&conn, &id, info, txid, *vout, owner_address)?;
        }
    }

    let names: Vec<&String> = owned.iter().map(|(n, _, _, _)| &n.name).collect();
    Ok(serde_json::json!({ "discovered": owned.len(), "names": names, "partial": partial }))
}

/// Single-name lookup with live auction state. Prefers the node (`getnameinfo`
/// is the authoritative source of phase + countdown data, and works on regtest
/// where there's no explorer), falling back to the HNSFans explorer when no node
/// is reachable. Both paths normalize to the frontend `HsdName` shape.
///
/// When the node confirms a name exists in the Handshake namespace but has never
/// been opened on-chain (`info` is null), we synthesize an `AVAILABLE` response
/// so the frontend can offer the "Open auction" action without treating the name
/// as an error.
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

    // Node first — but ONLY when it is fully synced. An unsynced node answers
    // `getnameinfo` with a stale/partial `state`, which would feed a wrong
    // `badge.phase` to the modal. Gate on the same sync check as the balance/name
    // reads so an unsynced node falls through to the explorer path below.
    // `getnameinfo` returns `{ info: { name, state, stats:{…phase…} } }`
    // (or null `info` for a name that has never been touched on-chain).
    let node = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);
    if is_node_ready_for_local_reads(&state).await {
        if let Ok(raw) = node.get_name_info(&name).await {
            // The node knows the name — `info` is present and carries auction state.
            if let Some(info) = raw.get("info").filter(|v| !v.is_null()) {
                if let Some(normalized) = crate::providers::hnsfans::normalize_name(info) {
                    return Ok(serde_json::to_value(&normalized)?);
                }
            }
            // `info` is null → the name exists in the HNS namespace but has never
            // been opened. Synthesize an AVAILABLE entry so the frontend can offer
            // the "Open auction" action cleanly.
            return Ok(serde_json::to_value(&crate::hsd::types::HsdName {
                name: name.clone(),
                name_hash: None,
                state: Some("AVAILABLE".to_string()),
                height: None,
                renewal: None,
                owner: None,
                value: None,
                highest: None,
                registered: Some(false),
                expired: None,
                stats: None,
                transfer: None,
                revoked: None,
                bids: None,
            })?);
        }
    }

    // Node unreachable or not synced — fall back to the explorer.
    //
    // `get_name_info_optional` returns `Ok(None)` when the explorer reports the
    // name is not found (HTTP 404 / empty body), which we treat as AVAILABLE so
    // the user can start an auction.  Real transport failures (DNS, timeout,
    // 5xx) still propagate as `Err` instead of silently degrading to
    // AVAILABLE.
    match explorer.get_name_info_optional(&name).await {
        Ok(Some(info)) => Ok(serde_json::to_value(&info)?),
        Ok(None) => {
            // Explorer confirms the name is unknown — synthesize AVAILABLE.
            Ok(serde_json::to_value(&crate::hsd::types::HsdName {
                name: name.clone(),
                name_hash: None,
                state: Some("AVAILABLE".to_string()),
                height: None,
                renewal: None,
                owner: None,
                value: None,
                highest: None,
                registered: Some(false),
                expired: None,
                stats: None,
                transfer: None,
                revoked: None,
                bids: None,
            })?)
        }
        Err(e) => Err(e),
    }
}

/// Empty `read_name_bids` response — used whenever there is nothing to show
/// (no resolved profile, or the explorer has no data for the name). Never an
/// error: the auction bids panel must degrade gracefully, not break.
pub(crate) fn empty_name_bids_response(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "state": null,
        "highest": null,
        "value": null,
        "bids": [],
        "myBidCount": 0
    })
}

/// Pure join: shape the explorer's per-bid detail for `name` into the
/// frontend contract, marking which bids are the caller's own from LOCAL
/// `bid_commitments` only.
///
/// Honesty (Vickrey): this NEVER fabricates another bidder's value — `value`
/// on each returned bid is exactly what the explorer reported (which is
/// nothing, pre-REVEAL). `myValue` comes only from plaintext we already hold
/// locally (`bid_value_doos` on a matching commitment), never inferred.
///
/// `commitments` MUST already be scoped to the resolved profile by the
/// caller (per-wallet read isolation) — this function additionally matches
/// only commitments for `name`, so a stray commitment for a different name
/// (even within the same profile) can never mark a bid as "mine". A bid
/// without a `txid` (not yet indexed) can never match and is reported with
/// `mine:false, myValue:null`, but still counts in the returned `bids` array.
pub(crate) fn merge_name_bids(
    info: &crate::hsd::types::HsdName,
    commitments: &[queries::BidCommitmentRow],
    name: &str,
) -> serde_json::Value {
    use std::collections::HashMap;

    let mine_by_txid: HashMap<&str, i64> = commitments
        .iter()
        .filter(|c| c.name == name)
        .filter_map(|c| c.bid_txid.as_deref().map(|txid| (txid, c.bid_value_doos)))
        .collect();

    let mut my_bid_count: u32 = 0;
    let bids: Vec<serde_json::Value> = info
        .bids
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|bid| {
            let my_value = bid
                .txid
                .as_deref()
                .and_then(|txid| mine_by_txid.get(txid).copied());
            let is_mine = my_value.is_some();
            if is_mine {
                my_bid_count += 1;
            }
            serde_json::json!({
                "txid": bid.txid,
                "index": bid.index,
                "lockup": bid.lockup,
                "value": bid.value,
                "revealed": bid.revealed,
                "win": bid.win,
                "reveal": bid.reveal,
                "time": bid.time,
                "mine": is_mine,
                "myValue": my_value,
            })
        })
        .collect();

    serde_json::json!({
        "name": name,
        "state": info.state,
        "highest": info.highest,
        "value": info.value,
        "bids": bids,
        "myBidCount": my_bid_count,
    })
}

/// Shape chain-scanner-indexed bids (`name_bid_outpoints`) into the frontend
/// contract, marking the caller's own bids from LOCAL `bid_commitments`.
///
/// Same honesty guarantees as [`merge_name_bids`]: `value` is only what the
/// scanner observed on-chain (the REVEAL output value, `None` pre-reveal); a
/// bid is "mine" only when its txid matches a local commitment, and `myValue`
/// is the plaintext bid value we already hold — never inferred from a
/// competitor's coin. `commitments` MUST be scoped to the resolved profile by
/// the caller.
pub(crate) fn merge_indexed_bids(
    indexed: &[crate::hsd::types::HsdBid],
    commitments: &[queries::BidCommitmentRow],
    name: &str,
) -> serde_json::Value {
    use std::collections::HashMap;

    let mine_by_txid: HashMap<&str, i64> = commitments
        .iter()
        .filter(|c| c.name == name)
        .filter_map(|c| c.bid_txid.as_deref().map(|txid| (txid, c.bid_value_doos)))
        .collect();

    // The highest revealed value we observed on-chain, for the aggregate.
    let highest = indexed.iter().filter_map(|b| b.value).max();

    let mut my_bid_count: u32 = 0;
    let bids: Vec<serde_json::Value> = indexed
        .iter()
        .map(|bid| {
            let my_value = bid
                .txid
                .as_deref()
                .and_then(|txid| mine_by_txid.get(txid).copied());
            let is_mine = my_value.is_some();
            if is_mine {
                my_bid_count += 1;
            }
            serde_json::json!({
                "txid": bid.txid,
                "index": bid.index,
                "lockup": bid.lockup,
                "value": bid.value,
                "revealed": bid.revealed,
                "win": bid.win,
                "reveal": bid.reveal,
                "time": bid.time,
                "mine": is_mine,
                "myValue": my_value,
            })
        })
        .collect();

    serde_json::json!({
        "name": name,
        "state": null,
        "highest": highest,
        "value": highest,
        "bids": bids,
        "myBidCount": my_bid_count,
    })
}

/// Per-bid detail for a name from the HNSFans explorer, with the caller's own
/// bids marked via LOCAL `bid_commitments` (plaintext bid/lockup values this
/// wallet already holds — never fabricated from the explorer, which cannot
/// reveal a competitor's true value before REVEAL). Read-only; feeds the
/// auction bids panel inside the name actions modal.
///
/// Node `getnameinfo` has no per-bid array (aggregates only — see
/// `HsdName.bids` doc), so this always goes through the explorer. Degrades to
/// an empty response (never an error/panic) when no profile resolves or the
/// explorer has nothing for the name; a real transport failure still
/// propagates as `Err`. `wallet_profile_id` pins the read to a specific
/// wallet (defaults to active) — critical so a bid from another wallet is
/// never marked "mine".
#[tauri::command]
pub async fn read_name_bids(
    state: State<'_, AppState>,
    name: String,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(empty_name_bids_response(&name)),
    };

    // Node-only path: when the node is authoritative AND the chain scanner has
    // indexed past the name's auction height, serve bids from the local
    // `name_bid_outpoints` table — no HNSFans call. Fall through to the
    // explorer when the scanner hasn't caught up yet.
    if is_node_ready_for_local_reads(&state).await {
        let name_hash_hex = hex::encode(crate::noncustodial::names::hash_name(&name)?);
        let (indexed_bids, commitments, scanner_height, name_height) = {
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            let indexed = crate::commands::chain_scan::read_indexed_bids(&conn, &name_hash_hex)?;
            let comms = queries::list_bid_commitments(&conn, &id)?;
            let cursor_h = crate::commands::chain_scan::scan_cursor_height(&conn);
            // The name's on-chain height (auction OPEN height) — if we have a
            // tracked_name_states row, use its `height`; otherwise fall through.
            let nh: Option<i64> = conn
                .query_row(
                    "SELECT height FROM tracked_name_states WHERE wallet_profile_id = ?1 AND name = ?2",
                    rusqlite::params![id, name],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            (indexed, comms, cursor_h, nh)
        };

        // Only serve from the index if the scanner has reached the name's
        // auction height (so we're confident we've seen all BIDs). If the
        // name_height is unknown (name never opened?), or the scanner hasn't
        // caught up, fall through to the explorer.
        let scanner_covers = name_height.map(|nh| scanner_height >= nh).unwrap_or(false);

        if scanner_covers {
            return Ok(merge_indexed_bids(&indexed_bids, &commitments, &name));
        }
    }

    // Explorer fallback (pre-sync or scanner hasn't reached the name yet).
    let (client, commitments) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (
            explorer_client(&settings),
            queries::list_bid_commitments(&conn, &id)?,
        )
    };

    let info = match client.get_name_info_optional(&name).await? {
        Some(info) => info,
        None => return Ok(empty_name_bids_response(&name)),
    };

    Ok(merge_name_bids(&info, &commitments, &name))
}

/// Extract the `records` array from a `getnameresource` payload. Handshake's
/// `getnameresource` returns `{records:[...]}` for a name with a resource, or
/// `null` / a shape without `records` when the name has none. Never panics —
/// anything that isn't `resource.records: []` degrades to an empty vec so the
/// caller can treat the result uniformly.
pub(crate) fn records_from_resource(resource: &serde_json::Value) -> Vec<serde_json::Value> {
    resource
        .get("records")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Combined name info + DNS resource for the DnsRecords page. Returns:
/// `{ name, state, height, renewal, stats: { daysUntilExpire, blocksUntilExpire },
///    data: { records: [...] } }`
///
/// Fetches name info (from explorer or node) and resource records (from node
/// only — explorer doesn't serve resources). Degrades gracefully: if node is
/// unavailable, `data.records` will be empty but name info still populates.
#[tauri::command]
pub async fn get_resource(
    state: State<'_, AppState>,
    name: String,
) -> Result<serde_json::Value, AppError> {
    // 1. Fetch name info (state, height, stats) — reuses read_name_info logic.
    let (explorer, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let s = queries::get_settings(&conn)?;
        (explorer_client(&s), s)
    };
    let node = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);
    let node_ready = is_node_ready_for_local_reads(&state).await;

    // Name info: try node first, then explorer.
    let info: serde_json::Value = if node_ready {
        if let Ok(raw) = node.get_name_info(&name).await {
            if let Some(info) = raw.get("info").filter(|v| !v.is_null()) {
                crate::providers::hnsfans::normalize_name(info)
                    .map(|n| serde_json::to_value(&n).unwrap_or_default())
                    .unwrap_or_default()
            } else {
                serde_json::json!({ "name": name, "state": "AVAILABLE" })
            }
        } else {
            serde_json::json!({})
        }
    } else {
        match explorer.get_name_info_optional(&name).await {
            Ok(Some(i)) => serde_json::to_value(&i).unwrap_or_default(),
            Ok(None) => serde_json::json!({ "name": name, "state": "AVAILABLE" }),
            Err(_) => serde_json::json!({}),
        }
    };

    // 2. Fetch resource records (node only).
    let records: Vec<serde_json::Value> = if node_ready {
        match node.get_name_resource(&name).await {
            Ok(res) if !res.is_null() => records_from_resource(&res),
            _ => vec![],
        }
    } else {
        vec![]
    };

    // 3. Assemble the shape the frontend expects.
    let state_str = info.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let height = info.get("height").and_then(|v| v.as_u64());
    let renewal = info.get("renewal").and_then(|v| v.as_u64());
    let stats = info.get("stats").cloned();

    Ok(serde_json::json!({
        "name": name,
        "state": state_str,
        "height": height,
        "renewal": renewal,
        "stats": stats,
        "data": {
            "records": records,
        }
    }))
}

/// Current DNS *resource* for a name, read from the local hsd node
///
/// Returns the FULL resource object (`{ records: [...], ttl?, serial?, ... }`)
/// so callers that only care about DNS rows (the editor) AND callers that want
/// resource-level metadata (the name-info modal — TTL, serial, and unusual
/// record types) share one command. Degrades to an object with an empty
/// `records` array (never an error) when no profile is resolved, the node
/// isn't ready, the RPC fails, or the resource is `null` — the frontend seeds
/// its editor from `resource.records` and an empty seed is the honest signal
/// that we can't show current records.
///
/// `wallet_profile_id` pins the read to a specific wallet context so a fast
/// profile switch never returns another wallet's view; the resolved id itself
/// isn't used further (records are name-scoped, not per-wallet).
#[tauri::command]
pub async fn read_name_records(
    state: State<'_, AppState>,
    name: String,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    // Uniform empty-resource shape returned on every degrade path so consumers
    // can always do `resource.records` without a null check.
    let empty_resource = || {
        serde_json::json!({
            "records": [],
        })
    };

    if resolve_profile(&state, wallet_profile_id)?.is_none() {
        return Ok(empty_resource());
    }
    if is_node_ready_for_local_reads(&state).await {
        // Read settings under a short lock, then drop the guard BEFORE the
        // async RPC call — the same pattern the other node-first reads use so
        // we never hold the DB mutex across an .await.
        let settings = {
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            queries::get_settings(&conn)?
        };
        let node = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);
        if let Ok(res) = node.get_name_resource(&name).await {
            // hsd's `getnameresource` returns `null` (not an object) when a
            // name has no resource. Normalize to the uniform empty shape so
            // frontends can always index `.records` safely.
            if res.is_null() {
                return Ok(empty_resource());
            }
            // Guarantee `records` is always present as an array, even if the
            // node ever returns a resource without it — belt-and-braces so
            // the frontend contract holds regardless of node version quirks.
            if let serde_json::Value::Object(mut map) = res {
                if !map.get("records").map(|v| v.is_array()).unwrap_or(false) {
                    map.insert("records".to_string(), serde_json::Value::Array(vec![]));
                }
                return Ok(serde_json::Value::Object(map));
            }
            // Non-object, non-null (shouldn't happen with real hsd) —
            // degrade rather than surface a surprise shape.
            return Ok(empty_resource());
        }
    }
    Ok(empty_resource())
}

/// Compact block details for the in-app Block Info modal, read from the local
/// hsd node (`getblockhash` → `getblock`). Node-only: the explorer path does
/// not expose block internals, so this soft-degrades to `null` whenever no
/// synced node is reachable (the frontend renders a "requires synced node"
/// state rather than surfacing an error). Mirrors the graceful-degrade
/// contract of [`read_name_records`].
///
/// Returns `{ height, hash, time, txCount, minerReward, difficulty }` where
/// `minerReward` is the sum of the coinbase (first tx) output values in doos.
#[tauri::command]
pub async fn read_block_info(
    state: State<'_, AppState>,
    height: i64,
) -> Result<serde_json::Value, AppError> {
    // Node-only: without a synced local node there's nothing to read. Soft-
    // degrade to null so the modal can show a "requires synced node" hint.
    if height < 0 || !is_node_ready_for_local_reads(&state).await {
        return Ok(serde_json::Value::Null);
    }

    // Read settings under a short lock, then drop the guard BEFORE the async
    // RPC calls — never hold the DB mutex across an .await.
    let settings = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::get_settings(&conn)?
    };
    let node = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);

    // getblockhash(height) → getblock(hash, verbose, verboseTx). Any RPC
    // failure (node fell over mid-read, height beyond tip) soft-degrades.
    let hash = match node.get_block_hash(height).await {
        Ok(h) => h,
        Err(_) => return Ok(serde_json::Value::Null),
    };
    let block = match node.get_block(&hash).await {
        Ok(b) => b,
        Err(_) => return Ok(serde_json::Value::Null),
    };

    // hsd verbose block: { hash, height, time, difficulty, tx: [ { outputs:
    // [ { value } ] }, ... ] }. The coinbase is tx[0]; its outputs sum to the
    // miner reward (subsidy + fees). Guard every access — a missing/short
    // array yields a 0 reward rather than a panic.
    let txs = block.get("tx").and_then(|v| v.as_array());
    let tx_count = txs.map(|a| a.len()).unwrap_or(0);
    let miner_reward: i64 = txs
        .and_then(|a| a.first())
        .and_then(|coinbase| coinbase.get("outputs"))
        .and_then(|o| o.as_array())
        .map(|outs| {
            outs.iter()
                .filter_map(|out| out.get("value").and_then(|v| v.as_i64()))
                .sum()
        })
        .unwrap_or(0);

    let block_height = block
        .get("height")
        .and_then(|v| v.as_i64())
        .unwrap_or(height);
    let time = block.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
    let difficulty = block
        .get("difficulty")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    Ok(serde_json::json!({
        "height": block_height,
        "hash": hash,
        "time": time,
        "txCount": tx_count,
        "minerReward": miner_reward,
        "difficulty": difficulty,
    }))
}

/// Compact transaction details for the in-app Transaction Info modal, read
/// from the local hsd node via the REST `GET /tx/:hash` route. Node-only.
///
/// **Why REST, not `getrawtransaction`:** the JSON-RPC verbose path can't
/// resolve prevouts for a confirmed tx whose inputs are already spent (those
/// UTXOs left the coin set), so it omits `fee` and `inputs[].coin`. The REST
/// route resolves them through the tx-index, giving a reliable fee even for
/// old txs. Mirrors [`read_block_info`].
///
/// Returns `{ txid, confirmations, height, block, time, fee, inputsCount,
/// outputsCount, totalOut }`. **hsd emits all tx amounts as integer
/// dollarydoos** (matches `NodeCoin.value: i64` at `rpc.rs:561` and the
/// canonical extractors at `db/queries.rs:1854` and `commands/history.rs:123`
/// — no HNS-float conversion anywhere in the hsd RPC path).
///
/// Tri-state return:
/// - the full tx object (normal case);
/// - `{ "error": "tx_index_disabled" }` when the node lacks `--index-tx`
///   (the modal renders a distinct "enable --index-tx" hint);
/// - `Null` for any other soft-degrade (no synced node, unknown tx, generic
///   RPC failure) → the modal shows "requires synced node".
///
/// `fee` is nullable within the tx object: we use hsd's top-level `fee` when
/// present, else compute `Σ inputs[].coin.value − Σ outputs[].value`. It's
/// `null` only for genuine coinbase txs (no real inputs) — rendered as `—`,
/// an honest "unknown" rather than a misleading `0`.
#[tauri::command]
pub async fn read_tx_info(
    state: State<'_, AppState>,
    txid: String,
) -> Result<serde_json::Value, AppError> {
    let txid = txid.trim().to_string();
    if txid.is_empty() || !is_node_ready_for_local_reads(&state).await {
        return Ok(serde_json::Value::Null);
    }

    // Read settings under a short lock, then drop the guard BEFORE the async
    // RPC call — never hold the DB mutex across an .await.
    let settings = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::get_settings(&conn)?
    };
    let node = crate::noncustodial::rpc::NodeRpcClient::from_settings(&settings);

    // REST GET /tx/:hash — resolves spent prevouts via the tx-index, so
    // `fee` and `inputs[].coin` are populated even for old confirmed txs.
    let tx = match node.get_tx_by_hash(&txid).await {
        Ok(t) if !t.is_null() => t,
        Ok(_) => return Ok(serde_json::Value::Null), // 404 / miss
        Err(AppError::Rpc(msg)) if msg.to_ascii_lowercase().contains("tx index not enabled") => {
            // Distinct signal: the node responds but lacks --index-tx. The
            // modal renders a "tx index required" hint rather than the
            // misleading "requires synced node" message.
            return Ok(serde_json::json!({ "error": "tx_index_disabled" }));
        }
        Err(_) => return Ok(serde_json::Value::Null), // generic degrade
    };

    // REST tx shape: { hash, confirmations, height, block, time,
    //                  fee (integer doos), inputs: [ { coin: { value } } ],
    //                  outputs: [ { value (doos) } ] }
    // Guard every access; all amount extraction uses .as_i64().
    let hash = tx
        .get("hash")
        .and_then(|v| v.as_str())
        .unwrap_or(&txid)
        .to_string();
    let confirmations = tx
        .get("confirmations")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let height = tx.get("height").and_then(|v| v.as_i64()).unwrap_or(-1);
    let block = tx
        .get("block")
        .and_then(|v| v.as_str())
        .map(|s| serde_json::Value::String(s.to_string()))
        .unwrap_or(serde_json::Value::Null);
    let time = tx.get("time").and_then(|v| v.as_i64()).unwrap_or(0);

    let inputs_count = tx
        .get("inputs")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let outputs_count = tx
        .get("outputs")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Pure fee + total_out extraction (unit-tested — see tests below).
    let (fee, total_out) = compute_tx_fee_and_total(&tx);

    Ok(serde_json::json!({
        "txid": hash,
        "confirmations": confirmations,
        "height": height,
        "block": block,
        "time": time,
        "fee": fee,
        "inputsCount": inputs_count,
        "outputsCount": outputs_count,
        "totalOut": total_out,
    }))
}

/// Extract the fee and total output value from an hsd `getrawtransaction`
/// verbose response. Pure — no state, no lock, no RPC — so it's the seam we
/// unit-test the fee logic through.
///
/// Returns `(fee, total_out)` where both are in doos and `fee` is `None` when
/// hsd's top-level `fee` is absent AND we can't recover it from the input
/// coins (e.g. coinbase txs, or when hsd's `inputs[].coin.value` isn't fully
/// resolved). `None` intentionally propagates to the JSON as `null`, which
/// the frontend renders as `—` — an honest "unknown" beats a misleading `0`.
pub(crate) fn compute_tx_fee_and_total(tx: &serde_json::Value) -> (Option<i64>, i64) {
    // Sum outputs — integer doos, extracted the same way as
    // db/queries.rs:1854 and commands/history.rs:123.
    let total_out: i64 = tx
        .get("outputs")
        .and_then(|v| v.as_array())
        .map(|outs| {
            outs.iter()
                .filter_map(|out| out.get("value").and_then(|v| v.as_i64()))
                .sum()
        })
        .unwrap_or(0);

    // Try hsd's top-level `fee` first (integer doos when present).
    if let Some(f) = tx.get("fee").and_then(|v| v.as_i64()).filter(|d| *d >= 0) {
        return (Some(f), total_out);
    }

    // Otherwise compute from resolved input coins:
    //   fee = Σ inputs[].coin.value − Σ outputs[].value
    // Bail (return None) if any input lacks a resolved `coin.value` — that
    // covers coinbase txs and any partial hsd response.
    let fee = tx
        .get("inputs")
        .and_then(|v| v.as_array())
        .and_then(|inputs| {
            let mut total_in: i64 = 0;
            for i in inputs {
                let v = i
                    .get("coin")
                    .and_then(|c| c.get("value"))
                    .and_then(|v| v.as_i64())?;
                total_in = total_in.checked_add(v)?;
            }
            let diff = total_in.checked_sub(total_out)?;
            if diff >= 0 {
                Some(diff)
            } else {
                None
            }
        });

    (fee, total_out)
}

#[cfg(test)]
mod tx_fee_tests {
    use super::compute_tx_fee_and_total;
    use serde_json::json;

    #[test]
    fn uses_hsd_top_level_fee_when_present() {
        let tx = json!({
            "fee": 1200,
            "inputs": [
                { "coin": { "value": 500 } },
                { "coin": { "value": 500 } },
            ],
            "outputs": [ { "value": 300 }, { "value": 500 } ],
        });
        // Even though inputs/outputs are present, the explicit fee wins.
        assert_eq!(compute_tx_fee_and_total(&tx), (Some(1200), 800));
    }

    #[test]
    fn computes_fee_from_input_coins_when_hsd_fee_missing() {
        // Common case: hsd omits `fee` on `getrawtransaction` verbose but
        // gives us all input coins. fee = (500 + 400) − (600 + 200) = 100.
        let tx = json!({
            "inputs": [
                { "coin": { "value": 500 } },
                { "coin": { "value": 400 } },
            ],
            "outputs": [ { "value": 600 }, { "value": 200 } ],
        });
        assert_eq!(compute_tx_fee_and_total(&tx), (Some(100), 800));
    }

    #[test]
    fn returns_none_fee_when_any_input_coin_unresolved() {
        // Coinbase-shape: input has no `coin` field. Fee is genuinely
        // unknowable at this layer → None (renders as `—`).
        let tx = json!({
            "inputs": [
                { "prevout": { "hash": "00", "index": 4294967295u32 } },
            ],
            "outputs": [ { "value": 2_000_000_000i64 } ],
        });
        assert_eq!(compute_tx_fee_and_total(&tx), (None, 2_000_000_000));
    }

    #[test]
    fn returns_zero_total_out_when_outputs_missing() {
        // Defensive: a malformed response with no outputs shouldn't panic.
        let tx = json!({});
        assert_eq!(compute_tx_fee_and_total(&tx), (None, 0));
    }

    #[test]
    fn negative_hsd_fee_falls_through_to_computation() {
        // Defensive: a negative top-level fee (shouldn't happen but guard it)
        // is ignored, and we recompute from coins.
        let tx = json!({
            "fee": -5,
            "inputs": [ { "coin": { "value": 1000 } } ],
            "outputs": [ { "value": 900 } ],
        });
        assert_eq!(compute_tx_fee_and_total(&tx), (Some(100), 900));
    }
}

/// Transaction history from the local (node-synced) cache.
/// `wallet_profile_id` pins the read to a specific wallet (defaults to active).
#[tauri::command]
pub async fn read_transactions(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(serde_json::Value::Array(vec![])),
    };
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    Ok(serde_json::Value::Array(queries::read_cached_transactions(
        &conn, &id,
    )?))
}

// ============================================================================
// Renewals (Task 3 / C3): days-until-expiry computed LIVE from chain data
// ============================================================================

/// One name in the Renewals view.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenewalRow {
    pub name: String,
    pub state: Option<String>,
    /// Chain renewal height (`tracked_name_states.renewal_height`), when known.
    pub renewal_height: Option<i64>,
    /// `renewal_height + renewal_window` for chain rows; the CSV column for
    /// csv-import rows.
    pub expires_at_height: Option<i64>,
    pub blocks_until_expire: Option<i64>,
    pub days_until_expire: Option<f64>,
    /// `"chain"` — computed from tracked chain state; `"csv-import"` — stale
    /// CSV columns from the `assets` inventory (fallback only).
    pub source: String,
    /// `days_until_expire` known and ≤ the threshold (incl. negative = lapsed).
    pub expiring_soon: bool,
}

/// Response of `read_renewals`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenewalsResponse {
    pub wallet_profile_id: Option<String>,
    /// The height days were computed against, when one is known.
    pub current_height: Option<i64>,
    /// `"node"` — live height from a connected, fully synced node.
    /// `"explorer"` — estimate persisted by the last sync (explorer name-stats
    /// snapshots and/or the last node-synced height), extrapolated by wall
    /// time at ~10-minute blocks. NOT a live read.
    /// `"unknown"` — no height available; chain days are null, never invented.
    pub height_source: String,
    /// Frontend copy of [`crate::commands::names::EXPIRING_SOON_THRESHOLD_DAYS`].
    pub expiring_soon_threshold_days: f64,
    pub names: Vec<RenewalRow>,
}

fn empty_renewals() -> RenewalsResponse {
    RenewalsResponse {
        wallet_profile_id: None,
        current_height: None,
        height_source: "unknown".into(),
        expiring_soon_threshold_days: crate::commands::names::EXPIRING_SOON_THRESHOLD_DAYS,
        names: Vec::new(),
    }
}

/// Best persisted estimate of the current chain height when no synced node is
/// available, extrapolated to "now" by elapsed wall time (~10-minute blocks).
/// Extrapolation matters for safety: a stale snapshot UNDERestimates the
/// height and therefore INFLATES days-until-expiry — the dangerous direction.
///
/// Candidates (max wins):
/// * per-name explorer/node stats snapshots persisted in
///   `tracked_name_states.raw_json` — `renewalPeriodEnd - blocksUntilExpire`
///   is the chain height the stats were computed at, aged by `updated_at`;
/// * `wallet_profiles.last_synced_height`, aged by `last_synced_at`.
pub(crate) fn estimate_persisted_height(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Option<i64>, AppError> {
    let mut best: Option<i64> = None;
    let mut consider = |h: Option<i64>| {
        if let Some(h) = h {
            best = Some(best.map_or(h, |b| b.max(h)));
        }
    };

    // Per-name stats snapshots. raw_json is either the explorer HsdName shape
    // (stats at the root) or the node getnameinfo result ({"info": {...}}).
    let mut stmt = conn.prepare(
        "SELECT raw_json,
                CAST((strftime('%s','now') - strftime('%s', updated_at)) / 600 AS INTEGER)
         FROM tracked_name_states
         WHERE wallet_profile_id = ?1 AND raw_json IS NOT NULL",
    )?;
    let rows = stmt.query_map(rusqlite::params![profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (raw, elapsed_blocks) = row?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let info = match v.get("info") {
            Some(i) if !i.is_null() => i,
            _ => &v,
        };
        let Some(stats) = info.get("stats").filter(|s| !s.is_null()) else {
            continue;
        };
        let end = stats.get("renewalPeriodEnd").and_then(|x| x.as_i64());
        let until = stats.get("blocksUntilExpire").and_then(|x| x.as_i64());
        if let (Some(end), Some(until)) = (end, until) {
            consider(Some(end - until + elapsed_blocks.max(0)));
        }
    }

    // Last node-synced height (stale, but still a floor), aged the same way.
    let profile_snapshot: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT last_synced_height,
                    CAST((strftime('%s','now') - strftime('%s', COALESCE(last_synced_at, datetime('now')))) / 600 AS INTEGER)
             FROM wallet_profiles WHERE id = ?1",
            rusqlite::params![profile_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((Some(h), elapsed_blocks)) = profile_snapshot {
        consider(Some(h + elapsed_blocks.max(0)));
    }

    Ok(best)
}

/// Pure DB + math core of `read_renewals` (testable without Tauri state).
///
/// `live_node_height` is the tip of a connected, fully synced node when one is
/// available (→ `heightSource: "node"`); otherwise the best persisted estimate
/// from the last sync is used (`"explorer"`), and when nothing is available at
/// all (`"unknown"`) chain rows keep `days_until_expire: null` — the number is
/// never fabricated.
///
/// Row sources:
/// * every owned name (same ownership semantics as `read_names`) with a chain
///   `renewal_height` → `source: "chain"`, days computed live;
/// * an owned name without chain renewal data falls back to the `assets` CSV
///   columns when present → `source: "csv-import"`;
/// * CSV inventory rows with expiry data for names not otherwise covered are
///   included as `"csv-import"` (the `assets` inventory is global, not
///   per-profile — same as the legacy Renewals screen).
pub(crate) fn compute_renewals(
    conn: &rusqlite::Connection,
    profile_id: &str,
    live_node_height: Option<i64>,
) -> Result<RenewalsResponse, AppError> {
    use crate::noncustodial::network::{Network, BLOCKS_PER_DAY};

    let threshold = crate::commands::names::EXPIRING_SOON_THRESHOLD_DAYS;
    let network = queries::get_wallet_profile(conn, profile_id)?
        .and_then(|p| Network::from_str_opt(&p.network))
        .unwrap_or_default();
    let renewal_window = network.name_params().renewal_window as i64;

    let (current_height, height_source) = match live_node_height {
        Some(h) => (Some(h), "node"),
        None => match estimate_persisted_height(conn, profile_id)? {
            Some(h) => (Some(h), "explorer"),
            None => (None, "unknown"),
        },
    };

    // CSV expiry columns from the migration inventory (fallback source).
    // (name_state, expires_at_height, days_until_expire) keyed by lowercased TLD.
    type CsvExpiry = (Option<String>, Option<i64>, Option<f64>);
    let mut csv: std::collections::HashMap<String, CsvExpiry> = {
        let mut stmt = conn.prepare(
            "SELECT tld, name_state, expires_at_height, days_until_expire
             FROM assets
             WHERE expires_at_height IS NOT NULL OR days_until_expire IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get(1)?, row.get(2)?, row.get(3)?),
            ))
        })?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (tld, data) = r?;
            map.insert(tld.trim().to_lowercase(), data);
        }
        map
    };

    let expiring = |days: Option<f64>| days.map(|d| d <= threshold).unwrap_or(false);
    // Safety override for CSV-imported rows: `days_until_expire` is a
    // point-in-time snapshot from an unverified/legacy import path (no current
    // code writes these `assets` columns — see task-3 report), so we do NOT
    // trust its exact value or recompute it wholesale. But it and a known
    // `current_height` share the same axis (both are Handshake chain
    // heights — the column is literally named/exported as "Expires At
    // Height"), so once the chain has clearly passed it we CAN safely
    // override to expired styling rather than replay a stale reassurance
    // (green "200d" for an already-expired name is the dangerous direction;
    // see Finding 1). When not yet past, the stored value is left untouched.
    let csv_row =
        |name: &str, (state, expires_at, days): (Option<String>, Option<i64>, Option<f64>)| {
            let (days, blocks, forced_expired) = match (current_height, expires_at) {
                (Some(h), Some(exp)) if h > exp => {
                    let blocks = exp - h; // negative: already past the imported expiry height
                    (Some(blocks as f64 / BLOCKS_PER_DAY), Some(blocks), true)
                }
                _ => (days, None, false),
            };
            RenewalRow {
                name: name.to_string(),
                state,
                renewal_height: None,
                expires_at_height: expires_at,
                blocks_until_expire: blocks,
                days_until_expire: days,
                source: "csv-import".into(),
                expiring_soon: forced_expired || expiring(days),
            }
        };

    let mut names: Vec<RenewalRow> = Vec::new();

    // Owned names — same union as `read_names` (node-synced cache + explorer
    // discoveries), so the Renewals screen covers exactly what the wallet owns.
    for v in collect_read_names_data(conn, profile_id, live_node_height.is_some())? {
        let Some(name) = v.get("name").and_then(|x| x.as_str()) else {
            continue;
        };
        let key = name.trim().to_lowercase();
        let state = v
            .get("state")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let renewal_height = v.get("renewal").and_then(|x| x.as_i64());

        match renewal_height {
            Some(renewal) => {
                // Chain data: expiry = renewal height + network renewal window.
                csv.remove(&key);
                let expires_at = renewal + renewal_window;
                let blocks = current_height.map(|h| expires_at - h);
                let days = blocks.map(|b| b as f64 / BLOCKS_PER_DAY);
                names.push(RenewalRow {
                    name: key,
                    state,
                    renewal_height: Some(renewal),
                    expires_at_height: Some(expires_at),
                    blocks_until_expire: blocks,
                    days_until_expire: days,
                    source: "chain".into(),
                    expiring_soon: expiring(days),
                });
            }
            None => match csv.remove(&key) {
                // No chain renewal data → stale CSV columns, honestly marked.
                Some(data) => names.push(csv_row(&key, data)),
                None => names.push(RenewalRow {
                    name: key,
                    state,
                    renewal_height: None,
                    expires_at_height: None,
                    blocks_until_expire: None,
                    days_until_expire: None,
                    source: "chain".into(),
                    expiring_soon: false,
                }),
            },
        }
    }

    // Remaining CSV inventory rows (names not owned/tracked by this profile).
    for (tld, data) in csv {
        names.push(csv_row(&tld, data));
    }

    // Most urgent first; unknown-expiry rows last.
    names.sort_by(|a, b| {
        match (a.days_until_expire, b.days_until_expire) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.name.cmp(&b.name))
    });

    Ok(RenewalsResponse {
        wallet_profile_id: Some(profile_id.to_string()),
        current_height,
        height_source: height_source.into(),
        expiring_soon_threshold_days: threshold,
        names,
    })
}

/// Renewal/expiry data for the Renewals screen, computed LIVE from chain data
/// (never the stale CSV import alone — see [`compute_renewals`]).
/// `wallet_profile_id` pins the read to a specific wallet (defaults to active).
#[tauri::command]
pub async fn read_renewals(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<RenewalsResponse, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(empty_renewals()),
    };
    // Probe the node BEFORE taking the DB lock (the guard is !Send).
    let live_height = node_tip_height_if_synced(&state).await;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    compute_renewals(&conn, &id, live_height)
}

/// Reconcile owned names in the local cache against live explorer data,
/// repairing missing/incorrect fields so the owned-names list and auctions
/// view are accurate.
///
/// This is the in-app fix for the common case where:
/// - Namebase-transferred names are in the `assets` inventory but NOT in
///   `tracked_name_states` (or have incomplete `raw_json`)
/// - Explorer-discovered names lack `registered` / `expired` because the node
///   RPC payload doesn't include those fields
///
/// The command looks up each name in the explorer, determines if the wallet
/// still owns it, and upserts an authoritative `tracked_name_states` row.
#[tauri::command]
pub async fn repair_owned_names(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let id = match active_profile(&state)? {
        Some(id) => id,
        None => return Ok(serde_json::json!({"repaired":0,"discovered":0,"errors":[]})),
    };

    // Node-only path: when the node is authoritative, `getnameinfo`'s
    // `owner:{hash,index}` IS the current owner outpoint — no explorer history
    // crawl needed. We iterate the same candidate set (inventory TLDs + tracked
    // names) but resolve state and ownership directly against the node.
    if is_node_ready_for_local_reads(&state).await {
        return repair_owned_names_via_node(&state, &id).await;
    }

    let (client, inventory_tlds, tracked, all_addresses) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&conn)?;
        (
            explorer_client(&settings),
            queries::get_inventory_tlds(&conn)?,
            queries::list_tracked_name_names(&conn, &id)?,
            queries::get_profile_addresses(&conn, &id)?,
        )
    };
    let addr_set: HashSet<String> = all_addresses.iter().cloned().collect();

    // Build the candidate set: inventory TLDs + currently tracked names.
    let mut candidates: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for name in inventory_tlds.iter().chain(tracked.iter()) {
        let n = name.trim().to_lowercase();
        if n.is_empty() || !seen.insert(n.clone()) {
            continue;
        }
        candidates.push(n);
    }

    let mut errors: Vec<String> = Vec::new();
    let mut repaired = 0u32;

    for name in &candidates {
        sleep(DISCOVERY_THROTTLE).await;
        let info_opt = match client.get_name_info_optional(name).await {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{name}: lookup failed — {e}"));
                continue;
            }
        };

        // Resolve the CURRENT owner the correct way: via `/history` + the owner
        // tx's output *address* (Task 2), NOT the dead `info.owner.hash` (a txid).
        // Throttle again here: `get_name_info_optional` above and the resolver's
        // first internal call are two separate explorer HTTP calls, so every
        // call must be preceded by a sleep.
        sleep(DISCOVERY_THROTTLE).await;
        let resolution =
            crate::commands::sync::resolve_owner_via_history(&client, name, &addr_set).await;

        match resolution {
            Ok(Some(res)) if res.owned_by_wallet => {
                if let Some(info) = &info_opt {
                    // Upsert with live explorer data (includes `registered`/`expired`)
                    // and advance the inventory row to finalized_owned.
                    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                    queries::upsert_owned_name(
                        &conn,
                        &id,
                        info,
                        &res.owner_txid,
                        res.owner_vout,
                        &res.owner_address,
                    )?;
                    queries::mark_asset_finalized_owned(&conn, name, info.state.as_deref())?;
                    repaired += 1;
                } else {
                    // Owned per history but name-info 404'd — record the check.
                    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                    queries::touch_asset_synced(&conn, name)?;
                }
            }
            // Checked and not owned: stamp last_synced_at so repeated runs converge.
            Ok(_) => {
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                queries::touch_asset_synced(&conn, name)?;
            }
            Err(e) => {
                errors.push(format!("{name}: owner resolution failed — {e}"));
            }
        }
    }

    Ok(serde_json::json!({
        "repaired": repaired,
        "errors": errors,
        "candidates": candidates.len(),
    }))
}

/// List every derived receive-branch address for a wallet, each tagged with
/// whether it has been used (referenced by a tracked UTXO or bid commitment).
///
/// Used purely for the "all addresses" list in the Receive card. Returns an
/// empty list when no profile is resolved or no addresses have been derived
/// yet (a fresh wallet before its first sync/provision).
///
/// `wallet_profile_id` pins the read to a specific wallet (defaults to active).
#[tauri::command]
pub async fn list_receive_addresses(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<Vec<queries::ReceiveAddressRow>, AppError> {
    let id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(Vec::new()),
    };
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let account_index = match queries::get_wallet_profile(&conn, &id)? {
        Some(p) => p.account_index as u32,
        None => return Ok(Vec::new()),
    };
    queries::list_receive_addresses(&conn, &id, account_index)
}

/// Allocate the next unused receive-branch address and persist it, returning
/// the derived address string. Because the new address is written to
/// `derived_addresses`, the next sync scan picks it up automatically (no
/// separate re-derivation needed).
///
/// Watch-only profiles can still derive from their account xpub, so this is
/// allowed for every profile kind. `wallet_profile_id` pins the write to a
/// specific wallet (defaults to active).
#[tauri::command]
pub async fn derive_next_receive_address(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<String, AppError> {
    let id = resolve_profile(&state, wallet_profile_id)?
        .ok_or_else(|| AppError::InvalidInput("no active wallet profile".into()))?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = queries::get_wallet_profile(&conn, &id)?
        .ok_or_else(|| AppError::InvalidInput("wallet profile not found".into()))?;
    let network = crate::noncustodial::derivation::network_from_profile(&profile.network)?;
    let xpub = crate::noncustodial::hd::ExtendedPubKey::from_xpub(network, &profile.account_xpub)?;
    let derived = crate::noncustodial::derivation::next_unused_receive_address(
        &conn,
        &id,
        profile.account_index as u32,
        network,
        &xpub,
    )?;
    Ok(derived.address)
}
