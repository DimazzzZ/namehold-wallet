//! Covenant / name action commands: build OPEN/BID/REVEAL/REDEEM/REGISTER/
//! UPDATE/RENEW/TRANSFER/FINALIZE/CANCEL/REVOKE drafts.
//!
//! Each command resolves the active profile, fetches the current name state from
//! the node (`getnameinfo`), constructs the covenant + funded plan via
//! `noncustodial::actions`, and persists a `wallet_tx_drafts` row. Signing and
//! broadcast reuse `commands::tx::{sign_tx_draft, broadcast_tx_draft}` — covenant
//! draft plans are signed by `actions::sign_plan` (dispatched there by action).
//!
//! NOTE: on-chain validity (value math, renewal-block selection, bid matching)
//! must be validated against a regtest node before mainnet use; the default
//! network is regtest and writes are gated by the unlocked signer + broadcaster.

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::{self, queries};
use crate::error::AppError;
use crate::noncustodial::actions::{self, NameInputSpec, PrimaryOutput};
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::noncustodial::send::{self, SpendableCoin};
use crate::noncustodial::sync::{self, COV_REVEAL, COV_REGISTER};
use crate::noncustodial::tx::sighash;
use crate::noncustodial::types::TxDraftSummary;
use crate::noncustodial::{address, bids, covenants, names, resource};
use crate::AppState;

pub(crate) fn random_id() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Resolved, secret-free build context for a covenant action.
#[derive(Debug)]
pub(crate) struct Ctx {
    pub(crate) profile_id: String,
    pub(crate) network: Network,
    pub(crate) account: u32,
    pub(crate) account_xpub: ExtendedPubKey,
    pub(crate) change_address: String,
    pub(crate) funding: Vec<SpendableCoin>,
    pub(crate) settings: std::collections::HashMap<String, String>,
}

pub(crate) fn load_ctx(state: &State<'_, AppState>) -> Result<Ctx, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let id = queries::get_active_profile_id(&conn)?;
    if id.is_empty() {
        return Err(AppError::InvalidInput("no active wallet profile".into()));
    }
    let profile = queries::get_wallet_profile(&conn, &id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))?;
    if profile.watch_only {
        return Err(AppError::InvalidInput("active profile is watch-only".into()));
    }
    let network = crate::noncustodial::derivation::network_from_profile(&profile.network)?;
    let account_xpub = ExtendedPubKey::from_xpub(network, &profile.account_xpub)?;
    let change = crate::noncustodial::derivation::derive_one(
        network,
        &account_xpub,
        crate::noncustodial::derivation::BRANCH_CHANGE,
        0,
    )?;
    let funding = send::load_spendable_coins(&conn, &id, None)?;
    let settings = queries::get_settings(&conn)?;
    Ok(Ctx {
        profile_id: id,
        network,
        account: profile.account_index as u32,
        account_xpub,
        change_address: change.address,
        funding,
        settings,
    })
}

pub(crate) fn fee_rate(ctx: &Ctx, fee_rate: Option<u64>) -> u64 {
    fee_rate
        .or_else(|| {
            ctx.settings
                .get("fee_rate_doos_per_kvb")
                .and_then(|s| s.parse::<u64>().ok())
                .map(|kvb| (kvb / 1000).max(send::MIN_FEE_RATE_PER_BYTE))
        })
        .unwrap_or(send::DEFAULT_FEE_RATE_PER_BYTE)
}

/// Minimal view of `getnameinfo` we need to build covenants.
#[derive(Debug)]
pub(crate) struct NameState {
    pub(crate) height: u32,
    pub(crate) value: u64,
    pub(crate) renewals: u32,
    pub(crate) claimed: u32,
    pub(crate) weak: bool,
}

