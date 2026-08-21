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
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::noncustodial::send::{self, SpendableCoin};
use crate::noncustodial::sync::{self, COV_REGISTER, COV_REVEAL};
use crate::noncustodial::tx::sighash;
use crate::noncustodial::types::TxDraftSummary;
use crate::noncustodial::{address, bids, covenants, names, resource};
use crate::AppState;

use super::names_pure;

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
        return Err(AppError::InvalidInput(
            "active profile is watch-only".into(),
        ));
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
    /// On-chain auction phase (e.g. "BIDDING", "OPENING", "REVEAL", "CLOSED").
    /// Populated from `getnameinfo.info.state`. Empty string when the node
    /// returns null / no state field. Uppercased for case-insensitive matching
    /// against consensus phase strings.
    pub(crate) phase: String,
}

pub(crate) async fn fetch_name_state(
    client: &dyn NodeRpc,
    name: &str,
) -> Result<NameState, AppError> {
    let v = client.get_name_info(name).await?;
    let info = v.get("info");
    let info = match info {
        Some(i) if !i.is_null() => i,
        _ => {
            return Err(AppError::InvalidInput(format!(
                "name '{name}' has no on-chain state"
            )))
        }
    };
    let geti = |k: &str| info.get(k).and_then(|x| x.as_i64());
    Ok(NameState {
        height: geti("height").unwrap_or(0) as u32,
        value: geti("value").unwrap_or(0) as u64,
        renewals: geti("renewals").unwrap_or(0) as u32,
        claimed: geti("claimed").unwrap_or(0) as u32,
        weak: info.get("weak").and_then(|x| x.as_bool()).unwrap_or(false),
        phase: info
            .get("state")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_uppercase(),
    })
}

/// `getRenewalBlock`: internal-order 32-byte hash at `height - 2*renewalMaturity`.
pub(crate) async fn renewal_block(
    client: &dyn NodeRpc,
    network: Network,
) -> Result<[u8; 32], AppError> {
    let tip = client.get_blockchain_info().await?.blocks;
    let maturity = network.name_params().renewal_maturity as i64;
    let height = (tip - 2 * maturity).max(0);
    let hash_hex = client.get_block_hash(height).await?;
    let bytes =
        hex::decode(&hash_hex).map_err(|e| AppError::Rpc(format!("bad block hash: {e}")))?;
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
    /// Full list of names when this draft covers more than one (batch-bid,
    /// batch-renew, etc.). Serialized as `nameList` in JSON so
    /// `has_pending_*_draft_for_name` queries can enumerate the batch's
    /// members and match any of them. `None` for single-name drafts keeps
    /// their `summary_json` byte-identical to pre-batch-bid history.
    #[serde(skip_serializing_if = "Option::is_none")]
    name_list: Option<&'a [&'a str]>,
}

