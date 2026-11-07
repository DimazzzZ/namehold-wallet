//! What the secure window shows at a Commit (spec R16): every amount and
//! address is read from the transactions about to be signed and sent, never
//! from the terms alone, and every time from the terms the scripts commit
//! to.

use serde_json::{json, Value};

use crate::noncustodial::exchange::commit::{BuyerCommit, SellerCommit};
use crate::noncustodial::exchange::price;
use crate::noncustodial::exchange::terms::Terms;
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
        .unwrap_or_else(|| format!("{unix} (not a date)"))
}

fn row(label: &str, value: String) -> Value {
    json!({ "label": label, "value": value })
}

/// The Buyer's rows. `btc_lock_address` and `hns_to` are shown so the user
/// can compare them with what the trade screen showed.
pub fn buyer_rows(c: &BuyerCommit, terms: &Terms, btc_lock_address: &str) -> Value {
    let paid = c
        .funding
        .output
        .first()
        .map(|o| o.value.to_sat())
        .unwrap_or(0);
    let refund_low = c.btc_refunds.first().map(|r| r.fee).unwrap_or(0);
    let refund_high = c.btc_refunds.last().map(|r| r.fee).unwrap_or(0);
    let refund_to = c
        .btc_refunds
        .first()
        .and_then(|r| crate::noncustodial::exchange::btc_htlc::decode(&r.tx_hex).ok())
        .and_then(|t| t.output.first().cloned())
        .and_then(|o| {
            bitcoin::Address::from_script(
                &o.script_pubkey,
                crate::noncustodial::exchange::btc_htlc::btc_network(
                    terms
                        .network()
                        .unwrap_or(crate::noncustodial::network::Network::Main),
                ),
            )
            .ok()
        })
        .map(|a| a.to_string())
        .unwrap_or_else(|| "(unreadable)".into());
    let rows = vec![
        row("You pay", sats_text(paid)),
        row("Bitcoin network fee", sats_text(c.funding_fee)),
        row(
            "You receive",
            format!(
                "{} HNS, to {}",
                doos_to_hns_string(c.hns_claim.to_value),
                c.hns_claim.to_address
            ),
        ),
        row(
            "HNS network fee (taken from what you receive)",
            format!("{} HNS", doos_to_hns_string(c.hns_claim.lock_value - c.hns_claim.to_value)),
        ),
        row("Price", format!("{} sats per HNS", price::format_sats_per_hns(terms.price_msat_per_hns))),
        row("BTC lock", btc_lock_address.to_owned()),
        row(
            "When you get the HNS",
            format!(
                "Automatically, once the seller's HNS lock has {} confirmations, if that happens before {}",
                terms.hns_confirmations,
                time_text(terms.reveal_deadline)
            ),
        ),
        row(
            "If the seller does not lock HNS",
            format!(
                "Your BTC comes back to {refund_to} after {}, less a fee of {} to {} sats",
                time_text(terms.btc_refund_time),
                refund_low,
                refund_high
            ),
        ),
        row("Seller's deadline", time_text(terms.seller_fund_deadline)),
        row("Trade", terms.id()),
    ];
    json!({ "rows": rows })
}

/// The Seller's rows. `funding_fee` is the HNS funding's network fee from
/// the signed plan.
pub fn seller_rows(
    c: &SellerCommit,
    terms: &Terms,
    funding_fee: u64,
    hns_lock_address: &str,
) -> Value {
    let claim_low = c.btc_claims.first().map(|r| r.fee).unwrap_or(0);
    let claim_high = c.btc_claims.last().map(|r| r.fee).unwrap_or(0);
    let (to, receive) = c
        .btc_claims
        .first()
        .and_then(|r| crate::noncustodial::exchange::btc_htlc::decode(&r.tx_hex).ok())
        .and_then(|t| t.output.first().cloned())
        .map(|o| {
            let a = bitcoin::Address::from_script(
                &o.script_pubkey,
                crate::noncustodial::exchange::btc_htlc::btc_network(
                    terms
                        .network()
                        .unwrap_or(crate::noncustodial::network::Network::Main),
                ),
            )
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "(unreadable)".into());
            (a, o.value.to_sat())
        })
        .unwrap_or_else(|| ("(unreadable)".into(), 0));
    let refund_to = c
        .hns_refund
        .outputs
        .first()
        .and_then(|o| {
            let program: [u8; 20] = o.address.hash.as_slice().try_into().ok()?;
            crate::noncustodial::address::encode_p2wpkh(terms.network().ok()?, &program).ok()
        })
        .unwrap_or_else(|| "(unreadable)".into());
    let refund_fee = terms.hns_amount - c.hns_refund.outputs.first().map(|o| o.value).unwrap_or(0);
    let rows = vec![
        row("You lock", format!("{} HNS", doos_to_hns_string(terms.hns_amount))),
        row("HNS network fee", format!("{} HNS", doos_to_hns_string(funding_fee))),
        row(
            "You receive",
            format!(
                "{} to {to}, less a Bitcoin fee of {claim_low} to {claim_high} sats (shown: the lowest)",
                sats_text(receive)
            ),
        ),
        row("Price", format!("{} sats per HNS", price::format_sats_per_hns(terms.price_msat_per_hns))),
        row("The buyer's BTC lock", format!("{} confirmations, checked by proof of work", terms.btc_confirmations)),
        row("HNS lock", hns_lock_address.to_owned()),
        row(
            "When you get the BTC",
            "Automatically, as soon as the buyer takes the HNS: taking it reveals the secret that opens the BTC lock".into(),
        ),
        row(
            "If the buyer does not take the HNS",
            format!(
                "Your HNS comes back to {refund_to} after {}, less a fee of {} HNS",
                time_text(terms.hns_refund_time),
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
    json!({ "rows": rows })
}
