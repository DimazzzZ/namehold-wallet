//! What the secure window shows before a name action is signed, read from the
//! plan that is signed rather than from a summary written beside it.
//!
//! The window is the one place a user can catch a swapped draft: a
//! compromised main webview can ask to sign any draft, but it cannot draw in
//! the secure window. So every row here is computed from the
//! `DraftPlan` itself: the names come from the covenants' name hashes (the
//! summary's spelling of a name is shown only once it hashes to one of them),
//! a TRANSFER's recipient from the address its covenant commits to, a
//! FINALIZE's from the output that receives the name, a bid's value from the
//! wallet's own record of it, checked against the blind the BID commits to,
//! and the fee and txid from the plan's own inputs and outputs.
//!
//! Anything that cannot be shown truthfully refuses the draft ("corrupted
//! draft" or "cannot show"), never a row with a guessed or zero value.

use std::collections::BTreeMap;

use crate::error::AppError;
use crate::noncustodial::actions::{rebuild_unsigned, DraftPlan, PlanOutput};
use crate::noncustodial::address;
use crate::noncustodial::bids::compute_blind;
use crate::noncustodial::names::hash_name;
use crate::noncustodial::network::Network;
use crate::noncustodial::sync::{
    COV_BID, COV_FINALIZE, COV_NONE, COV_REDEEM, COV_REVEAL, COV_REVOKE, COV_TRANSFER,
};
use crate::noncustodial::types::doos_to_hns_string;

/// One row of the secure window: a label and the value shown beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub value: String,
}