pub(crate) async fn fetch_name_state(client: &NodeRpcClient, name: &str) -> Result<NameState, AppError> {
    let v = client.get_name_info(name).await?;
    let info = v.get("info");
    let info = match info {
        Some(i) if !i.is_null() => i,
        _ => return Err(AppError::InvalidInput(format!("name '{name}' has no on-chain state"))),
    };
    let geti = |k: &str| info.get(k).and_then(|x| x.as_i64());
    Ok(NameState {
        height: geti("height").unwrap_or(0) as u32,
        value: geti("value").unwrap_or(0) as u64,
        renewals: geti("renewals").unwrap_or(0) as u32,
        claimed: geti("claimed").unwrap_or(0) as u32,
        weak: info.get("weak").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

/// `getRenewalBlock`: internal-order 32-byte hash at `height - 2*renewalMaturity`.
pub(crate) async fn renewal_block(client: &NodeRpcClient, network: Network) -> Result<[u8; 32], AppError> {
    let tip = client.get_blockchain_info().await?.blocks;
    let maturity = network.name_params().renewal_maturity as i64;
    let height = (tip - 2 * maturity).max(0);
    let hash_hex = client.get_block_hash(height).await?;
    let bytes = hex::decode(&hash_hex)
        .map_err(|e| AppError::Rpc(format!("bad block hash: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::Rpc("block hash not 32 bytes".into()));
    }
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    h.reverse(); // display -> internal
    Ok(h)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionSummary<'a> {
    action: &'a str,
    name: &'a str,
    send_total_doos: i64,
    fee_doos: i64,
    change_doos: i64,
    input_total_doos: i64,
    num_inputs: i64,
    recipient_address: Option<&'a str>,
    txid: Option<&'a str>,
}

/// Persist a planned covenant draft and return its summary.
fn persist(
    state: &State<'_, AppState>,
    profile_id: &str,
    action: &str,
    name: &str,
    recipient: Option<&str>,
    res: &actions::PlanResult,
) -> Result<TxDraftSummary, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    persist_with_conn(&conn, profile_id, action, name, recipient, res)
}

/// Same as [`persist`] but takes an already-held connection instead of
/// locking `state.db` itself. Lets a caller that needs its own writes (e.g.
/// `build_bid_draft`'s bid-multiplicity guard + commitment insert, I2) to run
/// in the SAME critical section as the draft insert + coin reservation,
/// closing the race window a separate lock/unlock per step would leave open.
fn persist_with_conn(
    conn: &rusqlite::Connection,
    profile_id: &str,
    action: &str,
    name: &str,
    recipient: Option<&str>,
    res: &actions::PlanResult,
) -> Result<TxDraftSummary, AppError> {
    let summary = ActionSummary {
        action,
        name,
        send_total_doos: res.plan.outputs[0].value as i64,
        fee_doos: res.fee as i64,
        change_doos: res.change as i64,
        input_total_doos: res.input_total as i64,
        num_inputs: res.plan.inputs.len() as i64,
        recipient_address: recipient,
        txid: Some(&res.txid),
    };
    let id = random_id();
    // Reserve every input the plan spends (I3): the funding coins AND, when
    // present, the name UTXO itself — two covenant drafts must not be able to
    // grab the same name coin (e.g. two REVEALs) any more than two plain
    // sends can grab the same liquid coin.
    let reserved_inputs: Vec<(String, u32)> =
        res.plan.inputs.iter().map(|i| (i.txid.clone(), i.vout)).collect();
    db::queries::insert_tx_draft_reserving_coins(
        conn,
        &id,
        profile_id,
        action,
        &res.unsigned_tx_hex,
        &serde_json::to_string(&res.plan)?,
        &serde_json::to_string(&summary)?,
        &reserved_inputs,
    )?;
    db::queries::get_tx_draft(conn, &id)?
        .map(|d| d.to_summary())
        .ok_or_else(|| AppError::Other("draft vanished after insert".into()))
}

pub(crate) fn name_input_from(coin: queries::NameCoin) -> NameInputSpec {
    NameInputSpec {
        txid: coin.txid,
        vout: coin.vout,
        value: coin.value,
        branch: coin.branch,
        child_index: coin.child_index,
        sighash_type: sighash::ALL,
    }
}

// ============================================================================
// Name action capability model
// ============================================================================

/// Whether a specific action is allowed for the current wallet/name context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameActionCapability {
    pub allowed: bool,
    pub reason: Option<String>,
}

/// User-facing task state derived from the raw phase + wallet evidence.
/// Serialized as a camelCase string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuctionTaskState {
    #[serde(rename = "availableToOpen")]
    AvailableToOpen,
    #[serde(rename = "waitingForBidding")]
    WaitingForBidding,
    #[serde(rename = "readyToBid")]
    ReadyToBid,
    #[serde(rename = "readyToReveal")]
    ReadyToReveal,
    #[serde(rename = "wonNeedsRegister")]
    WonNeedsRegister,
    #[serde(rename = "lostNeedsRedeem")]
    LostNeedsRedeem,
    #[serde(rename = "transferPendingFinalize")]
    TransferPendingFinalize,
    #[serde(rename = "ownedNoUrgentAction")]
    OwnedNoUrgentAction,
    #[serde(rename = "expiringSoon")]
    ExpiringSoon,
    #[serde(rename = "unavailableOther")]
    UnavailableOther,
}

/// Days-until-expiry threshold below which an owned name's task state becomes
/// [`AuctionTaskState::ExpiringSoon`] (and the Renewals screen flags the row).
/// A missed renewal on Handshake loses the name forever, so this errs early.
/// (A settings-configurable threshold was considered and skipped for now —
/// the constant is the single source of truth, surfaced to the frontend via
/// `read_renewals.expiringSoonThresholdDays`.)
pub const EXPIRING_SOON_THRESHOLD_DAYS: f64 = 30.0;

/// Full capability response for a name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameActionCapabilities {
    pub name: String,
    pub phase: String,
    pub task_state: AuctionTaskState,
    pub owns_name: bool,
    pub has_bid_commitment: bool,
    pub has_bid_coin: bool,
    pub has_reveal_coin: bool,
    pub has_owner_coin: bool,
    pub can_open: NameActionCapability,
    pub can_bid: NameActionCapability,
    pub can_reveal: NameActionCapability,
    pub can_redeem: NameActionCapability,
    pub can_register: NameActionCapability,
    pub can_update: NameActionCapability,
    pub can_transfer: NameActionCapability,
    pub can_finalize: NameActionCapability,
    pub can_cancel_transfer: NameActionCapability,
    pub can_renew: NameActionCapability,
    pub can_revoke: NameActionCapability,
    pub next_action_key: Option<String>,
    pub next_action_label: Option<String>,
    pub next_action_reason: Option<String>,
    pub countdown_label: Option<String>,
    pub countdown_blocks: Option<i64>,
    pub countdown_hours: Option<f64>,
}

/// Context gathered from the DB for a name action evaluation.
pub(crate) struct NameActionContext {
    pub has_bid_commitment: bool,
    /// Unspent COV_BID coin for this name (the coin a REVEAL spends). Gates
    /// [`can_reveal`] — see the doc comment on that field below for why
    /// `has_reveal_coin` is the WRONG signal for revealing.
    pub has_bid_coin: bool,
    /// Unspent COV_REVEAL coin for this name. Only ever exists AFTER a
    /// reveal has already happened (a REVEAL tx spends the BID coin and
    /// creates this one) — so it gates [`can_redeem`] (reclaiming a losing
    /// reveal), never [`can_reveal`] itself.
    pub has_reveal_coin: bool,
    pub has_owner_coin: bool,
    pub owner_covenant_type: Option<i64>,
    pub name_height: Option<i64>,
    pub transfer_has_items: Option<bool>,
    pub existing_bid_count: i64,
}

/// Gather wallet evidence from the DB for a name.
pub(crate) fn find_name_action_context(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<NameActionContext, AppError> {
    let bid = queries::get_bid_commitment(conn, profile_id, name)
        .ok()
        .flatten();
    // Part 3 (confirmed pre-existing bug, folded in from the Task 2 review):
    // revealing SPENDS the COV_BID coin and CREATES a COV_REVEAL coin — so a
    // COV_REVEAL coin can only exist AFTER a successful reveal, never before.
    // `can_reveal` must gate on the coin a reveal actually spends
    // (`has_bid_coin`, COV_BID), not on `has_reveal_coin` (COV_REVEAL), which
    // was previously used for both `can_reveal` AND `can_redeem` — making the
    // Reveal button permanently disabled for a perfectly healthy unrevealed
    // bid. Both lookups share the same address (a reveal's output always
    // lands back on the bid coin's own address, see `build_reveal_draft`), so
    // only the covenant type differs between the two queries below.
    let bid_coin = bid.as_ref().and_then(|b| {
        queries::find_unspent_covenant_utxo(
            conn,
            profile_id,
            &b.address,
            sync::COV_BID as i64,
            name,
            &b.name_hash_hex,
        )
        .ok()
        .flatten()
    });
    let reveal_coin = bid.as_ref().and_then(|b| {
        queries::find_unspent_covenant_utxo(
            conn,
            profile_id,
            &b.address,
            COV_REVEAL as i64,
            name,
            &b.name_hash_hex,
        )
        .ok()
        .flatten()
    });
    let owner_coin = queries::get_name_coin(conn, profile_id, name)
        .ok()
        .flatten();
    let owner_cov_type = owner_coin.as_ref().map(|c| c.covenant_type);
    let nh = owner_coin.as_ref().and_then(|c| c.name_height);

    // Check if there's a TRANSFER output we can finalize.
    let transfer = owner_coin.as_ref().and_then(|c| {
        c.covenant_json.as_ref().and_then(|j| {
            let v: serde_json::Value = serde_json::from_str(j).ok()?;
            let items = v.get("items")?.as_array()?;
            Some(items.len() >= 4)
        })
    });

    // Count existing bids for bid multiplicity rule.
    let existing_bid_count = queries::list_bid_commitments(conn, profile_id)
        .map(|v| v.iter().filter(|b| b.name == name).count() as i64)
        .unwrap_or(0);

    Ok(NameActionContext {
        has_bid_commitment: bid.is_some(),
        has_bid_coin: bid_coin.is_some(),
        has_reveal_coin: reveal_coin.is_some(),
        has_owner_coin: owner_coin.is_some(),
        owner_covenant_type: owner_cov_type,
        name_height: nh,
        transfer_has_items: transfer,
        existing_bid_count,
    })
}

/// Reason surfaced on spend-capable actions when a name is owned per explorer
/// evidence but its owner coin has not been synced from a node yet. Nothing may
/// build a spend without a real local `tracked_utxos` owner coin.
const OWNER_COIN_NOT_SYNCED_REASON: &str =
    "owner coin not synced locally — connect a node and Refresh to manage";

/// Evaluate what the wallet can do with `name` right now.
///
/// `wallet_profile_id` pins the evaluation to a specific wallet (per-wallet read
/// isolation), defaulting to the active profile — resolved exactly like
/// `read_names` via [`crate::commands::read::resolve_profile`].
///
/// The node RPC (`getnameinfo`) is authoritative ONLY when the node is fully
/// synced. When the node is unreachable OR still catching up (its wallet scan is
/// incomplete, so its owner-coin view is unreliable), we fall back to LOCAL Sync
/// evidence (`tracked_name_states`). A name whose recorded owner address is one
/// of this wallet's addresses is classified as owned (`owns_name = true`) even
/// though its owner coin is not synced — but every spend-capable action is
/// forced to `allowed: false` with a "not synced locally" reason, since a spend
/// still requires a real node-synced owner coin. This symmetry holds on both the
/// synced-node path and the fallback path. Only a genuinely unknown name (no
/// tracked row at all) on the fallback path yields `conservative_capabilities`.
#[tauri::command]
pub async fn get_name_action_capabilities(
    state: State<'_, AppState>,
    name: String,
    wallet_profile_id: Option<String>,
) -> Result<NameActionCapabilities, AppError> {
    // Resolve the target profile the same way reads do — never trust a backend
    // "active" notion over the caller's explicit id.
    let profile_id = match crate::commands::read::resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(conservative_capabilities(&name, "no active wallet profile")),
    };
    evaluate_name_action_capabilities(&state, name, &profile_id).await
}

/// Max names accepted by [`get_names_action_capabilities`] per call. Each
/// per-name evaluation may do its own node RPC round-trip plus a handful of
/// DB queries, so an unbounded batch could block the UI thread on a huge
/// fan-out; 200 comfortably covers any real wallet's active-auction list.
/// Callers must chunk beyond that (none currently need to).
pub const MAX_NAMES_ACTION_CAPABILITIES_BATCH: usize = 200;

/// Batch form of [`get_name_action_capabilities`] (F5 fix) — resolves the
/// wallet profile ONCE (per-wallet read isolation, exactly like every other
/// read command: an explicit `wallet_profile_id` wins over the backend's
/// "active" notion) and then runs the identical per-name evaluation for each
/// name. This is what lets `AuctionsView` fetch capabilities for its whole
/// task list in one invoke instead of spawning N+1 requests (one per row).
///
/// When no wallet profile can be resolved, every name gets the same
/// conservative fallback the single-name command returns in that case —
/// still one result per input name, never an empty list.
#[tauri::command]
pub async fn get_names_action_capabilities(
    state: State<'_, AppState>,
    names: Vec<String>,
    wallet_profile_id: Option<String>,
) -> Result<Vec<NameActionCapabilities>, AppError> {
    if names.len() > MAX_NAMES_ACTION_CAPABILITIES_BATCH {
        return Err(AppError::InvalidInput(format!(
            "batch too large: {} names (max {})",
            names.len(),
            MAX_NAMES_ACTION_CAPABILITIES_BATCH
        )));
    }
    let profile_id = match crate::commands::read::resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => {
            return Ok(names
                .into_iter()
                .map(|n| conservative_capabilities(&n, "no active wallet profile"))
                .collect());
        }
    };
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        out.push(evaluate_name_action_capabilities(&state, name, &profile_id).await?);
    }
    Ok(out)
}

