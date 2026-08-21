//! Paid swap offer commands: seller-side tracking for atomic finalizeWithPayment.

use crate::db::queries;
use crate::error::AppError;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::AppState;
use rusqlite::params;
use serde::Serialize;
use tauri::State;

/// A paid swap offer (seller tracking).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaidSwapOffer {
    pub name: String,
    pub buyer_address: String,
    pub price_doos: i64,
    pub transfer_txid: Option<String>,
    pub claimed: bool,
    pub created_at: String,
}

/// Create a paid swap offer: seller initiates a "sell with payment" flow.
/// Records the buyer's address and expected price for later verification.
#[tauri::command]
pub fn create_paid_swap_offer(
    state: State<'_, AppState>,
    name: String,
    buyer_address: String,
    price_doos: i64,
) -> Result<(), AppError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::InvalidInput("name cannot be empty".into()));
    }
    if buyer_address.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "buyer_address cannot be empty".into(),
        ));
    }
    if price_doos <= 0 {
        return Err(AppError::InvalidInput("price_doos must be positive".into()));
    }

    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "INSERT INTO paid_swap_offers (name, buyer_address, price_doos)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET buyer_address = excluded.buyer_address,
                                         price_doos = excluded.price_doos,
                                         claimed = 0,
                                         transfer_txid = NULL",
        params![name, buyer_address.trim(), price_doos],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn find_payment_output_verified_match() {
        // Buyer address = "hs1qbuyer", seller pays 1_000_000 to "hs1qseller".
        let tx = json!({
            "outputs": [
                { "value": 5_000_000, "address": "hs1qbuyer" }, // finalize covenant (name coin)
                { "value": 1_000_000, "address": "hs1qseller" }, // payment
                { "value": 500_000, "address": "hs1qbuyer" }, // change back to buyer
            ],
        });
        let result = find_payment_output(&tx, "hs1qbuyer", 1_000_000);
        assert_eq!(result, Some(1_000_000));
    }

    #[test]
    fn find_payment_output_no_matching_output() {
        // Only outputs are to the buyer.
        let tx = json!({
            "outputs": [
                { "value": 5_000_000, "address": "hs1qbuyer" },
                { "value": 500_000, "address": "hs1qbuyer" },
            ],
        });
        let result = find_payment_output(&tx, "hs1qbuyer", 1_000_000);
        assert_eq!(result, None);
    }

    #[test]
    fn find_payment_output_short_payment_rejected() {
        // Payment output exists but is below the expected price.
        let tx = json!({
            "outputs": [
                { "value": 5_000_000, "address": "hs1qbuyer" },
                { "value": 999_999, "address": "hs1qseller" }, // 1 doo short
            ],
        });
        let result = find_payment_output(&tx, "hs1qbuyer", 1_000_000);
        assert_eq!(result, None);
    }

    #[test]
    fn find_payment_output_higher_amount_accepted() {
        // Payment ≥ expected price is valid (buyer paid more than asked).
        let tx = json!({
            "outputs": [
                { "value": 5_000_000, "address": "hs1qbuyer" },
                { "value": 2_000_000, "address": "hs1qseller" }, // more than 1M
            ],
        });
        let result = find_payment_output(&tx, "hs1qbuyer", 1_000_000);
        assert_eq!(result, Some(2_000_000));
    }

    #[test]
    fn find_payment_output_missing_outputs_field() {
        // Malformed tx: no outputs array at all.
        let tx = json!({ "txid": "abc" });
        let result = find_payment_output(&tx, "hs1qbuyer", 1_000_000);
        assert_eq!(result, None);
    }

    #[test]
    fn find_payment_output_picks_first_matching() {
        // Two eligible outputs — should return the first.
        let tx = json!({
            "outputs": [
                { "value": 5_000_000, "address": "hs1qbuyer" },
                { "value": 1_500_000, "address": "hs1qseller" },
                { "value": 3_000_000, "address": "hs1qother" },
            ],
        });
        let result = find_payment_output(&tx, "hs1qbuyer", 1_000_000);
        assert_eq!(result, Some(1_500_000));
    }
}

/// Fetch a paid swap offer by name.
#[tauri::command]
pub fn get_paid_swap_offer(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<PaidSwapOffer>, AppError> {
    let name = name.trim().to_string();
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let mut stmt = db.prepare(
        "SELECT name, buyer_address, price_doos, transfer_txid, claimed, created_at
         FROM paid_swap_offers WHERE name = ?1",
    )?;
    let result = stmt.query_row(params![name], |row| {
        let claimed_int: i32 = row.get(4)?;
        Ok(PaidSwapOffer {
            name: row.get(0)?,
            buyer_address: row.get(1)?,
            price_doos: row.get(2)?,
            transfer_txid: row.get(3)?,
            claimed: claimed_int != 0,
            created_at: row.get(5)?,
        })
    });

    match result {
        Ok(offer) => Ok(Some(offer)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::Db(e)),
    }
}

/// Result of claiming a paid transfer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResult {
    pub verified: bool,
    pub paid_doos: i64,
    pub confirmations: u32,
}

