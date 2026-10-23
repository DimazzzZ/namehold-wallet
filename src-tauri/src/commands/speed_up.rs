//! Speed up (child pays for parent, spec 2026-10-21): the commands that check
//! a parent, preview its child, and build the child's draft, and the re-check
//! of the parent before the child is broadcast.
//!
//! The child is an ordinary stored-plan draft: it is signed by `sign_plan`
//! through `sign_tx_draft` (after the secure confirmation,
//! `tx::confirm_details_for_draft`) and sent by `broadcast_tx_draft`, which
//! calls [`recheck_before_broadcast`] first. The pure part (fee, plan,
//! refusals on the parent's entry) is `noncustodial::speed_up`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use crate::commands::draft_ctx::random_id;
use crate::db::queries::{self, DraftStatus, PurchaseState, TxDraftRow};
use crate::error::AppError;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{broadcast_gates, is_not_in_mempool, NodeRpcClient};
use crate::noncustodial::send::{self, SpendableCoin};
use crate::noncustodial::shakedex::purchase::PURCHASE_ACTION;
use crate::noncustodial::speed_up::{
    self as su, parse_mempool_entry, plan_child, refuse_parent_entry, ChildRequest, MempoolEntry,
    SpeedUpSummary, TargetSource, SPEED_UP_ACTION,
};
use crate::noncustodial::types::TxDraftSummary;
use crate::AppState;

/// The purchase a child speeds up is no longer an Unconfirmed purchase at
/// broadcast (R14).
pub const PURCHASE_MOVED_ON: &str =
    "The purchase it speeds up is no longer unconfirmed; nothing was sent.";

fn refuse(msg: &str) -> AppError {
    AppError::InvalidInput(msg.to_string())
}

fn lock(
    db: &Mutex<rusqlite::Connection>,
) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, AppError> {
    db.lock().map_err(|e| AppError::Lock(e.to_string()))
}

/// What the database says of a parent that may be sped up.
#[derive(Debug, Clone)]
pub(crate) struct Parent {
    pub(crate) draft_id: String,
    pub(crate) profile_id: String,
    pub(crate) txid: String,
    pub(crate) network: Network,
    pub(crate) network_name: String,
    pub(crate) account: u32,
    /// The parent's change, the child's one input.
    pub(crate) change: SpendableCoin,
    /// The profile's change address, the child's one output.
    pub(crate) change_address: String,
    pub(crate) is_purchase: bool,
    pub(crate) settings: HashMap<String, String>,
}