/// Persist a planned covenant draft and return its summary.
fn persist(
    state: &State<'_, AppState>,
    profile_id: &str,
    action: &str,
    name: &str,
    recipient: Option<&str>,
    name_list: Option<&[&str]>,
    res: &actions::PlanResult,
) -> Result<TxDraftSummary, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    persist_with_conn(&conn, profile_id, action, name, recipient, name_list, res)
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
    name_list: Option<&[&str]>,
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
        name_list,
    };
    let id = random_id();
    // Reserve every input the plan spends (I3): the funding coins AND, when
    // present, the name UTXO itself — two covenant drafts must not be able to
    // grab the same name coin (e.g. two REVEALs) any more than two plain
    // sends can grab the same liquid coin.
    let reserved_inputs: Vec<(String, u32)> = res
        .plan
        .inputs
        .iter()
        .map(|i| (i.txid.clone(), i.vout))
        .collect();
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
    /// The wallet has broadcast its reveal, but the tx has not yet confirmed.
    /// No user action is needed — it's a "your reveal is in flight" state that
    /// keeps the auctions row from re-inviting another Reveal click.
    #[serde(rename = "revealBroadcastPending")]
    RevealBroadcastPending,
    /// The wallet's reveal has confirmed on-chain, but the auction is still in
    /// its REVEAL window (other bidders may still reveal). Nothing to do but
    /// wait for the phase to close into won/lost.
    #[serde(rename = "revealDoneWaitingForClose")]
    RevealDoneWaitingForClose,
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
    /// The txid of the wallet's reveal broadcast (if any). Surfaced so the
    /// frontend can render an explorer link + copy button on the pending/done
    /// card without re-deriving from raw data.
    pub reveal_txid: Option<String>,
    /// The wallet's true bid value (doos) from the local commitment row.
    /// Surfaced so the confirm-before-broadcast panel can show the amount.
    pub bid_value_doos: Option<i64>,
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
    /// Task 1: an OPEN for this name is already pending — either an unspent
    /// COV_OPEN coin (broadcast, awaiting confirmation) or a not-yet-terminal
    /// `open` draft (queued/signed/broadcast_pending/broadcasted). Mirrors the
    /// exact two checks `build_open_draft`'s own double-open guard enforces,
    /// so `can_open` and the task state reflect the same rule the backend
    /// gate applies. Only catches OUR OWN pending open — someone ELSE having
    /// already opened the name is a phase change (AVAILABLE -> OPENING),
    /// which `can_open`'s phase check already handles separately.
    pub has_pending_open: bool,
    /// The `reveal_txid` stamped on the bid commitment row (if any).
    pub reveal_txid: Option<String>,
    /// Status of the local tx_draft matching `reveal_txid` (if one exists).
    /// `None` means either no reveal_txid or no local draft for it (cross-
    /// device / chain-scan stamped).
    pub reveal_draft_status: Option<String>,
    /// The wallet's true bid value (doos) from the commitment row.
    pub bid_value_doos: Option<i64>,
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

    // Task 1: pending-OPEN evidence, mirroring the two checks
    // `build_open_draft`'s guard enforces — (a) an unspent COV_OPEN coin for
    // this name anywhere in the profile, OR (b) a not-yet-terminal `open`
    // draft. Either makes `can_open` reflect the pending state instead of
    // staying "allowed" until the user hits the guard directly.
    let has_pending_open_coin = names::hash_name(name)
        .ok()
        .map(hex::encode)
        .map(|nh_hex| {
            queries::find_unspent_covenant_utxos_by_name_hash(
                conn,
                profile_id,
                sync::COV_OPEN as i64,
                &nh_hex,
            )
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        })
        .unwrap_or(false);
    let has_pending_open_draft =
        queries::has_pending_draft_for_name(conn, profile_id, "open", name).unwrap_or(false);
    let has_pending_open = has_pending_open_coin || has_pending_open_draft;

    // Reveal-in-flight evidence: the commitment row's `reveal_txid` (stamped
    // either by our own broadcast in `build_reveal_draft`, or by `chain_scan`
    // observing a matching on-chain reveal). When we have a local draft for it,
    // its status distinguishes broadcasted (pending) from confirmed from
    // dropped/failed; when there's no draft (restored/cross-device wallet), the
    // caller falls back to the bid-coin-spent chain fact.
    let reveal_txid = bid.as_ref().and_then(|b| b.reveal_txid.clone());
    let reveal_draft_status = reveal_txid.as_ref().and_then(|txid| {
        queries::get_draft_status_by_txid(conn, profile_id, txid)
            .ok()
            .flatten()
    });
    let bid_value_doos = bid.as_ref().map(|b| b.bid_value_doos);

    Ok(NameActionContext {
        has_bid_commitment: bid.is_some(),
        has_bid_coin: bid_coin.is_some(),
        has_reveal_coin: reveal_coin.is_some(),
        has_owner_coin: owner_coin.is_some(),
        owner_covenant_type: owner_cov_type,
        name_height: nh,
        transfer_has_items: transfer,
        existing_bid_count,
        has_pending_open,
        reveal_txid,
        reveal_draft_status,
        bid_value_doos,
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
    let name_info = if crate::commands::read::is_node_ready_for_local_reads(state).await {
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
                let action_ctx = find_name_action_context(&conn, profile_id, &name)?;
                let tracked_owner_address =
                    queries::get_tracked_name_state(&conn, profile_id, &name)?
                        .and_then(|t| t.owner_address);
                let profile_addrs = queries::get_profile_addresses(&conn, profile_id)?;
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
                let tracked = queries::get_tracked_name_state(&conn, profile_id, &name)?;
                let action_ctx = find_name_action_context(&conn, profile_id, &name)?;
                let addrs = queries::get_profile_addresses(&conn, profile_id)?;
                // Same expiry math as `read_renewals::compute_renewals`: network
                // renewal window + the best persisted height estimate (no live
                // node here by definition of this branch). Reused, not
                // duplicated — both read the same helpers.
                let network = queries::get_wallet_profile(&conn, profile_id)?
                    .and_then(|p| Network::from_str_opt(&p.network))
                    .unwrap_or_default();
                let renewal_window = network.name_params().renewal_window as i64;
                let current_height =
                    crate::commands::read::estimate_persisted_height(&conn, profile_id)?;
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_name_action_capabilities(
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
    // Task 1: `can_open` also reflects a pending OPEN for this name (our own
    // unconfirmed OPEN coin or a not-yet-terminal `open` draft) — the same
    // rule `build_open_draft`'s guard enforces server-side, so the button
    // disables with a clear reason instead of staying enabled until the user
    // hits the guard directly.
    let can_open = NameActionCapability {
        allowed: (phase == "AVAILABLE" || phase.is_empty()) && !action_ctx.has_pending_open,
        reason: if phase != "AVAILABLE" && !phase.is_empty() {
            Some(format!("name is in phase '{phase}', not AVAILABLE"))
        } else if action_ctx.has_pending_open {
            Some("an auction is already opening for this name (pending confirmation)".into())
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
            Some(
                "you already have a bid commitment for this name (one bid per wallet per name)"
                    .into(),
            )
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
    let registration_needed = phase == "CLOSED"
        && action_ctx.has_owner_coin
        && action_ctx
            .owner_covenant_type
            .map(|t| t < COV_REGISTER as i64)
            .unwrap_or(true);
    let can_register = NameActionCapability {
        allowed: registration_needed,
        reason: if phase != "CLOSED" {
            Some(format!("auction not yet closed (phase: '{phase}')"))
        } else if !action_ctx.has_owner_coin {
            Some("wallet does not own the winning name coin".into())
        } else if action_ctx
            .owner_covenant_type
            .map(|t| t >= COV_REGISTER as i64)
            .unwrap_or(false)
        {
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
    let (
        can_register,
        can_update,
        can_transfer,
        can_finalize,
        can_cancel_transfer,
        can_renew,
        can_revoke,
    ) = if spend_locked {
        let locked = || NameActionCapability {
            allowed: false,
            reason: Some(OWNER_COIN_NOT_SYNCED_REASON.to_string()),
        };
        (
            locked(),
            locked(),
            locked(),
            locked(),
            locked(),
            locked(),
            locked(),
        )
    } else {
        (
            can_register,
            can_update,
            can_transfer,
            can_finalize,
            can_cancel_transfer,
            can_renew,
            can_revoke,
        )
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
            s.get("daysUntilExpire")
                .and_then(|v| v.as_f64())
                .or_else(|| {
                    s.get("blocksUntilExpire")
                        .and_then(|v| v.as_i64())
                        .map(|b| b as f64 / crate::noncustodial::network::BLOCKS_PER_DAY)
                })
        })
    });
    let task_state = derive_auction_task_state(
        &phase,
        owns_name,
        action_ctx.has_bid_commitment,
        action_ctx.has_bid_coin,
        action_ctx.has_reveal_coin,
        action_ctx.has_owner_coin,
        action_ctx.owner_covenant_type,
        days_until_expire,
        action_ctx.has_pending_open,
        action_ctx.reveal_txid.as_deref(),
        action_ctx.reveal_draft_status.as_deref(),
    );

    // 6. Determine next action.
    let (next_action_key, next_action_label, next_action_reason) =
        next_action_for_task(&task_state);

    // 7. Extract countdown from stats.
    let (countdown_label, countdown_blocks, countdown_hours) =
        names_pure::extract_countdown(raw_phase, stats);

    NameActionCapabilities {
        name,
        phase,
        task_state,
        owns_name,
        has_bid_commitment: action_ctx.has_bid_commitment,
        has_bid_coin: action_ctx.has_bid_coin,
        has_reveal_coin: action_ctx.has_reveal_coin,
        has_owner_coin: action_ctx.has_owner_coin,
        reveal_txid: action_ctx.reveal_txid.clone(),
        bid_value_doos: action_ctx.bid_value_doos,
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
pub(crate) fn conservative_capabilities(name: &str, reason: &str) -> NameActionCapabilities {
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
        reveal_txid: None,
        bid_value_doos: None,
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
///
/// `has_pending_open` (Task 1): when true and the phase hasn't reached
/// OPENING yet (still "AVAILABLE"/""), this returns [`AuctionTaskState::WaitingForBidding`]
/// instead of [`AuctionTaskState::AvailableToOpen`] — we reuse the existing
/// variant (its "Wait for Bidding" label reads fine for "your OPEN is
/// confirming") rather than adding a new one.
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
    has_pending_open: bool,
    reveal_txid: Option<&str>,
    reveal_draft_status: Option<&str>,
) -> AuctionTaskState {
    let expiring_soon = days_until_expire
        .map(|d| d <= EXPIRING_SOON_THRESHOLD_DAYS)
        .unwrap_or(false);
    match phase {
        "AVAILABLE" | "" => {
            if has_pending_open {
                AuctionTaskState::WaitingForBidding
            } else {
                AuctionTaskState::AvailableToOpen
            }
        }
        "OPENING" => AuctionTaskState::WaitingForBidding,
        "BIDDING" => {
            if has_bid_commitment {
                AuctionTaskState::WaitingForBidding
            } else {
                AuctionTaskState::ReadyToBid
            }
        }
        "REVEAL" => {
            if !has_bid_commitment {
                return AuctionTaskState::UnavailableOther;
            }
            // Reveal state machine (grilled design): prefer a local draft's
            // status, then fall back to on-chain facts.
            //  1. Local draft exists: broadcasted/broadcast_pending → pending;
            //     confirmed → done; dropped/failed → back to ReadyToReveal so
            //     the user can re-broadcast (the bid coin is still unspent).
            //  2. No draft but reveal_txid set AND the bid coin is spent
            //     (!has_bid_coin) → done (chain ground truth; covers restored /
            //     cross-device wallets that revealed elsewhere).
            //  3. Otherwise → ReadyToReveal (still prompt; `can_reveal.allowed`,
            //     which requires has_bid_coin, is the real button gate).
            match reveal_draft_status {
                Some("broadcasted") | Some("broadcast_pending") => {
                    AuctionTaskState::RevealBroadcastPending
                }
                Some("confirmed") => AuctionTaskState::RevealDoneWaitingForClose,
                _ => {
                    if reveal_txid.is_some() && !has_bid_coin {
                        AuctionTaskState::RevealDoneWaitingForClose
                    } else {
                        AuctionTaskState::ReadyToReveal
                    }
                }
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
        AuctionTaskState::RevealBroadcastPending => {
            // Passive: no inline action (the row renders a "View" button and
            // the modal shows the pending-confirmation card).
            (None, Some("Reveal pending confirmation".into()), Some("Your reveal is broadcast and waiting to confirm (usually ~10 minutes).".into()))
        }
        AuctionTaskState::RevealDoneWaitingForClose => {
            (None, Some("Revealed — waiting for close".into()), Some("Your reveal is confirmed. The auction stays open for reveals until the window closes.".into()))
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
    // Single critical section: the guard checks AND draft-insert + coin
    // reservation share ONE MutexGuard (see the doc comment inside
    // `build_open_draft_inner`). We grab the lock here and hand `&Connection`
    // to the inner function.
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_open_draft_inner(&conn, &ctx, &name, fee_rate)
}

/// Pure inner logic for `build_open_draft`, testable without a Tauri
/// `State<AppState>`. Callers must hold the DB mutex for the full duration
/// of this call — the double-open guard + persist path is atomic under that
/// single held guard.
pub(crate) fn build_open_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    let nh_hex = hex::encode(nh);
    let raw = names::raw_name(name)?;
    // OPEN output goes to the next unused wallet receive address (value 0).
    let recv = crate::noncustodial::derivation::next_unused_receive_address(
        conn,
        &ctx.profile_id,
        ctx.account,
        ctx.network,
        &ctx.account_xpub,
    )?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        None,
        PrimaryOutput {
            value: 0,
            address: recv.address,
            covenant: covenants::open(&nh, &raw),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;

    // --- Double-open guard (Task 1, mirrors the I2
    // bid-multiplicity guard in `build_bid_draft` above) + draft insert/coin
    // reservation, ALL under the caller's held MutexGuard.
    //
    // Safety rule: don't let this wallet broadcast a second OPEN for a name
    // it already opened. The UI already gates this (`can_open`, which now
    // reflects `has_pending_open` too), but that's advisory only — a second
    // window, a stale UI, or a replayed call can still reach this command
    // directly, so the rule must be enforced here. Two checks, either of
    // which blocks a second open:
    //   (a) an unspent COV_OPEN coin for this name anywhere in the profile —
    //       our OPEN is already live on-chain (or awaiting confirmation);
    //   (b) a not-yet-terminal `open` draft for this name — one is already
    //       queued/signed/broadcast and might still land.
    // This deliberately does NOT fetch node state to detect someone ELSE
    // having already opened the name — that's a phase change the UI's
    // `can_open` (phase != AVAILABLE) already catches; this guard only
    // stops OUR OWN duplicate broadcasts.
    //
    // Atomicity: the caller (the `#[tauri::command]` wrapper) locks the DB
    // mutex ONCE for this whole function — no unlock/relock. Without that,
    // two concurrent calls could both pass the checks before either had
    // written anything (classic TOCTOU). Tests call this function directly
    // with an owned `&Connection` (no mutex), which is fine because tests
    // are single-threaded.
    let existing_open_coins = queries::find_unspent_covenant_utxos_by_name_hash(
        conn,
        &ctx.profile_id,
        sync::COV_OPEN as i64,
        &nh_hex,
    )?;
    if !existing_open_coins.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "an auction for '{name}' is already being opened — wait for it to confirm"
        )));
    }
    if queries::has_pending_draft_for_name(conn, &ctx.profile_id, "open", name)? {
        return Err(AppError::InvalidInput(format!(
            "an auction for '{name}' is already being opened — wait for it to confirm"
        )));
    }

    persist_with_conn(conn, &ctx.profile_id, "open", name, None, None, &res)
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
        return Err(AppError::InvalidInput(
            "lockup must be >= bid value > 0".into(),
        ));
    }
    let ctx = load_ctx(&state)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_bid_draft_inner(&conn, &ctx, &name, bid_value, lockup, fee_rate, &ns)
}

/// Pure inner logic for `build_bid_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the multiplicity guard + commitment + draft persist are
/// atomic under that single held guard.
///
/// `ns` is the pre-fetched on-chain name state (the async RPC call
/// happens in the wrapper before the lock is acquired).
pub(crate) fn build_bid_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    bid_value: i64,
    lockup: i64,
    fee_rate: Option<u64>,
    ns: &NameState,
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    let nh_hex = hex::encode(nh);
    let raw = names::raw_name(name)?;

    // Bid output goes to the NEXT UNUSED wallet receive address. Rotation keeps
    // every bid on its own address; reveal/redeem additionally match the coin by
    // name hash, which is what keeps legacy bids (all on receive[0]) revealable.
    let bid_addr = crate::noncustodial::derivation::next_unused_receive_address(
        conn,
        &ctx.profile_id,
        ctx.account,
        ctx.network,
        &ctx.account_xpub,
    )?;
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
        PrimaryOutput {
            value: lockup as u64,
            address: bid_addr.address.clone(),
            covenant: cov,
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;

    // --- Bid-multiplicity guard (I2) + commitment persist + draft
    // insert/reservation, ALL under the caller's held MutexGuard.
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
    // Atomicity: the caller (the `#[tauri::command]` wrapper) locks the DB
    // mutex ONCE for this whole function — no unlock/relock. Without that,
    // two concurrent calls could both pass the checks before either had
    // written anything (classic TOCTOU). Tests call this function directly
    // with an owned `&Connection` (no mutex), which is fine because tests
    // are single-threaded.
    //
    // This mostly SUBSUMES the `insert_bid_commitment` ON CONFLICT fix
    // (I2 part 2, defense-in-depth in `queries::insert_bid_commitment`): with
    // this guard in place, a second bid on the same name is rejected here,
    // before a conflicting commitment row could ever be attempted. The
    // ON CONFLICT fix still matters as a second line of defense — e.g. if
    // this guard's on-chain/draft evidence is somehow stale — a same-value
    // re-bid must error instead of silently dropping its commitment row.
    let existing_bid_coins = queries::find_unspent_covenant_utxos_by_name_hash(
        conn,
        &ctx.profile_id,
        sync::COV_BID as i64,
        &nh_hex,
    )?;
    if !existing_bid_coins.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "wallet already has an unspent bid for '{name}' — one bid per wallet per name"
        )));
    }
    if queries::has_pending_bid_draft_for_name(conn, &ctx.profile_id, name)? {
        return Err(AppError::InvalidInput(format!(
            "a bid draft for '{name}' is already pending — one bid per wallet per name"
        )));
    }

    // Persist the bid commitment (secret nonce/blind) before building the
    // draft. If this fails — including the honest ON CONFLICT error above —
    // the function returns here and NO draft is ever persisted; a bid whose
    // commitment can't be trusted must never reach the chain.
    queries::insert_bid_commitment(
        conn,
        &ctx.profile_id,
        name,
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
    let reveal_end_height = names_pure::reveal_end_height(ns.height as i64, &params);
    queries::set_reveal_end_height(
        conn,
        &ctx.profile_id,
        &hex::encode(blind),
        reveal_end_height,
    )?;

    let summary = persist_with_conn(conn, &ctx.profile_id, "bid", name, None, None, &res)?;
    // Task 1 fix: stamp the on-chain bid txid onto this commitment NOW, while
    // still under the same held `conn` lock as the multiplicity guard and the
    // commitment/draft writes above — no unlock/relock, guard untouched.
    // `res.txid` is the deterministic pre-signing Handshake txid (no-witness
    // hash, identical before/after signing), verbatim, not reversed. Without
    // this, `bid_commitments.bid_txid` stays NULL forever (it was previously
    // only ever set by tests), which is what makes `merge_name_bids` unable
    // to recognize a real bid as the wallet's own.
    queries::set_bid_txid(conn, &ctx.profile_id, &hex::encode(blind), &res.txid)?;
    Ok(summary)
}

/// One name's pre-fetched auction state, gathered before the batch-bid
/// critical section so all network/hash errors surface before any DB write.
struct NameSpec {
    name: String,
    nh: [u8; 32],
    nh_hex: String,
    raw: Vec<u8>,
    ns: NameState,
}

/// One name's bid result inside a batch: the plan output plus the blind hex
/// needed to stamp the txid after the draft is persisted. Bundling these keeps
/// the two index-coupled — no risk of a `primaries[i]` / `blind_hexes[i]`
/// mismatch.
struct BidOutcome {
    primary: PrimaryOutput,
    blind_hex: String,
}

/// Batch-bid on multiple names in a single transaction. All names share the
/// same bid value and lockup; each gets its own receive address, nonce, blind,
/// and bid commitment row. Atomic: if any name fails the multiplicity guard or
/// phase check, the entire batch is rejected and no draft is persisted.
#[tauri::command]
pub async fn build_batch_bid_draft(
    state: State<'_, AppState>,
    names: Vec<String>,
    bid_value: i64,
    lockup: i64,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if names.is_empty() {
        return Err(AppError::InvalidInput("no names provided".into()));
    }
    if names.len() > MAX_BATCH_SIZE {
        return Err(AppError::InvalidInput(format!(
            "batch too large: {} names (max {})",
            names.len(),
            MAX_BATCH_SIZE
        )));
    }
    if bid_value <= 0 || lockup < bid_value {
        return Err(AppError::InvalidInput(
            "lockup must be >= bid value > 0".into(),
        ));
    }

    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let client = NodeRpcClient::from_settings(&ctx.settings);

    // Pre-fetch all name states and hashes to validate phase + catch errors
    // early, before any writes.
    let mut name_specs: Vec<NameSpec> = Vec::with_capacity(names.len());
    for name in &names {
        let nh = names::hash_name(name)?;
        let nh_hex = hex::encode(nh);
        let raw = names::raw_name(name)?;
        let ns = fetch_name_state(&client, name).await?;
        name_specs.push(NameSpec {
            name: name.clone(),
            nh,
            nh_hex,
            raw,
            ns,
        });
    }

    // Phase check: consensus only accepts a `bid` covenant while the auction
    // is in BIDDING (or the immediately-preceding OPENING window that the UI's
    // `is_bidding_compatible` check also accepts — see `names.rs` `derive_...`
    // capability logic). We enforce it here, BEFORE the DB critical section,
    // so no bid_commitments row or draft is ever persisted for an un-biddable
    // name — even when the UI is bypassed (the modal's own preflight is
    // advisory). A single ineligible name aborts the entire batch: batch-bid
    // is defined as all-or-nothing (see the fn doc-comment above).
    for spec in &name_specs {
        let phase = spec.ns.phase.as_str();
        if phase != "BIDDING" && phase != "OPENING" {
            let shown = if phase.is_empty() { "AVAILABLE" } else { phase };
            return Err(AppError::InvalidInput(format!(
                "'{}' is not open for bidding (phase: {})",
                spec.name, shown
            )));
        }
    }

    // --- Atomic section: multiplicity guard + all commitment/draft writes.
    // Hold the lock for the entire batch so no concurrent bid can slip in
    // between our guard checks and our writes.
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;

    // Guard: check that NO name in the batch already has an unspent bid coin
    // or pending bid draft.
    for spec in &name_specs {
        let existing_bid_coins = queries::find_unspent_covenant_utxos_by_name_hash(
            &conn,
            &ctx.profile_id,
            sync::COV_BID as i64,
            &spec.nh_hex,
        )?;
        if !existing_bid_coins.is_empty() {
            return Err(AppError::InvalidInput(format!(
                "wallet already has an unspent bid for '{}' — one bid per wallet per name",
                spec.name
            )));
        }
        if queries::has_pending_bid_draft_for_name(&conn, &ctx.profile_id, &spec.name)? {
            return Err(AppError::InvalidInput(format!(
                "a bid draft for '{}' is already pending — one bid per wallet per name",
                spec.name
            )));
        }
    }

    // All guards passed. Now derive addresses, compute nonces/blinds, and
    // persist commitments for all names. Collect one `BidOutcome` per name so
    // the plan-output list and the blind-hex list stay index-coupled by
    // construction (no risk of a `primaries[i]` / `blind_hexes[i]` mismatch).
    let mut outcomes: Vec<BidOutcome> = Vec::with_capacity(name_specs.len());
    let params = ctx.network.name_params();

    for spec in name_specs {
        // Derive next unused receive address for this bid.
        //
        // NOTE — divergence from `build_bid_draft`: the single-bid path
        // derives its address in a SEPARATE short-lived lock scope BEFORE
        // the critical section, then re-acquires the lock. We derive
        // INSIDE the held lock instead: for a batch of N names, each
        // iteration's `next_unused_receive_address` must see the previous
        // iteration's write to `derived_addresses` so it advances the
        // index — deriving them concurrently (or split across
        // lock/unlock/relock) would hand two names the same address. The
        // longer critical section is the price of that per-batch
        // atomicity.
        let bid_addr = crate::noncustodial::derivation::next_unused_receive_address(
            &conn,
            &ctx.profile_id,
            ctx.account,
            ctx.network,
            &ctx.account_xpub,
        )?;
        let (_v, program) = address::decode(ctx.network, &bid_addr.address)?;
        let mut addr_hash = [0u8; 20];
        if program.len() != 20 {
            return Err(AppError::InvalidInput("bid address is not p2wpkh".into()));
        }
        addr_hash.copy_from_slice(&program);

        let nonce = bids::compute_nonce(&ctx.account_xpub, &spec.nh, &addr_hash, bid_value as u64)?;
        let blind = bids::compute_blind(bid_value as u64, &nonce);
        let blind_hex = hex::encode(blind);
        let cov = covenants::bid(&spec.nh, spec.ns.height, &spec.raw, &blind);

        // Persist commitment before adding to the batch plan.
        queries::insert_bid_commitment(
            &conn,
            &ctx.profile_id,
            &spec.name,
            &spec.nh_hex,
            &bid_addr.address,
            bid_addr.branch as i64,
            bid_addr.child_index as i64,
            bid_value,
            lockup,
            &hex::encode(nonce),
            &blind_hex,
        )?;

        // Estimate reveal-end height and stamp it.
        let reveal_end_height = names_pure::reveal_end_height(spec.ns.height as i64, &params);
        queries::set_reveal_end_height(&conn, &ctx.profile_id, &blind_hex, reveal_end_height)?;

        outcomes.push(BidOutcome {
            primary: PrimaryOutput {
                value: lockup as u64,
                address: bid_addr.address.clone(),
                covenant: cov,
            },
            blind_hex,
        });
    }

    // Build the batch plan with all bid outputs (no name inputs — bids are
    // fresh outputs, not name coin spends).
    let primaries: Vec<PrimaryOutput> = outcomes.iter().map(|o| o.primary.clone()).collect();
    let res = actions::build_batch_plan(
        ctx.network,
        ctx.account,
        // Bids create fresh covenant outputs; they don't spend any name coin,
        // so there are no name inputs (only wallet funding, added inside).
        &[],
        &primaries,
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;

    // Persist the draft. Note: the schema stores one name per draft; for a
    // batch we use the first name as the draft label (a limitation).
    let display_name = names_pure::display_names(&names);
    let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let summary = persist_with_conn(
        &conn,
        &ctx.profile_id,
        "batch-bid",
        &display_name,
        None,
        Some(&name_refs),
        &res,
    )?;

    // Stamp the pre-signing txid onto each commitment (same as single-bid).
    for outcome in &outcomes {
        queries::set_bid_txid(&conn, &ctx.profile_id, &outcome.blind_hex, &res.txid)?;
    }

    Ok(summary)
}

// --- REVEAL ----------------------------------------------------------------

#[tauri::command]
pub async fn build_reveal_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    // Look up our bid commitment + the unspent BID coin at that address.
    let (bid, bid_coin) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let nh = names::hash_name(&name)?;
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

    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let summary = build_reveal_draft_inner(&conn, &ctx, &name, fee_rate, &ns, &bid, &bid_coin)?;

    Ok(summary)
}

/// Pure inner logic for `build_reveal_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the commitment lookup + draft persist are atomic under that
/// single held guard.
///
/// `ns` is the pre-fetched on-chain name state (the async RPC call happens
/// in the wrapper before the lock is acquired). `bid` and `bid_coin` are
/// pre-fetched from the DB in the wrapper.
pub(crate) fn build_reveal_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    fee_rate: Option<u64>,
    ns: &NameState,
    bid: &queries::BidCommitmentRow,
    bid_coin: &queries::NameCoin,
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;

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
    let summary = persist_with_conn(conn, &ctx.profile_id, "reveal", name, None, None, &res)?;
    // Task 1 fix (companion to build_bid_draft): stamp the reveal txid onto
    // the SAME commitment row (keyed by name — `set_bid_reveal_txid`), so the
    // reveal-deadline scanner (which reads `reveal_txid`) can see this bid as
    // resolved. Done under the caller's held lock, right after the draft
    // persist — `res.txid` is the deterministic pre-signing Handshake txid.
    queries::set_bid_reveal_txid(conn, &ctx.profile_id, name, &res.txid)?;
    Ok(summary)
}

// --- REDEEM ----------------------------------------------------------------

#[tauri::command]
pub async fn build_redeem_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    let coin = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let nh = names::hash_name(&name)?;
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
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_redeem_draft_inner(&conn, &ctx, &name, fee_rate, &ns, &coin)
}