/// Core per-name evaluation, shared by [`get_name_action_capabilities`] and
/// [`get_names_action_capabilities`] — takes an ALREADY-resolved
/// `profile_id` so the batch command only resolves the profile once instead
/// of once per name.
async fn evaluate_name_action_capabilities(
    state: &State<'_, AppState>,
    name: String,
    profile_id: &str,
) -> Result<NameActionCapabilities, AppError> {
    let settings = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::get_settings(&conn)?
    };
    let client = NodeRpcClient::from_settings(&settings);

    // 1. Fetch the name's on-chain state via RPC — but ONLY trust the node when
    //    it is fully synced. An unsynced node answers RPC yet its wallet scan is
    //    incomplete, so `has_owner_coin` (from `tracked_utxos`) would be
    //    spuriously false for names we actually own. Treat "reachable but not
    //    synced" exactly like unreachable: fall back to local Sync evidence.
    //    Reuses the same gate as `read_balance`/`read_names` — no duplicate logic.
    let name_info = if crate::commands::read::is_node_ready_for_local_reads(&state).await {
        client.get_name_info(&name).await.ok()
    } else {
        None
    };

    match name_info {
        Some(name_info) => {
            let raw_phase = name_info
                .get("info")
                .and_then(|i| i.get("state"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_uppercase())
                .unwrap_or_else(|| "AVAILABLE".to_string());

            let (action_ctx, tracked_owner_address, profile_addrs) = {
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                let action_ctx = find_name_action_context(&conn, &profile_id, &name)?;
                let tracked_owner_address =
                    queries::get_tracked_name_state(&conn, &profile_id, &name)?
                        .and_then(|t| t.owner_address);
                let profile_addrs = queries::get_profile_addresses(&conn, &profile_id)?;
                (action_ctx, tracked_owner_address, profile_addrs)
            };
            let stats = name_info.get("info").and_then(|i| i.get("stats"));

            // Symmetric ownership: even on a synced node the owner-coin scan may
            // not have reached this name yet. Explorer evidence (recorded owner
            // address is ours) still classifies it as owned — but WITHOUT a real
            // node-synced owner coin, every spend stays locked, exactly as on the
            // node-unreachable path. This keeps the invariant airtight: explorer
            // evidence gives classification, never spend capability.
            let explorer_owned = tracked_owner_address
                .as_deref()
                .map(|a| profile_addrs.iter().any(|p| p == a))
                .unwrap_or(false);
            let owns_name = action_ctx.has_owner_coin || explorer_owned;
            let spend_locked = !action_ctx.has_owner_coin;
            Ok(build_name_action_capabilities(
                name,
                raw_phase.clone(),
                &raw_phase,
                stats,
                &action_ctx,
                owns_name,
                spend_locked,
                None,
            ))
        }
        None => {
            // Node unreachable or not synced: fall back to local Sync evidence
            // rather than blindly declaring "nothing allowed".
            let (tracked, action_ctx, profile_addrs, renewal_window, current_height) = {
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                let tracked = queries::get_tracked_name_state(&conn, &profile_id, &name)?;
                let action_ctx = find_name_action_context(&conn, &profile_id, &name)?;
                let addrs = queries::get_profile_addresses(&conn, &profile_id)?;
                // Same expiry math as `read_renewals::compute_renewals`: network
                // renewal window + the best persisted height estimate (no live
                // node here by definition of this branch). Reused, not
                // duplicated — both read the same helpers.
                let network = queries::get_wallet_profile(&conn, &profile_id)?
                    .and_then(|p| Network::from_str_opt(&p.network))
                    .unwrap_or_default();
                let renewal_window = network.name_params().renewal_window as i64;
                let current_height =
                    crate::commands::read::estimate_persisted_height(&conn, &profile_id)?;
                (tracked, action_ctx, addrs, renewal_window, current_height)
            };
            let tracked = match tracked {
                Some(t) => t,
                // Genuinely unknown locally — the true "nothing to go on" case.
                None => return Ok(conservative_capabilities(&name, "node unreachable")),
            };
            let phase = tracked
                .state
                .as_deref()
                .map(|s| s.to_uppercase())
                .unwrap_or_default();
            // Owned per explorer evidence: the recorded owner address is ours.
            let explorer_owned = tracked
                .owner_address
                .as_deref()
                .map(|a| profile_addrs.iter().any(|p| p == a))
                .unwrap_or(false);
            let owns_name = action_ctx.has_owner_coin || explorer_owned;
            // No node-synced owner coin → nothing may build a spend.
            let spend_locked = !action_ctx.has_owner_coin;
            // Days-until-expire from tracked chain evidence, so the modal's
            // `expiringSoon` matches the WalletView banner / Renewals screen
            // instead of staying silent for lack of live node stats.
            let days_until_expire = tracked.renewal_height.and_then(|renewal| {
                current_height.map(|h| {
                    let expires_at = renewal + renewal_window;
                    (expires_at - h) as f64 / crate::noncustodial::network::BLOCKS_PER_DAY
                })
            });
            Ok(build_name_action_capabilities(
                name,
                phase.clone(),
                &phase,
                None,
                &action_ctx,
                owns_name,
                spend_locked,
                days_until_expire,
            ))
        }
    }
}