/// The refusals the database decides (R1-R4): the draft is ours, sent and
/// unconfirmed, not a child itself, from a profile that can sign; a purchase
/// is an Unconfirmed purchase; its change is a tracked, covenant-free coin
/// of ours that nothing spends or holds.
pub(crate) fn load_parent(
    conn: &rusqlite::Connection,
    parent_draft_id: &str,
) -> Result<Parent, AppError> {
    let draft = queries::get_tx_draft(conn, parent_draft_id)?
        .ok_or_else(|| AppError::NotFound(format!("draft {parent_draft_id}")))?;
    if draft.action == SPEED_UP_ACTION {
        return Err(refuse(su::IS_CHILD));
    }
    let profile = queries::get_wallet_profile(conn, &draft.wallet_profile_id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {}", draft.wallet_profile_id)))?;
    if profile.watch_only {
        return Err(refuse(su::WATCH_ONLY));
    }
    if draft.status != DraftStatus::Broadcasted || draft.confirmation_height.is_some() {
        return Err(refuse(su::NOT_SENT_UNCONFIRMED));
    }
    let txid = draft
        .txid
        .clone()
        .ok_or_else(|| refuse(su::NOT_SENT_UNCONFIRMED))?;
    let is_purchase = draft.action == PURCHASE_ACTION;
    if is_purchase {
        let unconfirmed = queries::get_shakedex_purchase_by_draft(conn, &draft.id)?
            .is_some_and(|p| p.state == PurchaseState::Unconfirmed);
        if !unconfirmed {
            return Err(refuse(su::PURCHASE_NOT_UNCONFIRMED));
        }
    }
    let index = su::parent_change_index(
        &draft.action,
        &draft.signing_inputs_json,
        &draft.summary_json,
    )?
    .ok_or_else(|| refuse(su::NO_CHANGE))?;
    let vout = u32::try_from(index).map_err(|_| refuse(su::NO_CHANGE))?;
    if let Some(child) = queries::find_live_speed_up_child(conn, &draft.wallet_profile_id, &txid)? {
        // A child never sent still holds the change: say where to discard it,
        // never pass it by.
        return Err(refuse(if queries::never_sent(child.status) {
            su::UNSENT_CHILD
        } else {
            su::ALREADY_SPED_UP
        }));
    }
    send::release_stale_reservations(conn, &draft.wallet_profile_id)?;
    let out = queries::get_tracked_output(conn, &draft.wallet_profile_id, &txid, vout)?
        .ok_or_else(|| refuse(su::CHANGE_NOT_SYNCED))?;
    if out.covenant_type != 0 || out.spend_class != "liquid_hns" {
        return Err(refuse(su::NO_CHANGE));
    }
    if out.spent_by_txid.is_some() {
        return Err(refuse(su::CHANGE_SPENT));
    }
    if out.reserved_by_draft_id.is_some() {
        return Err(refuse(su::CHANGE_HELD));
    }
    let network = derivation::network_from_profile(&profile.network)?;
    let xpub = ExtendedPubKey::from_xpub(network, &profile.account_xpub)?;
    let change_address =
        derivation::derive_one(network, &xpub, derivation::BRANCH_CHANGE, 0)?.address;
    Ok(Parent {
        draft_id: draft.id,
        profile_id: draft.wallet_profile_id,
        txid: txid.clone(),
        network,
        network_name: profile.network,
        account: profile.account_index as u32,
        change: SpendableCoin {
            txid,
            vout,
            value: out.value,
            branch: out.branch,
            child_index: out.child_index,
        },
        change_address,
        is_purchase,
        settings: queries::get_settings(conn)?,
    })
}

/// The refusals the node decides (R1, R5, R6): a node that can send (the
/// gates of any broadcast), holding the parent in its mempool, not close to
/// its expiry, and paying at least the relay floor.
pub(crate) async fn read_parent_entry(
    client: &dyn NodeRpc,
    parent: &Parent,
    now: i64,
) -> Result<MempoolEntry, AppError> {
    if !client.source().can_broadcast() {
        return Err(refuse(su::NEEDS_NODE));
    }
    broadcast_gates(client, &parent.settings, Some(&parent.network_name)).await?;
    let entry = match client.get_mempool_entry(&parent.txid).await {
        Ok(v) => parse_mempool_entry(&v)?,
        Err(e) if is_not_in_mempool(&e) => return Err(refuse(su::NOT_IN_MEMPOOL)),
        Err(e) => return Err(e),
    };
    refuse_parent_entry(&entry, now)?;
    Ok(entry)
}

/// The target rate (R9) and where it came from: the typed one, else the
/// node's estimate (`estimatesmartfee`, as `tx::resolve_fee_rate` asks it),
/// else the user's fee-rate setting, else the relay-floor default. Unlike a
/// send's, the estimate comes before the setting: the setting is seeded at
/// the relay floor (migration 009), which every parent the mempool took
/// already pays, so it would make the default a refusal.
pub(crate) async fn resolve_target(
    client: &dyn NodeRpc,
    settings: &HashMap<String, String>,
    typed: Option<u64>,
) -> Result<(u64, TargetSource), AppError> {
    if let Some(rate) = typed {
        if rate == 0 {
            return Err(refuse("choose a rate of at least 1 doo per vbyte"));
        }
        return Ok((rate, TargetSource::Typed));
    }
    if let Ok(rate) = client.estimate_smart_fee(6).await {
        return Ok((rate, TargetSource::Estimate));
    }
    if let Some(kvb) = settings
        .get("fee_rate_doos_per_kvb")
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Ok((
            (kvb / 1000).max(send::MIN_FEE_RATE_PER_BYTE),
            TargetSource::Setting,
        ));
    }
    Ok((send::DEFAULT_FEE_RATE_PER_BYTE, TargetSource::Fallback))
}