/// Pure inner logic for `build_redeem_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the coin-reservation + draft persist are atomic under that
/// single held guard.
///
/// `ns` is the pre-fetched on-chain name state (the async RPC call happens
/// in the wrapper before the lock is acquired). `reveal_coin` is the wallet's
/// unspent losing-REVEAL coin for `name`, pre-fetched by the wrapper.
pub(crate) fn build_redeem_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    fee_rate: Option<u64>,
    ns: &NameState,
    reveal_coin: &queries::NameCoin,
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;

    // REDEEM reclaims the reveal output value back to the wallet.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(reveal_coin.clone())),
        PrimaryOutput {
            value: reveal_coin.value,
            address: reveal_coin.address.clone(),
            covenant: covenants::redeem(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist_with_conn(conn, &ctx.profile_id, "redeem", name, None, None, &res)
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
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_register_draft_inner(
        &conn,
        &ctx,
        &name,
        records.as_deref(),
        fee_rate,
        &ns,
        &coin,
        &rblock,
    )
}

/// Pure inner logic for `build_register_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the coin-reservation + draft persist are atomic under that
/// single held guard.
///
/// The two async RPC calls (`fetch_name_state` for `ns` and `renewal_block`
/// for `rblock`) happen in the wrapper before the lock is acquired; the
/// owner `NameCoin` is looked up by the wrapper as well. This inner takes
/// all three as resolved inputs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_register_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    records: Option<&[serde_json::Value]>,
    fee_rate: Option<u64>,
    ns: &NameState,
    owner_coin: &queries::NameCoin,
    renewal_block: &[u8; 32],
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    let res_bytes = match records {
        Some(r) if !r.is_empty() => resource::encode(r)?,
        _ => Vec::new(), // EMPTY resource — a REGISTER with no DNS records.
    };
    // REGISTER locks `ns.value` (the auction's clearing price); the rest of
    // the owner-coin value returns as change to the wallet.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(owner_coin.clone())),
        PrimaryOutput {
            value: ns.value,
            address: owner_coin.address.clone(),
            covenant: covenants::register(&nh, ns.height, &res_bytes, renewal_block),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist_with_conn(conn, &ctx.profile_id, "register", name, None, None, &res)
}

#[tauri::command]
pub async fn build_update_draft(
    state: State<'_, AppState>,
    name: String,
    records: Vec<serde_json::Value>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_update_draft_inner(&conn, &ctx, &name, &records, fee_rate, &ns, &coin)
}

/// Pure inner logic for `build_update_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the coin-reservation + draft persist are atomic under that
/// single held guard.
///
/// `ns` is the pre-fetched on-chain name state (the async RPC call happens
/// in the wrapper before the lock is acquired). `owner_coin` is the wallet's
/// current owner coin for `name`, pre-fetched by the wrapper.
pub(crate) fn build_update_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    records: &[serde_json::Value],
    fee_rate: Option<u64>,
    ns: &NameState,
    owner_coin: &queries::NameCoin,
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    let res_bytes = resource::encode(records)?;
    // UPDATE keeps the full owner-coin value on the name (no price change);
    // only the resource records get rewritten.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(owner_coin.clone())),
        PrimaryOutput {
            value: owner_coin.value,
            address: owner_coin.address.clone(),
            covenant: covenants::update(&nh, ns.height, &res_bytes),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist_with_conn(conn, &ctx.profile_id, "update", name, None, None, &res)
}

#[tauri::command]
pub async fn build_renew_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_renew_draft_inner(&conn, &ctx, &name, fee_rate, &ns, &coin, &rblock)
}