/// Derive the full capability matrix from a phase + wallet evidence.
///
/// `spend_locked` forces every spend-capable action (register/update/transfer/
/// finalize/cancel/renew/revoke) to `allowed: false` with
/// [`OWNER_COIN_NOT_SYNCED_REASON`], regardless of what its own logic would
/// compute. This is the single mechanism that preserves the invariant "no spend
/// without a real local owner coin" on the node-unreachable, explorer-owned path
/// (where `owns_name` is true but `has_owner_coin` is false). On the node path
/// it is always `false`, leaving that branch's behavior unchanged.
fn build_name_action_capabilities(
    name: String,
    phase: String,
    raw_phase: &str,
    stats: Option<&serde_json::Value>,
    action_ctx: &NameActionContext,
    owns_name: bool,
    spend_locked: bool,
    days_until_expire_override: Option<f64>,
) -> NameActionCapabilities {
    // 4. Derive capabilities with strengthened validation rules.
    let can_open = NameActionCapability {
        allowed: phase == "AVAILABLE" || phase.is_empty(),
        reason: if phase != "AVAILABLE" && !phase.is_empty() {
            Some(format!("name is in phase '{phase}', not AVAILABLE"))
        } else {
            None
        },
    };

    let is_bidding_compatible = phase == "BIDDING" || phase == "OPENING";
    // Product rule: only allow bidding if wallet has no existing bid commitment
    // for this name (single-bid-per-wallet-per-name).
    let bid_allowed = is_bidding_compatible && action_ctx.existing_bid_count == 0;
    let can_bid = NameActionCapability {
        allowed: bid_allowed,
        reason: if !is_bidding_compatible {
            Some(format!("bidding is not open (phase: '{phase}')"))
        } else if action_ctx.existing_bid_count > 0 {
            Some("you already have a bid commitment for this name (one bid per wallet per name)".into())
        } else {
            None
        },
    };

    // Part 3 (Task 2 review, confirmed pre-existing bug): revealing SPENDS
    // the COV_BID coin, so `has_bid_coin` — not `has_reveal_coin` (which only
    // exists AFTER a reveal) — is what must gate this. Gating on
    // `has_reveal_coin` here made the Reveal button permanently disabled for
    // a perfectly healthy unrevealed bid.
    let can_reveal = NameActionCapability {
        allowed: phase == "REVEAL" && action_ctx.has_bid_commitment && action_ctx.has_bid_coin,
        reason: if phase != "REVEAL" {
            Some(format!("reveal phase not active (phase: '{phase}')"))
        } else if !action_ctx.has_bid_commitment {
            Some("no bid commitment found for this name".into())
        } else if !action_ctx.has_bid_coin {
            Some("no unspent bid coin found (sync first?)".into())
        } else {
            None
        },
    };

    let can_redeem = NameActionCapability {
        allowed: phase == "CLOSED" && action_ctx.has_reveal_coin && !owns_name,
        reason: if phase != "CLOSED" {
            Some(format!("auction not yet closed (phase: '{phase}')"))
        } else if !action_ctx.has_reveal_coin {
            Some("no unspent reveal coin to redeem".into())
        } else {
            Some("you won this auction (redeem not applicable)".into())
        },
    };

    // Register requires: CLOSED phase + wallet owns the name coin +
    // the covenant type is below REGISTER (i.e. not already registered).
    let registration_needed = phase == "CLOSED" && action_ctx.has_owner_coin
        && action_ctx.owner_covenant_type.map(|t| t < COV_REGISTER as i64).unwrap_or(true);
    let can_register = NameActionCapability {
        allowed: registration_needed,
        reason: if phase != "CLOSED" {
            Some(format!("auction not yet closed (phase: '{phase}')"))
        } else if !action_ctx.has_owner_coin {
            Some("wallet does not own the winning name coin".into())
        } else if action_ctx.owner_covenant_type.map(|t| t >= COV_REGISTER as i64).unwrap_or(false) {
            Some("name is already registered".into())
        } else {
            None
        },
    };

    let can_update = NameActionCapability {
        allowed: owns_name,
        reason: if !owns_name {
            Some("wallet does not control this name".into())
        } else {
            None
        },
    };

    let can_transfer = NameActionCapability {
        allowed: owns_name,
        reason: if !owns_name {
            Some("wallet does not control this name".into())
        } else {
            None
        },
    };

    let can_finalize = NameActionCapability {
        allowed: owns_name && action_ctx.transfer_has_items.unwrap_or(false),
        reason: if !owns_name {
            Some("wallet does not control this name".into())
        } else if !action_ctx.transfer_has_items.unwrap_or(false) {
            Some("name is not in TRANSFER state".into())
        } else {
            None
        },
    };

    let can_cancel_transfer = NameActionCapability {
        allowed: owns_name,
        reason: if !owns_name {
            Some("wallet does not control this name".into())
        } else {
            None
        },
    };

    let can_renew = NameActionCapability {
        allowed: owns_name,
        reason: if !owns_name {
            Some("wallet does not control this name".into())
        } else {
            None
        },
    };

    let can_revoke = NameActionCapability {
        allowed: owns_name,
        reason: if !owns_name {
            Some("wallet does not control this name".into())
        } else {
            None
        },
    };

    // 4b. Spend lock: when we can't build a spend (no node-synced owner coin),
    // force every spend-capable action to disallowed with a clear "not synced"
    // reason — regardless of what the ownership-based logic above computed. This
    // is what keeps an explorer-owned-but-unsynced name from offering actions it
    // can't actually perform, without duplicating the check in each branch.
    let (can_register, can_update, can_transfer, can_finalize, can_cancel_transfer, can_renew, can_revoke) =
        if spend_locked {
            let locked = || NameActionCapability {
                allowed: false,
                reason: Some(OWNER_COIN_NOT_SYNCED_REASON.to_string()),
            };
            (locked(), locked(), locked(), locked(), locked(), locked(), locked())
        } else {
            (can_register, can_update, can_transfer, can_finalize, can_cancel_transfer, can_renew, can_revoke)
        };

    // 5. Derive task state. Days-until-expire comes from the node/explorer
    // stats when present (`daysUntilExpire`, falling back to
    // `blocksUntilExpire` at ~10-minute blocks) so an owned name inside the
    // renewal threshold surfaces as `expiringSoon`. On the node-unreachable
    // fallback path there are no live `stats` (always `None` there) — the
    // caller instead computes it from tracked chain evidence the same way
    // `read_renewals` does and passes it as `days_until_expire_override`, so
    // the modal doesn't contradict the WalletView/Renewals expiry alarm just
    // because the node isn't synced right now.
    let days_until_expire = days_until_expire_override.or_else(|| {
        stats.and_then(|s| {
            s.get("daysUntilExpire").and_then(|v| v.as_f64()).or_else(|| {
                s.get("blocksUntilExpire")
                    .and_then(|v| v.as_i64())
                    .map(|b| b as f64 / crate::noncustodial::network::BLOCKS_PER_DAY)
            })
        })
    });
    let task_state = derive_auction_task_state(&phase, owns_name, action_ctx.has_bid_commitment, action_ctx.has_bid_coin, action_ctx.has_reveal_coin, action_ctx.has_owner_coin, action_ctx.owner_covenant_type, days_until_expire);

    // 6. Determine next action.
    let (next_action_key, next_action_label, next_action_reason) = next_action_for_task(&task_state);

    // 7. Extract countdown from stats.
    let (countdown_label, countdown_blocks, countdown_hours) = extract_countdown(raw_phase, stats);

    NameActionCapabilities {
        name,
        phase,
        task_state,
        owns_name,
        has_bid_commitment: action_ctx.has_bid_commitment,
        has_bid_coin: action_ctx.has_bid_coin,
        has_reveal_coin: action_ctx.has_reveal_coin,
        has_owner_coin: action_ctx.has_owner_coin,
        can_open,
        can_bid,
        can_reveal,
        can_redeem,
        can_register,
        can_update,
        can_transfer,
        can_finalize,
        can_cancel_transfer,
        can_renew,
        can_revoke,
        next_action_key,
        next_action_label,
        next_action_reason,
        countdown_label,
        countdown_blocks,
        countdown_hours,
    }
}

