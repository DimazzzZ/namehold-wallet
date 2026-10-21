//! Finding our own Shakedex locks after a restore (R34): the candidates are
//! read from the profile's address history on the node (`GET /tx/address`,
//! no seed needed), each candidate's name from its hash, and every candidate
//! committing to the lock of the key derived for its name goes through R32's
//! restore by name unchanged ([`restore_lock_inner`]). The search runs once
//! per profile, until one run has read every address and judged every
//! candidate; after that only when the seller asks ("Find my locks").

use std::collections::HashSet;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::commands::shakedex::{
    authorize_signer, derive_listing_key, restore_lock_inner, software_writer_ctx, RestoreInput,
};
use crate::db::queries;
use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::shakedex::discover::{commits_to_lock, lock_candidates, LockCandidate};
use crate::noncustodial::shakedex::lock_key::LockKey;
use crate::noncustodial::shakedex::sell;
use crate::AppState;

/// Why the search does not run on a node without the address index. My
/// listings shows the same sentence.
pub const LOCK_SEARCH_NEEDS_INDEX: &str = "finding your locks needs a node run with --index-tx and --index-address; Restore a lock by name works without them";

/// What one search did: whether it ran at all (not when done already and not
/// forced, or against a node still syncing), the names adopted as Restored
/// locks, and whether the search is now done for the profile.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindLocksReport {
    pub ran: bool,
    pub adopted: Vec<String>,
    pub done: bool,
}

/// What My listings needs to offer the search while the signer is locked:
/// whether it is done, whether the node has the address index, and how many
/// candidates the history holds (read from the xpub alone).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockSearchStatus {
    pub done: bool,
    pub address_index: bool,
    pub candidates: u32,
}

/// Whether `e` is the node's refusal of `GET /tx/address` for want of the
/// address index, as `NodeRpcClient::get_txs_by_address` words it.
fn names_the_address_index(e: &AppError) -> bool {
    matches!(e, AppError::Rpc(m) if m.contains("address index not enabled"))
}

/// Whether hsd's `GET /` reply reports both indexes the search reads with
/// (`chain.indexers.indexTX` and `.indexAddress`). hsd answers
/// `GET /tx/address` with an empty list when either is off, so an empty
/// history proves nothing until this says both are on. A reply without both
/// flags as booleans is an error: never taken as "on".
async fn address_index_on(node: &dyn NodeRpc) -> Result<bool, AppError> {
    let status = node.get_node_status().await?;
    let flag = |name: &str| {
        status
            .pointer(&format!("/chain/indexers/{name}"))
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                AppError::Rpc(format!(
                    "node status did not report chain.indexers.{name}: could not check its indexes"
                ))
            })
    };
    Ok(flag("indexTX")? && flag("indexAddress")?)
}

fn lock_db(db: &Mutex<Connection>) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
    db.lock().map_err(|e| AppError::Lock(e.to_string()))
}

/// The history of every derived address of the profile, and whether every
/// one was read. A node without the address index refuses with
/// [`LOCK_SEARCH_NEEDS_INDEX`]; any other failed read leaves that address
/// unread.
async fn read_history(
    node: &dyn NodeRpc,
    addresses: &[String],
) -> Result<(Vec<serde_json::Value>, bool), AppError> {
    let mut txs = Vec::new();
    let mut all_read = true;
    for a in addresses {
        match node.get_txs_by_address(a).await {
            Ok(mut t) => txs.append(&mut t),
            Err(e) if names_the_address_index(&e) => {
                return Err(AppError::InvalidInput(LOCK_SEARCH_NEEDS_INDEX.into()));
            }
            Err(e) => {
                eprintln!("shakedex lock search: history of {a} not read: {e}");
                all_read = false;
            }
        }
    }
    Ok((txs, all_read))
}

/// One candidate, judged: `Ok(Some(name))` adopted, `Ok(None)` or an
/// `InvalidInput` refusal skipped (not ours, or not a lock R32 adopts), any
/// other error not judged. The reads mirror [`super::shakedex::shakedex_restore_lock`]'s.
async fn adopt_candidate(
    node: &dyn NodeRpc,
    db: &Mutex<Connection>,
    profile_id: &str,
    network: Network,
    c: &LockCandidate,
    derive: &(dyn Fn(&str) -> Result<LockKey, AppError> + Sync),
) -> Result<Option<String>, AppError> {
    let Some(name) = node.get_name_by_hash(&c.name_hash).await? else {
        return Ok(None);
    };
    // Only R32's checks judge a candidate: a key not derived (the signer
    // locked, or switched to another profile, mid-search) leaves it unjudged,
    // whatever error the signer gave.
    let key = derive(&name)
        .map_err(|e| AppError::Other(format!("the lock key of '{name}' was not derived: {e}")))?;
    if !commits_to_lock(c, &key.program, network) {
        return Ok(None);
    }
    let reply = node.get_name_info(&name).await?;
    let owner = sell::restore_owner(&reply, &name)?;
    let Some(coin) = node.get_coin(&owner.txid, owner.vout).await? else {
        return Ok(None);
    };
    let conn = lock_db(db)?;
    restore_lock_inner(
        &conn,
        &RestoreInput {
            profile_id,
            network,
            name: &name,
            key: &key,
            owner: &owner,
            coin: &coin,
        },
    )?;
    Ok(Some(name))
}