impl Row {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Row {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// The wallet's own record of a bid: its true value and the nonce whose
/// blind the BID covenant carries.
#[derive(Debug, Clone, Copy)]
pub struct BidOnRecord {
    pub value: u64,
    pub nonce: [u8; 32],
}

/// What the draft says about itself besides its plan, as stored in its
/// summary. Only `names` and `recipient` are cross-checked against the plan;
/// `warnings` are shown as they are.
pub struct DraftClaims<'a> {
    pub action: &'a str,
    pub names: &'a [String],
    pub recipient: Option<&'a str>,
    pub summary_txid: Option<&'a str>,
    pub warnings: &'a [String],
}

/// Said in the window for every BID: the lockup is lost if the bid is never
/// revealed (hsd burns an unrevealed BID's value once the reveal period ends).
pub const BID_REVEAL_WARNING: &str =
    "Reveal this bid during the reveal period, or the whole locked amount is lost.";

/// Said in the window for a REVOKE.
pub fn revoke_warning(name: &str) -> String {
    format!(
        "Revoking burns {name} for good: nobody, you included, can use or move it until it \
         expires and is auctioned again. This cannot be undone."
    )
}

/// The human name of a draft's action, as the window titles it.
pub fn action_label(action: &str) -> String {
    match action {
        "open" => "Open an auction".into(),
        "bid" | "batch-bid" => "Bid".into(),
        "reveal" | "batch-reveal" => "Reveal bid".into(),
        "redeem" | "batch-redeem" => "Reclaim a losing bid".into(),
        "register" => "Register name".into(),
        "update" => "Update DNS records".into(),
        "renew" | "batch-renew" => "Renew".into(),
        "transfer" | "batch-transfer" => "Transfer to another wallet".into(),
        "finalize" | "batch-finalize" => "Finalize transfer".into(),
        "cancel" => "Cancel transfer".into(),
        "revoke" => "Revoke name (burns it)".into(),
        other => format!("Name action: {other}"),
    }
}

fn corrupted(why: &str) -> AppError {
    AppError::Other(format!("corrupted draft: {why}; nothing was signed"))
}

/// The address a TRANSFER covenant commits the name to (items 2 and 3:
/// address version and hash), in the network's spelling.
pub fn transfer_recipient(network: Network, out: &PlanOutput) -> Result<String, AppError> {
    let items = &out.covenant_items_hex;
    if items.len() != 4 {
        return Err(corrupted("a TRANSFER without its four covenant items"));
    }
    let version = hex::decode(&items[2]).map_err(|_| corrupted("a TRANSFER version not hex"))?;
    let program = hex::decode(&items[3]).map_err(|_| corrupted("a TRANSFER hash not hex"))?;
    if version.as_slice() != [0u8] {
        return Err(AppError::Other(
            "cannot show the recipient: the transfer commits to an address version this \
             wallet does not display; nothing was signed"
                .into(),
        ));
    }
    match program.len() {
        20 => address::encode_p2wpkh(network, program.as_slice().try_into().expect("20 bytes")),
        32 => address::encode_p2wsh(network, program.as_slice().try_into().expect("32 bytes")),
        _ => Err(corrupted("a TRANSFER hash of a length no address has")),
    }
}

/// The plan's fee: what its inputs bring in minus what its outputs carry.
pub fn plan_fee(plan: &DraftPlan) -> Result<u64, AppError> {
    let ins = plan
        .inputs
        .iter()
        .try_fold(0u64, |s, i| s.checked_add(i.value))
        .ok_or_else(|| corrupted("input values overflow"))?;
    let outs = plan
        .outputs
        .iter()
        .try_fold(0u64, |s, o| s.checked_add(o.value))
        .ok_or_else(|| corrupted("output values overflow"))?;
    ins.checked_sub(outs)
        .ok_or_else(|| corrupted("the outputs carry more than the inputs"))
}

/// One label per distinct value, or one row per name when the names differ:
/// "To" for a single recipient of every name, "To (name)" otherwise.
fn grouped_rows(label: &str, per_name: &[(String, String)]) -> Vec<Row> {
    let mut distinct: Vec<&str> = per_name.iter().map(|(_, v)| v.as_str()).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() == 1 {
        return vec![Row::new(label, distinct[0])];
    }
    per_name
        .iter()
        .map(|(n, v)| Row::new(format!("{label} ({n})"), v.clone()))
        .collect()
}

/// The rows the secure window shows for a name action's plan.
///
/// `bids` returns the wallet's record of the bid whose blind is the given hex
/// (`bid_commitments`); a BID whose record is missing, or whose record does
/// not hash to the blind the plan commits to, refuses the draft.
pub fn covenant_rows(
    claims: &DraftClaims<'_>,
    plan: &DraftPlan,
    bids: &dyn Fn(&str) -> Option<BidOnRecord>,
) -> Result<Vec<Row>, AppError> {
    let network = Network::from_str_opt(&plan.network)
        .ok_or_else(|| corrupted(&format!("unknown network '{}'", plan.network)))?;

    // Names: every covenant output names a hash; each must be one of the
    // names the draft shows, and each name shown must be acted on.
    let mut by_hash: BTreeMap<String, String> = BTreeMap::new();
    for n in claims.names {
        let h = hash_name(n).map_err(|_| corrupted(&format!("an invalid name '{n}'")))?;
        by_hash.insert(hex::encode(h), n.clone());
    }
    let mut acted: Vec<String> = Vec::new();
    let mut name_of_output: Vec<Option<String>> = Vec::with_capacity(plan.outputs.len());
    for o in &plan.outputs {
        if o.covenant_type == COV_NONE {
            name_of_output.push(None);
            continue;
        }
        let hash = o
            .covenant_items_hex
            .first()
            .ok_or_else(|| corrupted("a name covenant without its name hash"))?
            .to_ascii_lowercase();
        let name = by_hash
            .get(&hash)
            .ok_or_else(|| corrupted("the transaction acts on a name it does not show"))?;
        if !acted.contains(name) {
            acted.push(name.clone());
        }
        name_of_output.push(Some(name.clone()));
    }
    if acted.is_empty() {
        return Err(corrupted("a name action whose transaction acts on no name"));
    }
    if let Some(missing) = by_hash.values().find(|n| !acted.contains(n)) {
        return Err(corrupted(&format!(
            "'{missing}' is shown but the transaction does not act on it"
        )));
    }

    let mut rows = vec![Row::new("Action", action_label(claims.action))];
    if acted.len() == 1 {
        rows.push(Row::new("Name", acted[0].clone()));
    } else {
        rows.push(Row::new(
            format!("Names ({})", acted.len()),
            acted.join(", "),
        ));
    }

    let mut transfers: Vec<(String, String)> = Vec::new();
    let mut finalizes: Vec<(String, String)> = Vec::new();
    let mut bid_rows: Vec<(String, u64)> = Vec::new();
    let mut locked: u64 = 0;
    let mut revealed: u64 = 0;
    let mut reclaimed: u64 = 0;
    let mut revoked: Vec<String> = Vec::new();
    let mut pays: Vec<Row> = Vec::new();
    for (i, (o, name)) in plan.outputs.iter().zip(&name_of_output).enumerate() {
        let Some(name) = name else {
            if Some(i) != plan.change_output_index {
                pays.push(Row::new(
                    "Pays",
                    format!("{} to {}", doos_to_hns_string(o.value), o.address),
                ));
            }
            continue;
        };
        match o.covenant_type {
            COV_TRANSFER => transfers.push((name.clone(), transfer_recipient(network, o)?)),
            COV_FINALIZE => finalizes.push((name.clone(), o.address.clone())),
            COV_BID => {
                let blind_hex = o
                    .covenant_items_hex
                    .get(3)
                    .ok_or_else(|| corrupted("a BID without its blind"))?
                    .to_ascii_lowercase();
                let bid = bids(&blind_hex).ok_or_else(|| {
                    AppError::Other(format!(
                        "cannot show the bid on {name}: the wallet has no record of its value; \
                         nothing was signed"
                    ))
                })?;
                if hex::encode(compute_blind(bid.value, &bid.nonce)) != blind_hex {
                    return Err(corrupted(&format!(
                        "the recorded bid on {name} is not the one the transaction commits to"
                    )));
                }
                if bid.value > o.value {
                    return Err(corrupted(&format!("the bid on {name} exceeds its lockup")));
                }
                bid_rows.push((name.clone(), bid.value));
                locked = locked.saturating_add(o.value);
            }
            COV_REVEAL => revealed = revealed.saturating_add(o.value),
            COV_REDEEM => reclaimed = reclaimed.saturating_add(o.value),
            COV_REVOKE => revoked.push(name.clone()),
            _ => {}
        }
    }

    if !transfers.is_empty() {
        if let Some(r) = claims.recipient {
            if transfers.iter().any(|(_, a)| a != r) {
                return Err(corrupted(
                    "the recipient shown is not the one the transfer commits to",
                ));
            }
        }
        rows.extend(grouped_rows("To", &transfers));
    }
    if !finalizes.is_empty() {
        rows.extend(grouped_rows("Name goes to", &finalizes));
    }
    if !bid_rows.is_empty() {
        if bid_rows.len() == 1 {
            rows.push(Row::new("Bid", doos_to_hns_string(bid_rows[0].1)));
        } else {
            let list = bid_rows
                .iter()
                .map(|(n, v)| format!("{n} {}", doos_to_hns_string(*v)))
                .collect::<Vec<_>>()
                .join(", ");
            rows.push(Row::new(format!("Bids ({})", bid_rows.len()), list));
        }
        rows.push(Row::new("Locked until reveal", doos_to_hns_string(locked)));
    }
    if revealed > 0 {
        rows.push(Row::new("Reveals", doos_to_hns_string(revealed)));
    }
    if reclaimed > 0 {
        rows.push(Row::new("Reclaims", doos_to_hns_string(reclaimed)));
    }
    rows.extend(pays);

    let fee = plan_fee(plan)?;
    rows.push(Row::new("Fee", doos_to_hns_string(fee)));
    let txid = rebuild_unsigned(plan, network)?.txid();
    if let Some(s) = claims.summary_txid {
        if s != txid {
            return Err(corrupted("the summary is of another transaction"));
        }
    }
    rows.push(Row::new("Txid", txid));

    if !bid_rows.is_empty() {
        rows.push(Row::new("Warning", BID_REVEAL_WARNING));
    }
    for n in &revoked {
        rows.push(Row::new("Warning", revoke_warning(n)));
    }
    for w in claims.warnings {
        rows.push(Row::new("Warning", w.clone()));
    }
    Ok(rows)
}