/// Conservative fallback when the node is unreachable.
fn conservative_capabilities(name: &str, reason: &str) -> NameActionCapabilities {
    let disallowed = NameActionCapability {
        allowed: false,
        reason: Some(reason.into()),
    };
    NameActionCapabilities {
        name: name.into(),
        phase: "UNKNOWN".into(),
        task_state: AuctionTaskState::UnavailableOther,
        owns_name: false,
        has_bid_commitment: false,
        has_bid_coin: false,
        has_reveal_coin: false,
        has_owner_coin: false,
        can_open: disallowed.clone(),
        can_bid: disallowed.clone(),
        can_reveal: disallowed.clone(),
        can_redeem: disallowed.clone(),
        can_register: disallowed.clone(),
        can_update: disallowed.clone(),
        can_transfer: disallowed.clone(),
        can_finalize: disallowed.clone(),
        can_cancel_transfer: disallowed.clone(),
        can_renew: disallowed.clone(),
        can_revoke: disallowed.clone(),
        next_action_key: None,
        next_action_label: None,
        next_action_reason: Some(reason.into()),
        countdown_label: None,
        countdown_blocks: None,
        countdown_hours: None,
    }
}

/// Derive a user-facing task state from the raw phase + wallet evidence.
///
/// `owner_covenant_type` is the covenant type of the wallet's owner coin for
/// this name, if available. `None` means the wallet does not hold the coin.
/// Values >= COV_REGISTER (6) indicate the name is already registered; lower
/// values (e.g. COV_REVEAL=4 or COV_OPEN=2) mean the wallet just won the
/// auction but has not yet registered.
///
/// `days_until_expire` (when known) turns a calm "Owned" into
/// [`AuctionTaskState::ExpiringSoon`] once it drops to
/// [`EXPIRING_SOON_THRESHOLD_DAYS`] or below (including negative values —
/// per our data the window already lapsed, which is MORE urgent, not less).
/// `None` means "unknown", which never fires the alarm.
///
/// `has_bid_coin` (unspent COV_BID) vs `has_reveal_coin` (unspent COV_REVEAL,
/// Part 3 / Task 6): a REVEAL spends the former and creates the latter, so
/// only `has_bid_coin` is meaningful for the REVEAL-phase branch below;
/// `has_reveal_coin` remains meaningful only for the post-auction CLOSED
/// branch (a losing reveal coin waiting to be redeemed).
#[allow(clippy::too_many_arguments)]
pub fn derive_auction_task_state(
    phase: &str,
    owns_name: bool,
    has_bid_commitment: bool,
    has_bid_coin: bool,
    has_reveal_coin: bool,
    has_owner_coin: bool,
    owner_covenant_type: Option<i64>,
    days_until_expire: Option<f64>,
) -> AuctionTaskState {
    let expiring_soon = days_until_expire
        .map(|d| d <= EXPIRING_SOON_THRESHOLD_DAYS)
        .unwrap_or(false);
    match phase {
        "AVAILABLE" | "" => AuctionTaskState::AvailableToOpen,
        "OPENING" => AuctionTaskState::WaitingForBidding,
        "BIDDING" => {
            if has_bid_commitment {
                AuctionTaskState::WaitingForBidding
            } else {
                AuctionTaskState::ReadyToBid
            }
        }
        "REVEAL" => {
            if has_bid_commitment && has_bid_coin {
                AuctionTaskState::ReadyToReveal
            } else if has_bid_commitment {
                // Has a bid but the bid coin isn't synced locally yet (sync
                // may be pending) — still prompt the user to reveal rather
                // than staying silent; `can_reveal.allowed` (which DOES
                // require `has_bid_coin`) is the actual button gate.
                AuctionTaskState::ReadyToReveal
            } else {
                AuctionTaskState::UnavailableOther
            }
        }
        "CLOSED" => {
            if owns_name && has_owner_coin {
                // If the owner coin is already REGISTER (6) or higher (UPDATE,
                // RENEW, TRANSFER, etc.), the name is already registered — no
                // REGISTER action needed. A coin with covenant type < COV_REGISTER
                // (e.g. OPEN=2, REVEAL=4) means the wallet just won but has not
                // yet registered.
                let already_registered = owner_covenant_type
                    .map(|t| t >= COV_REGISTER as i64)
                    .unwrap_or(false);
                if already_registered {
                    if expiring_soon {
                        AuctionTaskState::ExpiringSoon
                    } else {
                        AuctionTaskState::OwnedNoUrgentAction
                    }
                } else {
                    // Registration takes precedence over the renewal alarm:
                    // an unregistered win can't be renewed yet.
                    AuctionTaskState::WonNeedsRegister
                }
            } else if owns_name {
                // Owned per explorer evidence but the owner coin isn't synced
                // locally (node-unreachable path): treat as owned — and still
                // raise the renewal alarm when the expiry data says so, since
                // staying silent here is exactly how a name gets lost.
                if expiring_soon {
                    AuctionTaskState::ExpiringSoon
                } else {
                    AuctionTaskState::OwnedNoUrgentAction
                }
            } else if has_reveal_coin {
                AuctionTaskState::LostNeedsRedeem
            } else {
                AuctionTaskState::OwnedNoUrgentAction
            }
        }
        "TRANSFER" => AuctionTaskState::TransferPendingFinalize,
        "REVOKED" => AuctionTaskState::UnavailableOther,
        _ => {
            if owns_name {
                AuctionTaskState::OwnedNoUrgentAction
            } else {
                AuctionTaskState::UnavailableOther
            }
        }
    }
}