/// The wallet's clock, unix seconds: R5 compares the parent's deadline with
/// it, as hsd does not report its own.
pub(crate) fn wallet_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Every check of a parent (R1-R6): the database's, then the node's.
pub(crate) async fn check_with(
    db: &Mutex<rusqlite::Connection>,
    client: &dyn NodeRpc,
    parent_draft_id: &str,
    now: i64,
) -> Result<(Parent, MempoolEntry), AppError> {
    let parent = load_parent(&*lock(db)?, parent_draft_id)?;
    let entry = read_parent_entry(client, &parent, now).await?;
    Ok((parent, entry))
}

fn child_request<'a>(
    parent: &'a Parent,
    entry: &'a MempoolEntry,
    target: (u64, TargetSource),
    now: i64,
) -> ChildRequest<'a> {
    ChildRequest {
        network: parent.network,
        account: parent.account,
        change: &parent.change,
        change_address: &parent.change_address,
        entry,
        target_rate: target.0,
        target_source: target.1,
        now,
        parent_txid: &parent.txid,
        parent_draft_id: &parent.draft_id,
        parent_is_purchase: parent.is_purchase,
    }
}

/// The child Speed up would build now, without storing it (the form's
/// numbers).
pub(crate) async fn preview_with(
    db: &Mutex<rusqlite::Connection>,
    client: &dyn NodeRpc,
    parent_draft_id: &str,
    typed: Option<u64>,
    now: i64,
) -> Result<SpeedUpSummary, AppError> {
    let (parent, entry) = check_with(db, client, parent_draft_id, now).await?;
    let target = resolve_target(client, &parent.settings, typed).await?;
    Ok(plan(&parent, &entry, target, now)?.summary)
}

/// [`plan_child`] for `parent`; with no estimate from the node, a target
/// that changes nothing says so (R9).
fn plan(
    parent: &Parent,
    entry: &MempoolEntry,
    target: (u64, TargetSource),
    now: i64,
) -> Result<su::ChildPlan, AppError> {
    plan_child(&child_request(parent, entry, target, now)).map_err(|e| match e {
        AppError::InvalidInput(m)
            if m == su::RATE_NOT_HIGHER
                && matches!(target.1, TargetSource::Setting | TargetSource::Fallback) =>
        {
            refuse(su::NO_ESTIMATE)
        }
        other => other,
    })
}

/// Build the child's draft (R7): every check again, the plan, and the draft
/// written with its one input reserved in one database transaction. The
/// database checks run once more inside that transaction, so a child built
/// meanwhile, or a send that took the change, refuses this one.
pub(crate) async fn build_with(
    db: &Mutex<rusqlite::Connection>,
    client: &dyn NodeRpc,
    parent_draft_id: &str,
    typed: Option<u64>,
    now: i64,
) -> Result<TxDraftSummary, AppError> {
    let (parent, entry) = check_with(db, client, parent_draft_id, now).await?;
    let target = resolve_target(client, &parent.settings, typed).await?;
    let child = plan(&parent, &entry, target, now)?;
    let conn = lock(db)?;
    let tx = conn.unchecked_transaction()?;
    load_parent(&tx, parent_draft_id)?;
    let id = random_id();
    queries::insert_tx_draft_reserving_coins_in_tx(
        &tx,
        &id,
        &parent.profile_id,
        SPEED_UP_ACTION,
        &child.unsigned_tx_hex,
        &serde_json::to_string(&child.plan)?,
        &serde_json::to_string(&child.summary)?,
        &child.plan.own_inputs(),
    )?;
    tx.commit()?;
    crate::commands::draft_ctx::draft_summary(&conn, &id)
}

/// A child's stored summary, or a "corrupted draft" refusal.
pub(crate) fn child_summary(child: &TxDraftRow) -> Result<SpeedUpSummary, AppError> {
    serde_json::from_str(&child.summary_json)
        .map_err(|e| AppError::Other(format!("corrupted draft: unreadable summary: {e}")))
}