/// Pure inner logic for `build_renew_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the coin-reservation + draft persist are atomic under that
/// single held guard.
///
/// The two async RPC calls (`fetch_name_state` for `ns` and `renewal_block`
/// for `rblock`) happen in the wrapper before the lock is acquired; the
/// owner `NameCoin` is looked up by the wrapper as well. This inner takes
/// all three as resolved inputs.
pub(crate) fn build_renew_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    fee_rate: Option<u64>,
    ns: &NameState,
    owner_coin: &queries::NameCoin,
    renewal_block: &[u8; 32],
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    // RENEW keeps the full owner-coin value on the name (no price change);
    // only the renewal-block reference is refreshed to extend the lease.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(owner_coin.clone())),
        PrimaryOutput {
            value: owner_coin.value,
            address: owner_coin.address.clone(),
            covenant: covenants::renew(&nh, ns.height, renewal_block),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist_with_conn(conn, &ctx.profile_id, "renew", name, None, None, &res)
}

#[tauri::command]
pub async fn build_transfer_draft(
    state: State<'_, AppState>,
    name: String,
    recipient: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_transfer_draft_inner(&conn, &ctx, &name, &recipient, fee_rate, &ns, &coin)
}

/// Pure inner logic for `build_transfer_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the coin-reservation + draft persist are atomic under that
/// single held guard.
///
/// `ns` is the pre-fetched on-chain name state (the async RPC call happens
/// in the wrapper before the lock is acquired). `owner_coin` is the wallet's
/// current owner coin for `name`, pre-fetched by the wrapper.
pub(crate) fn build_transfer_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    recipient: &str,
    fee_rate: Option<u64>,
    ns: &NameState,
    owner_coin: &queries::NameCoin,
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    let (version, program) = address::decode(ctx.network, recipient)?;
    // TRANSFER initiates a name transfer to `recipient`; the output keeps the
    // full owner-coin value on the name and stays at the current owner address
    // until FINALIZE moves it.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(owner_coin.clone())),
        PrimaryOutput {
            value: owner_coin.value,
            address: owner_coin.address.clone(),
            covenant: covenants::transfer(&nh, ns.height, version, &program),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist_with_conn(
        conn,
        &ctx.profile_id,
        "transfer",
        name,
        Some(recipient),
        None,
        &res,
    )
}