/// Map task state to the recommended next action.
pub(crate) fn next_action_for_task(
    task: &AuctionTaskState,
) -> (Option<String>, Option<String>, Option<String>) {
    match task {
        AuctionTaskState::AvailableToOpen => {
            (Some("OPEN".into()), Some("Open Auction".into()), Some("Start the auction for this name.".into()))
        }
        AuctionTaskState::WaitingForBidding => {
            (Some("WAIT".into()), Some("Wait for Bidding".into()), Some("The auction opens for bidding soon.".into()))
        }
        AuctionTaskState::ReadyToBid => {
            (Some("BID".into()), Some("Place Bid".into()), Some("Place a blind bid before the bidding period ends.".into()))
        }
        AuctionTaskState::ReadyToReveal => {
            (Some("REVEAL".into()), Some("Reveal Bid".into()), Some("Reveal your bid now, or your lockup can't be reclaimed.".into()))
        }
        AuctionTaskState::WonNeedsRegister => {
            (Some("REGISTER".into()), Some("Register Name".into()), Some("You won the auction! Register the name to finalize ownership.".into()))
        }
        AuctionTaskState::LostNeedsRedeem => {
            (Some("REDEEM".into()), Some("Redeem Lockup".into()), Some("Your bid lost. Redeem your reveal coin to reclaim the funds.".into()))
        }
        AuctionTaskState::TransferPendingFinalize => {
            (Some("FINALIZE".into()), Some("Finalize Transfer".into()), Some("This name is being transferred. Finalize to complete the transfer.".into()))
        }
        AuctionTaskState::OwnedNoUrgentAction => {
            (Some("MANAGE".into()), Some("Manage Name".into()), Some("You own this name. Manage DNS, renew, or transfer.".into()))
        }
        AuctionTaskState::ExpiringSoon => {
            (Some("RENEW".into()), Some("Renew Name".into()), Some("This name is close to expiry. Renew now — an expired Handshake name is lost forever.".into()))
        }
        AuctionTaskState::UnavailableOther => (None, None, None),
    }
}

