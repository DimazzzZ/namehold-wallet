//! Paid swap offers recorded before the feature was withdrawn (2026-09-21),
//! read-only. Shakedex name sales replace it; nothing creates, claims or
//! removes an offer any more, and Activity lists the ones a seller recorded so
//! the agreed buyer and price stay visible. Tests:
//! `src/tests/paid_swaps_cmd_tests.rs`.

use crate::error::AppError;
use crate::AppState;
use serde::Serialize;
use tauri::State;

/// A recorded paid swap offer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaidSwapOffer {
    pub name: String,
    pub buyer_address: String,
    pub price_doos: i64,
    /// The buyer's transaction a claim checked, when one was claimed.
    pub transfer_txid: Option<String>,
    pub claimed: bool,
    pub created_at: String,
}

/// Every recorded offer, newest first.
#[tauri::command]
pub fn list_paid_swap_offers(state: State<'_, AppState>) -> Result<Vec<PaidSwapOffer>, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut stmt = db.prepare(
        "SELECT name, buyer_address, price_doos, transfer_txid, claimed, created_at
         FROM paid_swap_offers ORDER BY created_at DESC, name",
    )?;
    let rows = stmt.query_map([], |row| {
        let claimed: i32 = row.get(4)?;
        Ok(PaidSwapOffer {
            name: row.get(0)?,
            buyer_address: row.get(1)?,
            price_doos: row.get(2)?,
            transfer_txid: row.get(3)?,
            claimed: claimed != 0,
            created_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