/// R34: search the profile's address history for our locks and adopt each
/// one still in its lock. Runs nothing when the search is done and not
/// `force`d (no node call), or when the node is not synced; refuses a node
/// without the address index. A failed history read or an unjudged candidate
/// (a failed node read) leaves the search not done, with what it adopted
/// kept. `derive` gives the lock key of a name. The database lock is never
/// held across a node read.
pub(crate) async fn find_locks_with(
    node: &dyn NodeRpc,
    db: &Mutex<Connection>,
    profile_id: &str,
    network: Network,
    force: bool,
    derive: &(dyn Fn(&str) -> Result<LockKey, AppError> + Sync),
) -> Result<FindLocksReport, AppError> {
    let not_run = |done| FindLocksReport {
        ran: false,
        adopted: Vec::new(),
        done,
    };
    let addresses = {
        let conn = lock_db(db)?;
        if !force && queries::lock_search_done(&conn, profile_id)? {
            return Ok(not_run(true));
        }
        queries::get_profile_addresses(&conn, profile_id)?
    };
    if !node.get_blockchain_info().await?.is_synced(true) {
        return Ok(not_run(lock_search_done_in(db, profile_id)?));
    }
    if !address_index_on(node).await? {
        return Err(AppError::InvalidInput(LOCK_SEARCH_NEEDS_INDEX.into()));
    }
    let (txs, mut all_judged) = read_history(node, &addresses).await?;
    let own: HashSet<String> = addresses.into_iter().collect();
    let mut adopted = Vec::new();
    for c in lock_candidates(&txs, &own, network) {
        match adopt_candidate(node, db, profile_id, network, &c, derive).await {
            Ok(Some(name)) => adopted.push(name),
            Ok(None) | Err(AppError::InvalidInput(_)) => {}
            Err(e) => {
                eprintln!(
                    "shakedex lock search: candidate {} not judged: {e}",
                    c.name_hash
                );
                all_judged = false;
            }
        }
    }
    if all_judged {
        queries::mark_lock_search_done(&*lock_db(db)?, profile_id)?;
    }
    Ok(FindLocksReport {
        ran: true,
        adopted,
        done: lock_search_done_in(db, profile_id)?,
    })
}

fn lock_search_done_in(db: &Mutex<Connection>, profile_id: &str) -> Result<bool, AppError> {
    queries::lock_search_done(&*lock_db(db)?, profile_id)
}

/// R34: the search's status for My listings, read without the seed: done
/// from the profile; otherwise the history of every derived address, the
/// address index missing when the node says so, and the number of
/// candidates. A failed read is an error.
pub(crate) async fn lock_search_status_with(
    node: &dyn NodeRpc,
    db: &Mutex<Connection>,
    profile_id: &str,
    network: Network,
) -> Result<LockSearchStatus, AppError> {
    let addresses = {
        let conn = lock_db(db)?;
        if queries::lock_search_done(&conn, profile_id)? {
            return Ok(LockSearchStatus {
                done: true,
                address_index: true,
                candidates: 0,
            });
        }
        queries::get_profile_addresses(&conn, profile_id)?
    };
    let no_index = LockSearchStatus {
        done: false,
        address_index: false,
        candidates: 0,
    };
    if !address_index_on(node).await? {
        return Ok(no_index);
    }
    let mut txs = Vec::new();
    for a in &addresses {
        match node.get_txs_by_address(a).await {
            Ok(mut t) => txs.append(&mut t),
            Err(e) if names_the_address_index(&e) => return Ok(no_index),
            Err(e) => return Err(e),
        }
    }
    let own: HashSet<String> = addresses.into_iter().collect();
    let candidates = lock_candidates(&txs, &own, network).len();
    Ok(LockSearchStatus {
        done: false,
        address_index: true,
        candidates: u32::try_from(candidates).unwrap_or(u32::MAX),
    })
}

/// Find our own locks (R34) on the active profile: a recovery-phrase profile
/// with a node that can send (`software_writer_ctx`, as R32's restore needs)
/// and the unlocked signer, both checked before anything is read; then
/// [`find_locks_with`], each lock key derived from the seed of the unlocked
/// session once its name is read. `force` runs a search already done ("Find
/// my locks").
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_find_locks(
    state: State<'_, AppState>,
    force: bool,
) -> Result<FindLocksReport, AppError> {
    let ctx = software_writer_ctx(&state)?;
    authorize_signer(&state, &ctx)?;
    let derive = |n: &str| derive_listing_key(&state, &ctx, n);
    find_locks_with(
        &ctx.node,
        &state.db,
        &ctx.profile_id,
        ctx.network,
        force,
        &derive,
    )
    .await
}

/// The search's status (R34) for My listings, read without the signer
/// ([`lock_search_status_with`]). A profile the search is not for (Ledger,
/// watch-only, extended private key) or a node that cannot adopt (SPV, an
/// explorer, a remote node without sending) has nothing to offer: done, with
/// no candidates.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_lock_search_status(
    state: State<'_, AppState>,
) -> Result<LockSearchStatus, AppError> {
    let ctx = match software_writer_ctx(&state) {
        Ok(ctx) => ctx,
        Err(AppError::InvalidInput(_)) => {
            return Ok(LockSearchStatus {
                done: true,
                address_index: true,
                candidates: 0,
            })
        }
        Err(e) => return Err(e),
    };
    lock_search_status_with(&ctx.node, &state.db, &ctx.profile_id, ctx.network).await
}