/// Extract countdown data from the node's stats object.
fn extract_countdown(
    phase: &str,
    stats: Option<&serde_json::Value>,
) -> (Option<String>, Option<i64>, Option<f64>) {
    let stats = match stats {
        Some(s) => s,
        None => return (None, None, None),
    };

    match phase {
        "OPENING" => {
            let blocks = stats.get("blocksUntilBidding").and_then(|b| b.as_i64());
            let hours = stats.get("hoursUntilBidding").and_then(|h| h.as_f64());
            (blocks.map(|_| "Bidding opens in".into()), blocks, hours)
        }
        "BIDDING" => {
            let blocks = stats.get("blocksUntilReveal").and_then(|b| b.as_i64());
            let hours = stats.get("hoursUntilReveal").and_then(|h| h.as_f64());
            (blocks.map(|_| "Reveal starts in".into()), blocks, hours)
        }
        "REVEAL" => {
            let blocks = stats.get("blocksUntilClose").and_then(|b| b.as_i64());
            let hours = stats.get("hoursUntilClose").and_then(|h| h.as_f64());
            (blocks.map(|_| "Auction closes in".into()), blocks, hours)
        }
        "CLOSED" => {
            let blocks = stats.get("blocksUntilExpire").and_then(|b| b.as_i64());
            (blocks.map(|_| "Expires in".into()), blocks, None)
        }
        _ => (None, None, None),
    }
}

// ---------------------------------------------------------------------------
// Original covenant build commands follow
// ---------------------------------------------------------------------------

// --- OPEN ------------------------------------------------------------------

#[tauri::command]
pub async fn build_open_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let raw = names::raw_name(&name)?;
    // OPEN output goes to the next unused wallet receive address (value 0).
    let recv = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        crate::noncustodial::derivation::next_unused_receive_address(
            &conn,
            &ctx.profile_id,
            ctx.account,
            ctx.network,
            &ctx.account_xpub,
        )?
    };
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        None,
        PrimaryOutput { value: 0, address: recv.address, covenant: covenants::open(&nh, &raw) },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "open", &name, None, &res)
}

// --- BID -------------------------------------------------------------------

#[tauri::command]
pub async fn build_bid_draft(
    state: State<'_, AppState>,
    name: String,
    bid_value: i64,
    lockup: i64,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if bid_value <= 0 || lockup < bid_value {
        return Err(AppError::InvalidInput("lockup must be >= bid value > 0".into()));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let nh_hex = hex::encode(nh);
    let raw = names::raw_name(&name)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    // Bid output goes to the NEXT UNUSED wallet receive address. Rotation keeps
    // every bid on its own address; reveal/redeem additionally match the coin by
    // name hash, which is what keeps legacy bids (all on receive[0]) revealable.
    let bid_addr = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        crate::noncustodial::derivation::next_unused_receive_address(
            &conn,
            &ctx.profile_id,
            ctx.account,
            ctx.network,
            &ctx.account_xpub,
        )?
    };
    let (_v, program) = address::decode(ctx.network, &bid_addr.address)?;
    let mut addr_hash = [0u8; 20];
    if program.len() != 20 {
        return Err(AppError::InvalidInput("bid address is not p2wpkh".into()));
    }
    addr_hash.copy_from_slice(&program);

    let nonce = bids::compute_nonce(&ctx.account_xpub, &nh, &addr_hash, bid_value as u64)?;
    let blind = bids::compute_blind(bid_value as u64, &nonce);
    let cov = covenants::bid(&nh, ns.height, &raw, &blind);

    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        None,
        PrimaryOutput { value: lockup as u64, address: bid_addr.address.clone(), covenant: cov },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;

    // --- Single critical section: bid-multiplicity guard (I2) + commitment
    // persist + draft insert/reservation, ALL under one held MutexGuard.
    //
    // Product rule: one bid per wallet per name. The UI already gates this
    // (`build_name_action_capabilities` / `existing_bid_count`), but that's
    // advisory only — a second window, a stale UI, or a replayed call can
    // still reach this command directly, so the rule must be enforced here
    // too. Two checks, either of which blocks a second bid:
    //   (a) an unspent COV_BID coin for this name anywhere in the profile —
    //       a bid is already live on-chain (or awaiting confirmation);
    //   (b) a not-yet-terminal `bid` draft for this name — one is already
    //       queued/signed/broadcast and might still land.
    //
    // Both checks AND every write below (commitment insert, reveal-end-height
    // stamp, draft insert + coin reservation) share ONE MutexGuard — no
    // unlock/relock in between. Without that, two concurrent calls could both
    // pass the checks before either had written anything (classic TOCTOU);
    // `state.db` is a plain (non-reentrant) `std::sync::Mutex`, so holding it
    // across `persist_with_conn` (rather than calling `persist`, which would
    // try to lock it again and deadlock) is what makes this section atomic —
    // the second caller simply blocks on `lock()` until the first is done,
    // then sees the first's writes and is rejected by the same checks.
    //
    // This mostly SUBSUMES the `insert_bid_commitment` ON CONFLICT fix
    // (I2 part 2, defense-in-depth in `queries::insert_bid_commitment`): with
    // this guard in place, a second bid on the same name is rejected here,
    // before a conflicting commitment row could ever be attempted. The
    // ON CONFLICT fix still matters as a second line of defense — e.g. if
    // this guard's on-chain/draft evidence is somehow stale — a same-value
    // re-bid must error instead of silently dropping its commitment row.
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;

    let existing_bid_coins = queries::find_unspent_covenant_utxos_by_name_hash(
        &conn,
        &ctx.profile_id,
        sync::COV_BID as i64,
        &nh_hex,
    )?;
    if !existing_bid_coins.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "wallet already has an unspent bid for '{name}' — one bid per wallet per name"
        )));
    }
    if queries::has_pending_bid_draft_for_name(&conn, &ctx.profile_id, &name)? {
        return Err(AppError::InvalidInput(format!(
            "a bid draft for '{name}' is already pending — one bid per wallet per name"
        )));
    }

    // Persist the bid commitment (secret nonce/blind) before building the
    // draft. If this fails — including the honest ON CONFLICT error above —
    // the function returns here and NO draft is ever persisted; a bid whose
    // commitment can't be trusted must never reach the chain.
    queries::insert_bid_commitment(
        &conn,
        &ctx.profile_id,
        &name,
        &nh_hex,
        &bid_addr.address,
        bid_addr.branch as i64,
        bid_addr.child_index as i64,
        bid_value,
        lockup,
        &hex::encode(nonce),
        &hex::encode(blind),
    )?;
    // Estimate when the reveal window closes for the deadline scanner
    // (I1): the BID covenant's `start` item IS `ns.height` (the auction's
    // OPEN height). From there hsd's timeline is: OPENING for
    // `treeInterval + 1` blocks (name.js `isBidding`/`isOpening` — the
    // name only *enters* BIDDING once `height > start + treeInterval`),
    // THEN `biddingPeriod` blocks of BIDDING, THEN `revealPeriod` blocks
    // of REVEAL. So reveal_period_end = start + (treeInterval + 1) +
    // biddingPeriod + revealPeriod (network consensus params). Only
    // derivable here, where the live auction-start height is known;
    // `recover_bid_commitment` has no such height to work from and
    // leaves this NULL.
    let params = ctx.network.name_params();
    let reveal_end_height = ns.height as i64
        + (params.tree_interval as i64 + 1)
        + params.bidding_period as i64
        + params.reveal_period as i64;
    queries::set_reveal_end_height(&conn, &ctx.profile_id, &hex::encode(blind), reveal_end_height)?;

    persist_with_conn(&conn, &ctx.profile_id, "bid", &name, None, &res)
}

