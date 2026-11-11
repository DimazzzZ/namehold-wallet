//! What the secure window shows at a Commit (spec R16): every amount and
//! address is read from the transactions about to be signed and sent, never
//! from the terms alone, and every time from the terms the scripts commit
//! to.
//!
//! The rows are what the user is asked to trust, so they fail closed
//! (CODING_STANDARDS §Rust layering): a transaction that cannot be read is
//! a refused Commit, never a row reading `0` or `(unreadable)`.

use serde_json::{json, Value};

use crate::error::AppError;
use crate::noncustodial::exchange::commit::{BuyerCommit, SellerCommit};
use crate::noncustodial::exchange::price;
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::network::Network;
use crate::noncustodial::types::doos_to_hns_string;

pub const BUYER_TITLE: &str = "Lock BTC for HNS";
pub const BUYER_MESSAGE: &str = "This signs every transaction of your side of the trade at once: \
     your BTC goes into a lock the seller can open only by giving you the HNS, and comes back to \
     you on its own if the seller does not.";
pub const SELLER_TITLE: &str = "Lock HNS for BTC";
pub const SELLER_MESSAGE: &str = "This signs every transaction of your side of the trade at once: \
     your HNS goes into a lock the buyer can open only by revealing the secret that pays you the \
     BTC, and comes back to you on its own if the buyer does not.";

pub fn sats_text(sats: u64) -> String {
    let whole = sats / 100_000_000;
    let frac = sats % 100_000_000;
    format!("{whole}.{frac:08} BTC ({sats} sats)")
}

pub fn time_text(unix: u64) -> String {
    i64::try_from(unix)
        .ok()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        // A time no calendar holds is still shown as the number the scripts
        // commit to, never hidden.
        .unwrap_or_else(|| format!("{unix} (not a date)"))
}

/// "1 confirmation", "6 confirmations".
pub fn confirmations_text(n: u32) -> String {
    if n == 1 {
        "1 confirmation".into()
    } else {
        format!("{n} confirmations")
    }
}

fn row(label: &str, value: String) -> Value {
    json!({ "label": label, "value": value })
}

/// The refusal: the window cannot show what would be signed, so nothing is.
fn corrupted(what: &str) -> AppError {
    AppError::Other(format!(
        "this Commit cannot be shown, so nothing is signed: {what}"
    ))
}

fn network_of(terms: &Terms) -> Result<Network, AppError> {
    terms
        .network()
        .map_err(|_| corrupted("the terms name no known network"))
}

/// The address and value of the first output of a presigned Bitcoin spend.
fn first_btc_output(tx_hex: &str, network: Network) -> Result<(String, u64), AppError> {
    let tx = crate::noncustodial::exchange::btc_htlc::decode(tx_hex)
        .map_err(|_| corrupted("a presigned Bitcoin transaction cannot be read"))?;
    let o = tx
        .output
        .first()
        .ok_or_else(|| corrupted("a presigned Bitcoin transaction pays nobody"))?;
    let a = bitcoin::Address::from_script(
        &o.script_pubkey,
        crate::noncustodial::exchange::btc_htlc::btc_network(network),
    )
    .map_err(|_| corrupted("a presigned Bitcoin transaction pays no address"))?;
    Ok((a.to_string(), o.value.to_sat()))
}