#[tauri::command]
pub async fn build_finalize_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_finalize_draft_inner(&conn, &ctx, &name, fee_rate, &ns, &coin, &rblock)
}

/// Pure inner logic for `build_finalize_draft`, testable without a Tauri
/// `State<AppState>`. The caller must hold the DB mutex for the full
/// duration — the coin-reservation + draft persist are atomic under that
/// single held guard.
///
/// The two async RPC calls (`fetch_name_state` for `ns` and `renewal_block`
/// for `rblock`) happen in the wrapper before the lock is acquired; the
/// owner `NameCoin` is looked up by the wrapper as well. This inner takes
/// all three as resolved inputs and performs the covenant parsing + target
/// address extraction synchronously.
pub(crate) fn build_finalize_draft_inner(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
    name: &str,
    fee_rate: Option<u64>,
    ns: &NameState,
    owner_coin: &queries::NameCoin,
    renewal_block: &[u8; 32],
) -> Result<TxDraftSummary, AppError> {
    let rate = self::fee_rate(ctx, fee_rate);
    let nh = names::hash_name(name)?;
    let raw = names::raw_name(name)?;

    // The finalize output goes to the TRANSFER target recorded on the owner
    // coin's covenant: items = [nameHash, height, version(u8), addrHash].
    let cov_json = owner_coin.covenant_json.as_deref().ok_or_else(|| {
        AppError::InvalidInput("name is not in transfer; nothing to finalize".into())
    })?;
    let cov: serde_json::Value = serde_json::from_str(cov_json)?;
    let items = cov
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or_else(|| AppError::InvalidInput("owner coin has no covenant items".into()))?;
    if items.len() < 4 {
        return Err(AppError::InvalidInput(
            "owner coin is not a TRANSFER".into(),
        ));
    }
    let ver_hex = items[2].as_str().unwrap_or("00");
    let hash_hex = items[3].as_str().unwrap_or("");
    let version = u8::from_str_radix(ver_hex, 16).unwrap_or(0);
    let target_hash = hex::decode(hash_hex)
        .map_err(|e| AppError::InvalidInput(format!("bad transfer target: {e}")))?;
    if version != 0 || target_hash.len() != 20 {
        return Err(AppError::InvalidInput(
            "finalize target must be p2wpkh".into(),
        ));
    }
    let mut h160 = [0u8; 20];
    h160.copy_from_slice(&target_hash);
    let target_address = address::encode_p2wpkh(ctx.network, &h160)?;

    let flags: u8 = if ns.weak { 1 } else { 0 };
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(owner_coin.clone())),
        PrimaryOutput {
            value: owner_coin.value,
            address: target_address.clone(),
            covenant: covenants::finalize(
                &nh,
                ns.height,
                &raw,
                flags,
                ns.claimed,
                ns.renewals,
                renewal_block,
            ),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist_with_conn(
        conn,
        &ctx.profile_id,
        "finalize",
        name,
        Some(&target_address),
        None,
        &res,
    )
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
    persist(&state, &ctx.profile_id, "cancel", &name, None, None, &res)
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
    persist(&state, &ctx.profile_id, "revoke", &name, None, None, &res)
}