/// R14: before a child is broadcast, its parent must still wait in the
/// node's mempool and, for a purchase, still be an Unconfirmed purchase. A
/// refused child never sent (`draft`/`signed`) is dropped with the reason
/// and its coin released; a retry of one the node may hold
/// (`broadcast_pending`) is left for the confirmation poll. A node that does
/// not answer refuses the broadcast and changes nothing.
pub(crate) async fn recheck_before_broadcast(
    db: &Mutex<rusqlite::Connection>,
    client: &dyn NodeRpc,
    child_id: &str,
) -> Result<(), AppError> {
    let (child, summary, purchase_ok) = {
        let conn = lock(db)?;
        let child = queries::get_tx_draft(&conn, child_id)?
            .ok_or_else(|| AppError::NotFound(format!("draft {child_id}")))?;
        let summary = child_summary(&child)?;
        let purchase_ok = !summary.parent_is_purchase
            || queries::get_shakedex_purchase_by_draft(&conn, &summary.parent_draft_id)?
                .is_some_and(|p| p.state == PurchaseState::Unconfirmed);
        (child, summary, purchase_ok)
    };
    let refusal = if !purchase_ok {
        Some(PURCHASE_MOVED_ON)
    } else {
        match client.get_mempool_entry(&summary.parent_txid).await {
            Ok(_) => None,
            Err(e) if is_not_in_mempool(&e) => Some(su::PARENT_GONE),
            Err(e) => return Err(e),
        }
    };
    let Some(reason) = refusal else {
        return Ok(());
    };
    if queries::never_sent(child.status) {
        let conn = lock(db)?;
        queries::update_tx_draft_status(&conn, child_id, DraftStatus::Dropped, Some(reason), None)?;
        queries::release_reserved_utxos_for_draft(&conn, child_id)?;
    }
    Err(refuse(reason))
}

/// Whether Speed up is offered for a draft, and why not (R16).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeedUpCheck {
    pub available: bool,
    /// Why it is not offered; `None` when it is.
    pub reason: Option<String>,
    /// The parent's mempool deadline (unix seconds) when it is offered.
    pub expires_at: Option<i64>,
}

/// The sentence a refusal shows: a refusal's own words, any other error as
/// it reads.
fn reason_text(e: AppError) -> String {
    match e {
        AppError::InvalidInput(m) => m,
        other => other.to_string(),
    }
}

pub(crate) async fn speed_up_check_with(
    db: &Mutex<rusqlite::Connection>,
    client: &dyn NodeRpc,
    parent_draft_id: &str,
    now: i64,
) -> SpeedUpCheck {
    match check_with(db, client, parent_draft_id, now).await {
        Ok((_, entry)) => SpeedUpCheck {
            available: true,
            reason: None,
            expires_at: Some(su::expires_at(&entry)),
        },
        Err(e) => SpeedUpCheck {
            available: false,
            reason: Some(reason_text(e)),
            expires_at: None,
        },
    }
}

/// The node of the profile a draft belongs to (ADR-001).
fn client_for_draft(
    state: &State<'_, AppState>,
    draft_id: &str,
) -> Result<NodeRpcClient, AppError> {
    let conn = lock(&state.db)?;
    let draft = queries::get_tx_draft(&conn, draft_id)?
        .ok_or_else(|| AppError::NotFound(format!("draft {draft_id}")))?;
    NodeRpcClient::for_profile(&conn, &draft.wallet_profile_id)
}

/// Whether the draft can be sped up now, and why not.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn speed_up_check(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<SpeedUpCheck, AppError> {
    let client = client_for_draft(&state, &draft_id)?;
    Ok(speed_up_check_with(&state.db, &client, &draft_id, wallet_now()).await)
}

/// The child Speed up would build at `fee_rate` (doos per vbyte; the node's
/// estimate when `None`), without storing it.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn preview_speed_up(
    state: State<'_, AppState>,
    draft_id: String,
    fee_rate: Option<u64>,
) -> Result<SpeedUpSummary, AppError> {
    let client = client_for_draft(&state, &draft_id)?;
    preview_with(&state.db, &client, &draft_id, fee_rate, wallet_now()).await
}

/// Build the child's draft; it is signed and sent like any draft.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn build_speed_up_draft(
    state: State<'_, AppState>,
    draft_id: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let client = client_for_draft(&state, &draft_id)?;
    build_with(&state.db, &client, &draft_id, fee_rate, wallet_now()).await
}