/// Pure verification helper: given a tx JSON payload, the buyer's address, and
/// the expected minimum price, return the payment amount if a matching output
/// exists, otherwise `None`.
///
/// The payment output is any output whose address is NOT the buyer's and whose
/// value is at least `price_doos`. Extracted so it can be unit tested without
/// a live node.
pub fn find_payment_output(
    tx_json: &serde_json::Value,
    buyer_address: &str,
    price_doos: i64,
) -> Option<i64> {
    let outputs = tx_json.get("outputs")?.as_array()?;
    for output in outputs {
        let value = output.get("value").and_then(|v| v.as_i64()).unwrap_or(0);
        let addr = output.get("address").and_then(|a| a.as_str()).unwrap_or("");
        if addr == buyer_address {
            continue;
        }
        if value >= price_doos {
            return Some(value);
        }
    }
    None
}

/// Outcome of [`verify_paid_transfer_with_client`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PaidTransferVerification {
    /// The tx exists and contains a valid payment output to the seller.
    Verified { paid_doos: i64, confirmations: u32 },
    /// The tx exists but no qualifying payment output was found.
    NoPayment { confirmations: u32 },
}

/// Client-injected payment verification for [`claim_paid_transfer`]. Fetches
/// the tx by hash, checks it exists, extracts confirmations, and runs
/// [`find_payment_output`] to verify the finalize-with-payment protocol.
/// Returns `Err(NotFound)` when the node doesn't know the tx, or the
/// appropriate `PaidTransferVerification` variant. Testable against a mock.
pub(crate) async fn verify_paid_transfer_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    txid: &str,
    buyer_address: &str,
    price_doos: i64,
) -> Result<PaidTransferVerification, crate::error::AppError> {
    let tx_json = client.get_tx_by_hash(txid).await?;
    if tx_json.is_null() {
        return Err(crate::error::AppError::NotFound(format!(
            "tx not found: {}",
            txid
        )));
    }
    let confirmations = tx_json["confirmations"].as_u64().unwrap_or(0) as u32;
    match find_payment_output(&tx_json, buyer_address, price_doos) {
        Some(paid_doos) => Ok(PaidTransferVerification::Verified {
            paid_doos,
            confirmations,
        }),
        None => Ok(PaidTransferVerification::NoPayment { confirmations }),
    }
}

/// Claim a paid transfer: seller verifies the buyer's finalize-with-payment tx
/// contains a P2WPKH output to the seller's address with value ≥ price_doos.
/// This is verify-only; the payment already exists in the buyer's tx.
/// On success, marks the offer as claimed.
#[tauri::command]
pub async fn claim_paid_transfer(
    state: State<'_, AppState>,
    name: String,
    txid: String,
) -> Result<ClaimResult, AppError> {
    let name = name.trim().to_string();
    let txid = txid.trim().to_string();

    if name.is_empty() {
        return Err(AppError::InvalidInput("name cannot be empty".into()));
    }
    if txid.is_empty() {
        return Err(AppError::InvalidInput("txid cannot be empty".into()));
    }

    // Load the offer and settings (hold DB lock briefly).
    let (offer, settings) = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let mut stmt = db.prepare(
            "SELECT name, buyer_address, price_doos, transfer_txid, claimed, created_at
             FROM paid_swap_offers WHERE name = ?1",
        )?;
        let offer = match stmt.query_row(params![&name], |row| {
            let claimed_int: i32 = row.get(4)?;
            Ok(PaidSwapOffer {
                name: row.get(0)?,
                buyer_address: row.get(1)?,
                price_doos: row.get(2)?,
                transfer_txid: row.get(3)?,
                claimed: claimed_int != 0,
                created_at: row.get(5)?,
            })
        }) {
            Ok(o) => o,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(AppError::NotFound(format!(
                    "no paid swap offer for name: {}",
                    name
                )))
            }
            Err(e) => return Err(AppError::Db(e)),
        };

        if offer.claimed {
            return Err(AppError::InvalidInput("offer already claimed".into()));
        }

        let settings = queries::get_settings(&db)?;
        (offer, settings)
    };
    // DB lock dropped here — safe to do async RPC.

    // Fetch the tx from the node.
    let client = NodeRpcClient::from_settings(&settings);
    let verification =
        verify_paid_transfer_with_client(&client, &txid, &offer.buyer_address, offer.price_doos)
            .await?;
    let (paid_doos, confirmations) = match verification {
        PaidTransferVerification::Verified {
            paid_doos,
            confirmations,
        } => (paid_doos, confirmations),
        PaidTransferVerification::NoPayment { confirmations } => {
            return Ok(ClaimResult {
                verified: false,
                paid_doos: 0,
                confirmations,
            });
        }
    };

    // Mark as claimed in DB.
    {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db.execute(
            "UPDATE paid_swap_offers SET transfer_txid = ?1, claimed = 1 WHERE name = ?2",
            params![&txid, &name],
        )?;
    }

    Ok(ClaimResult {
        verified: true,
        paid_doos,
        confirmations,
    })
}

/// Remove a paid swap offer (seller cancels the sale).
#[tauri::command]
pub fn remove_paid_swap_offer(state: State<'_, AppState>, name: String) -> Result<(), AppError> {
    let name = name.trim().to_string();
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db.execute(
        "DELETE FROM paid_swap_offers WHERE name = ?1",
        params![name],
    )?;
    Ok(())
}