// ---------------------------------------------------------------------------
// Batch operations — build a single tx with multiple covenant outputs.
// ---------------------------------------------------------------------------

/// Max names per batch operation. Conservative cap that stays well under
/// block-size limits under any covenant mix. hsd itself enforces per-covenant-type
/// block limits (MAX_BLOCK_RENEWALS, MAX_BLOCK_OPENS, …); this batch cap is a
/// client-side safety net to prevent building a tx that the node would reject.
pub const MAX_BATCH_SIZE: usize = 100;

#[tauri::command]
pub async fn build_batch_renew_draft(
    state: State<'_, AppState>,
    names: Vec<String>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if names.is_empty() {
        return Err(AppError::InvalidInput("no names provided".into()));
    }
    if names.len() > MAX_BATCH_SIZE {
        return Err(AppError::InvalidInput(format!(
            "batch too large: {} names (max {})",
            names.len(),
            MAX_BATCH_SIZE
        )));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;

    let mut primaries = Vec::new();
    let mut name_inputs = Vec::new();
    let mut batch_names = Vec::new();

    for name in &names {
        let nh = names::hash_name(name)?;
        let (coin, ns) = owner_coin_and_state(&state, &ctx, name).await?;
        let addr = coin.address.clone();
        let value = coin.value;
        name_inputs.push(name_input_from(coin));
        primaries.push(PrimaryOutput {
            value,
            address: addr,
            covenant: covenants::renew(&nh, ns.height, &rblock),
        });
        batch_names.push(name.clone());
    }

    let res = actions::build_batch_plan(
        ctx.network,
        ctx.account,
        &name_inputs,
        &primaries,
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    // Persist with first name as primary; the draft plan contains all names.
    let display_name = names_pure::display_names(&batch_names);
    let name_refs: Vec<&str> = batch_names.iter().map(|s| s.as_str()).collect();
    persist(
        &state,
        &ctx.profile_id,
        "batch-renew",
        &display_name,
        None,
        Some(&name_refs),
        &res,
    )
}

#[tauri::command]
pub async fn build_batch_reveal_draft(
    state: State<'_, AppState>,
    names: Vec<String>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if names.is_empty() {
        return Err(AppError::InvalidInput("no names provided".into()));
    }
    if names.len() > MAX_BATCH_SIZE {
        return Err(AppError::InvalidInput(format!(
            "batch too large: {} names (max {})",
            names.len(),
            MAX_BATCH_SIZE
        )));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let mut primaries = Vec::new();
    let mut name_inputs = Vec::new();
    let mut batch_names = Vec::new();

    for name in &names {
        let nh = names::hash_name(name)?;
        // DB reads — lock is held only briefly, dropped before any await.
        let (bid, bid_coin) = {
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            let bid = queries::get_bid_commitment(&conn, &ctx.profile_id, name)?
                .ok_or_else(|| AppError::NotFound(format!("no bid commitment for '{}'", name)))?;
            let coin = queries::find_unspent_covenant_utxo(
                &conn,
                &ctx.profile_id,
                &bid.address,
                sync::COV_BID as i64,
                name,
                &hex::encode(nh),
            )?
            .ok_or_else(|| {
                AppError::NotFound(format!("no unspent bid coin for '{}' (sync first?)", name))
            })?;
            (bid, coin)
        };
        let mut nonce = [0u8; 32];
        let nb =
            hex::decode(&bid.nonce_hex).map_err(|e| AppError::Crypto(format!("nonce: {e}")))?;
        if nb.len() != 32 {
            return Err(AppError::Crypto(format!(
                "stored nonce for '{}' not 32 bytes",
                name
            )));
        }
        nonce.copy_from_slice(&nb);
        // Async RPC — no DB lock held here.
        let ns = fetch_name_state(&client, name).await?;
        let cov = covenants::reveal(&nh, ns.height, &nonce);
        name_inputs.push(name_input_from(bid_coin.clone()));
        primaries.push(PrimaryOutput {
            value: bid.bid_value_doos as u64,
            address: bid_coin.address.clone(),
            covenant: cov,
        });
        batch_names.push(name.clone());
    }

    let res = actions::build_batch_plan(
        ctx.network,
        ctx.account,
        &name_inputs,
        &primaries,
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    let display_name = names_pure::display_names(&batch_names);
    let name_refs: Vec<&str> = batch_names.iter().map(|s| s.as_str()).collect();
    persist(
        &state,
        &ctx.profile_id,
        "batch-reveal",
        &display_name,
        None,
        Some(&name_refs),
        &res,
    )
}

#[tauri::command]
pub async fn build_batch_redeem_draft(
    state: State<'_, AppState>,
    names: Vec<String>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if names.is_empty() {
        return Err(AppError::InvalidInput("no names provided".into()));
    }
    if names.len() > MAX_BATCH_SIZE {
        return Err(AppError::InvalidInput(format!(
            "batch too large: {} names (max {})",
            names.len(),
            MAX_BATCH_SIZE
        )));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let mut primaries = Vec::new();
    let mut name_inputs = Vec::new();
    let mut batch_names = Vec::new();

    for name in &names {
        let nh = names::hash_name(name)?;
        let ns = fetch_name_state(&client, name).await?;
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let bid = queries::get_bid_commitment(&conn, &ctx.profile_id, name)?
            .ok_or_else(|| AppError::NotFound(format!("no bid for '{}'", name)))?;
        let coin = queries::find_unspent_covenant_utxo(
            &conn,
            &ctx.profile_id,
            &bid.address,
            sync::COV_REVEAL as i64,
            name,
            &hex::encode(nh),
        )?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "no unspent losing reveal coin for '{}' (sync first?)",
                name
            ))
        })?;
        drop(conn);
        let cov = covenants::redeem(&nh, ns.height);
        name_inputs.push(name_input_from(coin.clone()));
        primaries.push(PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: cov,
        });
        batch_names.push(name.clone());
    }

    let res = actions::build_batch_plan(
        ctx.network,
        ctx.account,
        &name_inputs,
        &primaries,
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    let display_name = names_pure::display_names(&batch_names);
    let name_refs: Vec<&str> = batch_names.iter().map(|s| s.as_str()).collect();
    persist(
        &state,
        &ctx.profile_id,
        "batch-redeem",
        &display_name,
        None,
        Some(&name_refs),
        &res,
    )
}

#[tauri::command]
pub async fn build_batch_finalize_draft(
    state: State<'_, AppState>,
    names: Vec<String>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if names.is_empty() {
        return Err(AppError::InvalidInput("no names provided".into()));
    }
    if names.len() > MAX_BATCH_SIZE {
        return Err(AppError::InvalidInput(format!(
            "batch too large: {} names (max {})",
            names.len(),
            MAX_BATCH_SIZE
        )));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let mut primaries = Vec::new();
    let mut name_inputs = Vec::new();
    let mut batch_names = Vec::new();

    for name in &names {
        let nh = names::hash_name(name)?;
        let raw = names::raw_name(name)?;
        let (coin, ns) = owner_coin_and_state(&state, &ctx, name).await?;

        // Extract TRANSFER target from the owner coin's covenant.
        let cov_json = coin.covenant_json.as_deref().ok_or_else(|| {
            AppError::InvalidInput(format!(
                "'{}' is not in transfer; nothing to finalize",
                name
            ))
        })?;
        let cov: serde_json::Value = serde_json::from_str(cov_json)?;
        let items = cov.get("items").and_then(|i| i.as_array()).ok_or_else(|| {
            AppError::InvalidInput(format!("'{}' owner coin has no covenant items", name))
        })?;
        if items.len() < 4 {
            return Err(AppError::InvalidInput(format!(
                "'{}' owner coin is not a TRANSFER",
                name
            )));
        }
        let ver_hex = items[2].as_str().unwrap_or("00");
        let hash_hex = items[3].as_str().unwrap_or("");
        let version = u8::from_str_radix(ver_hex, 16).unwrap_or(0);
        let target_hash = hex::decode(hash_hex)
            .map_err(|e| AppError::InvalidInput(format!("'{}' bad transfer target: {e}", name)))?;
        if version != 0 || target_hash.len() != 20 {
            return Err(AppError::InvalidInput(format!(
                "'{}' finalize target must be p2wpkh",
                name
            )));
        }
        let mut h160 = [0u8; 20];
        h160.copy_from_slice(&target_hash);
        let target_address = address::encode_p2wpkh(ctx.network, &h160)?;

        let flags: u8 = if ns.weak { 1 } else { 0 };
        let cov = covenants::finalize(
            &nh,
            ns.height,
            &raw,
            flags,
            ns.claimed,
            ns.renewals,
            &rblock,
        );
        name_inputs.push(name_input_from(coin.clone()));
        primaries.push(PrimaryOutput {
            value: coin.value,
            address: target_address,
            covenant: cov,
        });
        batch_names.push(name.clone());
    }

    let res = actions::build_batch_plan(
        ctx.network,
        ctx.account,
        &name_inputs,
        &primaries,
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    let display_name = names_pure::display_names(&batch_names);
    let name_refs: Vec<&str> = batch_names.iter().map(|s| s.as_str()).collect();
    persist(
        &state,
        &ctx.profile_id,
        "batch-finalize",
        &display_name,
        None,
        Some(&name_refs),
        &res,
    )
}

// ---------------------------------------------------------------------------
// Paid name swaps (atomic finalize-with-payment)
// ---------------------------------------------------------------------------

/// Build a tx that finalizes a TRANSFER and pays the seller in the same tx.
///
/// Atomic name swap protocol:
/// 1. Seller transfers name to buyer's address (TRANSFER covenant, lockup period)
/// 2. Lockup expires → TRANSFER coin is now owned by buyer's address
/// 3. Buyer builds this tx:
///    - Spends TRANSFER coin (buyer owns it)
///    - Output 1: FINALIZE covenant (ownership transfers to buyer)
///    - Output 2: Payment to seller's address (buyer pays)
///    - Output 3: Change (if any)
///
/// The buyer's wallet funds the payment output from regular HNS coins.
#[tauri::command]
pub async fn build_finalize_with_payment_draft(
    state: State<'_, AppState>,
    name: String,
    payment_address: String,
    payment_value: u64,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if payment_value == 0 {
        return Err(AppError::InvalidInput(
            "payment value must be non-zero".into(),
        ));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let raw = names::raw_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;

    // Parse the TRANSFER covenant to extract the finalize target address.
    let cov_json = coin.covenant_json.as_deref().ok_or_else(|| {
        AppError::InvalidInput("name is not in transfer; nothing to finalize".into())
    })?;
    let cov: serde_json::Value = serde_json::from_str(cov_json)?;
    let items = cov
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or_else(|| AppError::InvalidInput("owner coin has no covenant items".into()))?;
    if items.len() < 4 {
        return Err(AppError::InvalidInput(
            "owner coin is not a TRANSFER".into(),
        ));
    }
    let ver_hex = items[2].as_str().unwrap_or("00");
    let hash_hex = items[3].as_str().unwrap_or("");
    let version = u8::from_str_radix(ver_hex, 16).unwrap_or(0);
    let target_hash = hex::decode(hash_hex)
        .map_err(|e| AppError::InvalidInput(format!("bad transfer target: {e}")))?;
    if version != 0 || target_hash.len() != 20 {
        return Err(AppError::InvalidInput(
            "finalize target must be p2wpkh".into(),
        ));
    }
    let mut h160 = [0u8; 20];
    h160.copy_from_slice(&target_hash);
    let target_address = address::encode_p2wpkh(ctx.network, &h160)?;

    // Validate the payment address is valid for this network.
    let (_, pay_program) = address::decode(ctx.network, &payment_address)?;
    if pay_program.is_empty() {
        return Err(AppError::InvalidInput(
            "invalid payment address for this network".into(),
        ));
    }

    let flags: u8 = if ns.weak { 1 } else { 0 };
    let finalize_cov = covenants::finalize(
        &nh,
        ns.height,
        &raw,
        flags,
        ns.claimed,
        ns.renewals,
        &rblock,
    );

    let res = actions::build_finalize_with_payment_plan(
        ctx.network,
        ctx.account,
        name_input_from(coin.clone()),
        PrimaryOutput {
            value: coin.value,
            address: target_address.clone(),
            covenant: finalize_cov,
        },
        payment_address.clone(),
        payment_value,
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(
        &state,
        &ctx.profile_id,
        "finalize-with-payment",
        &name,
        Some(&payment_address),
        None,
        &res,
    )
}