/// The Buyer's rows. `btc_lock_address` and `hns_to` are shown so the user
/// can compare them with what the trade screen showed.
pub fn buyer_rows(
    c: &BuyerCommit,
    terms: &Terms,
    btc_lock_address: &str,
) -> Result<Value, AppError> {
    let network = network_of(terms)?;
    let paid = c
        .funding
        .output
        .first()
        .map(|o| o.value.to_sat())
        .ok_or_else(|| corrupted("the BTC funding pays nobody"))?;
    let refund_low = c
        .btc_refunds
        .first()
        .ok_or_else(|| corrupted("no BTC refund is signed"))?;
    let refund_high = c
        .btc_refunds
        .last()
        .ok_or_else(|| corrupted("no BTC refund is signed"))?;
    let (refund_to, _) = first_btc_output(&refund_low.tx_hex, network)?;
    let hns_fee = c
        .hns_claim
        .lock_value
        .checked_sub(c.hns_claim.to_value)
        .ok_or_else(|| corrupted("the HNS claim pays out more than the lock holds"))?;
    let rows = vec![
        row("You pay", sats_text(paid)),
        row("Bitcoin network fee", sats_text(c.funding_fee)),
        row(
            "You receive",
            format!(
                "{}, to {}",
                doos_to_hns_string(c.hns_claim.to_value),
                c.hns_claim.to_address
            ),
        ),
        row(
            "HNS network fee (taken from what you receive)",
            doos_to_hns_string(hns_fee),
        ),
        row(
            "Price",
            format!(
                "{} sats per HNS",
                price::format_sats_per_hns(terms.price_msat_per_hns)
            ),
        ),
        row("BTC lock", btc_lock_address.to_owned()),
        row(
            "When you get the HNS",
            format!(
                "Automatically, once the seller's HNS lock has {}, if that happens before {}",
                confirmations_text(terms.hns_confirmations),
                time_text(terms.reveal_deadline)
            ),
        ),
        row(
            "If the seller does not lock HNS",
            format!(
                "Your BTC comes back to {refund_to} after {}, less a fee of {} to {} sats",
                time_text(terms.btc_refund_time),
                refund_low.fee,
                refund_high.fee
            ),
        ),
        row("Seller's deadline", time_text(terms.seller_fund_deadline)),
        row("Trade", terms.id()),
    ];
    Ok(json!({ "rows": rows }))
}

/// The Seller's rows. `funding_fee` is the HNS funding's network fee from
/// the signed plan.
pub fn seller_rows(
    c: &SellerCommit,
    terms: &Terms,
    funding_fee: u64,
    hns_lock_address: &str,
) -> Result<Value, AppError> {
    let network = network_of(terms)?;
    let claim_low = c
        .btc_claims
        .first()
        .ok_or_else(|| corrupted("no BTC claim is signed"))?;
    let claim_high = c
        .btc_claims
        .last()
        .ok_or_else(|| corrupted("no BTC claim is signed"))?;
    let (to, receive) = first_btc_output(&claim_low.tx_hex, network)?;
    let refund = c
        .hns_refund
        .outputs
        .first()
        .ok_or_else(|| corrupted("the HNS refund pays nobody"))?;
    let program: [u8; 20] = refund
        .address
        .hash
        .as_slice()
        .try_into()
        .map_err(|_| corrupted("the HNS refund pays no address"))?;
    let refund_to = crate::noncustodial::address::encode_p2wpkh(network, &program)
        .map_err(|_| corrupted("the HNS refund pays no address"))?;
    let refund_fee = terms
        .hns_amount
        .checked_sub(refund.value)
        .ok_or_else(|| corrupted("the HNS refund pays out more than the lock holds"))?;
    let rows = vec![
        row("You lock", doos_to_hns_string(terms.hns_amount)),
        row("HNS network fee", doos_to_hns_string(funding_fee)),
        row(
            "You receive",
            format!(
                "{} to {to}, less a Bitcoin fee of {} to {} sats (shown: the lowest)",
                sats_text(receive),
                claim_low.fee,
                claim_high.fee
            ),
        ),
        row(
            "Price",
            format!(
                "{} sats per HNS",
                price::format_sats_per_hns(terms.price_msat_per_hns)
            ),
        ),
        row(
            "The buyer's BTC lock",
            format!(
                "{}, checked by proof of work",
                confirmations_text(terms.btc_confirmations)
            ),
        ),
        row("HNS lock", hns_lock_address.to_owned()),
        row(
            "When you get the BTC",
            "Automatically, as soon as the buyer takes the HNS: taking it reveals the secret that opens the BTC lock".into(),
        ),
        row(
            "Buyer's deadline to take the HNS",
            time_text(terms.reveal_deadline),
        ),
        row(
            "If the buyer does not take the HNS",
            format!(
                "Your HNS comes back to {refund_to} after {}, less a fee of {}",
                time_text(terms.hns_refund_opens()),
                doos_to_hns_string(refund_fee)
            ),
        ),
        row(
            "Keep Namehold running",
            format!(
                "Until the trade completes. If the buyer reveals, your BTC must be claimed before {}: the background sync and a watchtower do it, but not while both are off",
                time_text(terms.btc_refund_time)
            ),
        ),
        row("Trade", terms.id()),
    ];
    Ok(json!({ "rows": rows }))
}