// --- REVEAL ----------------------------------------------------------------

#[tauri::command]
pub async fn build_reveal_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    // Look up our bid commitment + the unspent BID coin at that address.
    let (bid, bid_coin) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let bid = queries::get_bid_commitment(&conn, &ctx.profile_id, &name)?
            .ok_or_else(|| AppError::NotFound(format!("no bid commitment for '{name}'")))?;
        let coin = queries::find_unspent_covenant_utxo(
            &conn,
            &ctx.profile_id,
            &bid.address,
            sync::COV_BID as i64,
            &name,
            &hex::encode(nh),
        )?
        .ok_or_else(|| {
            AppError::NotFound(format!("no unspent bid coin for '{name}' (sync first?)"))
        })?;
        (bid, coin)
    };
    let mut nonce = [0u8; 32];
    let nb = hex::decode(&bid.nonce_hex).map_err(|e| AppError::Crypto(format!("nonce: {e}")))?;
    if nb.len() != 32 {
        return Err(AppError::Crypto("stored nonce not 32 bytes".into()));
    }
    nonce.copy_from_slice(&nb);

    let cov = covenants::reveal(&nh, ns.height, &nonce);
    // Reveal output value = the true bid value; output address = the bid coin's
    // address. The lockup − bid difference returns as change automatically.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(bid_coin.clone())),
        PrimaryOutput {
            value: bid.bid_value_doos as u64,
            address: bid_coin.address.clone(),
            covenant: cov,
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "reveal", &name, None, &res)
}

// --- REDEEM ----------------------------------------------------------------

#[tauri::command]
pub async fn build_redeem_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    let coin = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let bid = queries::get_bid_commitment(&conn, &ctx.profile_id, &name)?
            .ok_or_else(|| AppError::NotFound(format!("no bid for '{name}'")))?;
        queries::find_unspent_covenant_utxo(
            &conn,
            &ctx.profile_id,
            &bid.address,
            sync::COV_REVEAL as i64,
            &name,
            &hex::encode(nh),
        )?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "no unspent losing reveal coin for '{name}' (sync first?)"
            ))
        })?
    };
    // REDEEM reclaims the reveal output value back to the wallet.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::redeem(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "redeem", &name, None, &res)
}

// --- owner actions (spend the name's owner UTXO) ---------------------------

/// Common loader: fetch our owner coin + current name state.
async fn owner_coin_and_state(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    name: &str,
) -> Result<(queries::NameCoin, NameState), AppError> {
    let coin = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::get_name_coin(&conn, &ctx.profile_id, name)?
            .ok_or_else(|| AppError::NotFound(format!("wallet does not hold '{name}' (sync?)")))?
    };
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, name).await?;
    Ok((coin, ns))
}

#[tauri::command]
pub async fn build_register_draft(
    state: State<'_, AppState>,
    name: String,
    records: Option<Vec<serde_json::Value>>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let res_bytes = match &records {
        Some(r) if !r.is_empty() => resource::encode(r)?,
        _ => Vec::new(), // EMPTY resource
    };
    // REGISTER locks `ns.value` (the price); the rest returns as change.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: ns.value,
            address: coin.address.clone(),
            covenant: covenants::register(&nh, ns.height, &res_bytes, &rblock),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "register", &name, None, &res)
}

#[tauri::command]
pub async fn build_update_draft(
    state: State<'_, AppState>,
    name: String,
    records: Vec<serde_json::Value>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let res_bytes = resource::encode(&records)?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::update(&nh, ns.height, &res_bytes),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "update", &name, None, &res)
}

#[tauri::command]
pub async fn build_renew_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::renew(&nh, ns.height, &rblock),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "renew", &name, None, &res)
}

#[tauri::command]
pub async fn build_transfer_draft(
    state: State<'_, AppState>,
    name: String,
    recipient: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let (version, program) = address::decode(ctx.network, &recipient)?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::transfer(&nh, ns.height, version, &program),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "transfer", &name, Some(&recipient), &res)
}

#[tauri::command]
pub async fn build_finalize_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let raw = names::raw_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;

    // The finalize output goes to the TRANSFER target recorded on the owner
    // coin's covenant: items = [nameHash, height, version(u8), addrHash].
    let cov_json = coin
        .covenant_json
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("name is not in transfer; nothing to finalize".into()))?;
    let cov: serde_json::Value = serde_json::from_str(cov_json)?;
    let items = cov.get("items").and_then(|i| i.as_array())
        .ok_or_else(|| AppError::InvalidInput("owner coin has no covenant items".into()))?;
    if items.len() < 4 {
        return Err(AppError::InvalidInput("owner coin is not a TRANSFER".into()));
    }
    let ver_hex = items[2].as_str().unwrap_or("00");
    let hash_hex = items[3].as_str().unwrap_or("");
    let version = u8::from_str_radix(ver_hex, 16).unwrap_or(0);
    let target_hash = hex::decode(hash_hex)
        .map_err(|e| AppError::InvalidInput(format!("bad transfer target: {e}")))?;
    if version != 0 || target_hash.len() != 20 {
        return Err(AppError::InvalidInput("finalize target must be p2wpkh".into()));
    }
    let mut h160 = [0u8; 20];
    h160.copy_from_slice(&target_hash);
    let target_address = address::encode_p2wpkh(ctx.network, &h160)?;

    let flags: u8 = if ns.weak { 1 } else { 0 };
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: target_address.clone(),
            covenant: covenants::finalize(&nh, ns.height, &raw, flags, ns.claimed, ns.renewals, &rblock),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "finalize", &name, Some(&target_address), &res)
}

#[tauri::command]
pub async fn build_cancel_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::cancel(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "cancel", &name, None, &res)
}

#[tauri::command]
pub async fn build_revoke_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::revoke(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "revoke", &name, None, &res)
}
