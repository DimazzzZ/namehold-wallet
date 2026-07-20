//! Bid-commitment recovery + export.
//!
//! The `bid_commitments` DB row is the ONLY off-chain copy of a bid's true
//! value and nonce (the chain only carries the blind). Losing the row makes
//! the lockup unrecoverable without this module: `recover_bid_commitment`
//! rebuilds it from the account xpub + a user-supplied candidate value, and
//! `export_bid_commitments` lets the user back the whole table up alongside
//! their seed.
//!
//! Neither command touches secret key material — the nonce derivation (see
//! `noncustodial::bids`) uses only the account XPUB, which every profile
//! (including watch-only ones) already stores in the clear. So neither
//! command requires an unlocked signer session; they are read+DB-write, not
//! spends.

use serde::Serialize;
use tauri::State;

use crate::db::queries;
use crate::error::AppError;
use crate::noncustodial::bids::{compute_blind, compute_nonce};
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::sync::COV_BID;
use crate::noncustodial::{address, derivation, names};
use crate::AppState;

/// Non-secret confirmation returned after a successful recovery.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredBidCommitment {
    pub name: String,
    pub address: String,
    pub bid_value_doos: i64,
    pub lockup_value_doos: i64,
}

/// Recover a lost `bid_commitments` row for `name`.
///
/// Scans the profile's unspent BID coins across ALL its addresses (the row
/// that would normally tell us the address is exactly what's missing),
/// recomputes the nonce/blind for `bid_value_doos` against each candidate's
/// real address, and compares against the on-chain blind
/// (`covenant_json.items[3]`). The first match wins and is persisted; if none
/// match, nothing is written and a clear "doesn't match" error is returned.
///
/// `wallet_profile_id` is resolved the same way `get_name_action_capabilities`
/// resolves it (falls back to the active profile) — this pins recovery to a
/// specific wallet rather than silently trusting backend "active" state.
#[tauri::command]
pub async fn recover_bid_commitment(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
    name: String,
    bid_value_doos: i64,
) -> Result<RecoveredBidCommitment, AppError> {
    if bid_value_doos <= 0 {
        return Err(AppError::InvalidInput("bid value must be > 0".into()));
    }
    let profile_id = crate::commands::read::resolve_profile(&state, wallet_profile_id)?
        .ok_or_else(|| AppError::InvalidInput("no active wallet profile".into()))?;

    let nh = names::hash_name(&name)?;
    let nh_hex = hex::encode(nh);

    let (profile, candidates) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let profile = queries::get_wallet_profile(&conn, &profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("wallet profile {profile_id}")))?;
        let candidates = queries::find_unspent_covenant_utxos_by_name_hash(
            &conn,
            &profile_id,
            COV_BID as i64,
            &nh_hex,
        )?;
        (profile, candidates)
    };
    if candidates.is_empty() {
        return Err(AppError::NotFound(format!(
            "no unspent bid coin found for '{name}' (sync first?)"
        )));
    }

    let network = derivation::network_from_profile(&profile.network)?;
    let account_xpub = ExtendedPubKey::from_xpub(network, &profile.account_xpub)?;
    let value = bid_value_doos as u64;

    for coin in &candidates {
        let Ok((_version, program)) = address::decode(network, &coin.address) else {
            continue;
        };
        if program.len() != 20 {
            continue;
        }
        let mut addr_hash = [0u8; 20];
        addr_hash.copy_from_slice(&program);

        let nonce = compute_nonce(&account_xpub, &nh, &addr_hash, value)?;
        let blind = compute_blind(value, &nonce);
        let blind_hex = hex::encode(blind);

        let onchain_blind_hex = queries::covenant_item_hex(coin.covenant_json.as_deref(), 3);
        if onchain_blind_hex.as_deref() != Some(blind_hex.as_str()) {
            continue;
        }

        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        // I2: `insert_bid_commitment` now errors on a duplicate (name, blind)
        // key instead of silently no-op'ing (a same-value re-bid must never
        // silently drop its commitment row). Recovery is the one legitimate
        // idempotent caller — re-running it for an already-recovered bid
        // should succeed, not error — so check first and skip the insert
        // entirely when the exact row is already there.
        if !queries::bid_commitment_exists(&conn, &profile_id, &name, &blind_hex)? {
            queries::insert_bid_commitment(
                &conn,
                &profile_id,
                &name,
                &nh_hex,
                &coin.address,
                coin.branch as i64,
                coin.child_index as i64,
                bid_value_doos,
                coin.value as i64,
                &hex::encode(nonce),
                &blind_hex,
            )?;
        }
        return Ok(RecoveredBidCommitment {
            name,
            address: coin.address.clone(),
            bid_value_doos,
            lockup_value_doos: coin.value as i64,
        });
    }

    // No candidate's recomputed blind matched — nothing was written.
    Err(AppError::InvalidInput(
        "bid value doesn't match any unspent bid coin for this name".into(),
    ))
}

/// One exported bid commitment row. Includes the SECRET nonce/blind — this is
/// a wallet backup export, not a UI-facing read.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedBidCommitment {
    name: String,
    bid_value_doos: i64,
    lockup_value_doos: i64,
    address: String,
    branch: i64,
    child_index: i64,
    nonce_hex: String,
    blind_hex: String,
    bid_txid: Option<String>,
    reveal_txid: Option<String>,
}

/// Export every bid commitment for a profile as a JSON string, for the user to
/// save as a backup file. Contains secret nonce/blind material — the frontend
/// must warn the user to store it alongside their seed.
#[tauri::command]
pub async fn export_bid_commitments(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<String, AppError> {
    let profile_id = crate::commands::read::resolve_profile(&state, wallet_profile_id)?
        .ok_or_else(|| AppError::InvalidInput("no active wallet profile".into()))?;

    let rows = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::list_bid_commitments(&conn, &profile_id)?
    };

    let out: Vec<ExportedBidCommitment> = rows
        .into_iter()
        .map(|r| ExportedBidCommitment {
            name: r.name,
            bid_value_doos: r.bid_value_doos,
            lockup_value_doos: r.lockup_value_doos,
            address: r.address,
            branch: r.branch,
            child_index: r.child_index,
            nonce_hex: r.nonce_hex,
            blind_hex: r.blind_hex,
            bid_txid: r.bid_txid,
            reveal_txid: r.reveal_txid,
        })
        .collect();

    serde_json::to_string_pretty(&out)
        .map_err(|e| AppError::Other(format!("failed to serialize bid backup: {e}")))
}
