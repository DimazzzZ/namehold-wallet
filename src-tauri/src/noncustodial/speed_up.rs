//! Speed up: a child that spends an unconfirmed transaction's change and pays
//! the fee for both (child pays for parent, spec 2026-10-21).
//!
//! hsd 8.0.0 has no replace-by-fee (`mempool/mempool.js` refuses a second
//! spend of a coin the mempool already spends), but its block template takes
//! a transaction at the better of its own rate and the rate of it with all its
//! mempool descendants (`mining/miner.js` `cmpRate`). A child that pays well
//! pulls its parent into a block.
//!
//! This module is the pure part: hsd's fee arithmetic, the mempool entry as
//! hsd sends it, the child's plan and fee, the refusals that rest on the
//! parent's entry and the target rate, and the signed child's re-check. The
//! refusals that read the database and the node are in
//! `commands::speed_up`.

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::noncustodial::actions::{rebuild_unsigned, DraftPlan};
use crate::noncustodial::network::Network;
use crate::noncustodial::send::{check_standard_weight, SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::shakedex::funding::{own_input, plain, signed_vsize};
use crate::noncustodial::types::doos_to_hns_string;

/// The draft action of a Speed up child.
pub const SPEED_UP_ACTION: &str = "speed_up";

/// hsd's `policy.MIN_RELAY`: the relay floor in doos per kB, the same on
/// every network (`protocol/policy.js`, `protocol/networks.js`).
pub const MIN_RELAY: u64 = 1000;

/// hsd's `policy.ABSURD_FEE_FACTOR`: the mempool refuses a transaction whose
/// fee is above its relay floor times this (`mempool/mempool.js`, on by
/// default).
pub const ABSURD_FEE_FACTOR: u64 = 10_000;

/// hsd's `policy.MEMPOOL_EXPIRY_TIME`: a transaction without a mempool parent
/// is forgotten this long after it entered the node's mempool, with every
/// spender of it.
pub const MEMPOOL_EXPIRY_SECS: i64 = 72 * 60 * 60;

/// How long before the parent's mempool expiry Speed up stops being offered:
/// a choice of the spec, not an hsd rule (R5).
pub const EXPIRY_MARGIN_SECS: i64 = 6 * 60 * 60;

// --- refusals (R1-R6, R9, R10, R14) -------------------------------------------

/// R1: the draft is not a sent, unconfirmed transaction.
pub const NOT_SENT_UNCONFIRMED: &str =
    "Only a transaction that was sent and is not confirmed yet can be sped up.";
/// R1: the node does not hold the parent (never had it, or mined it).
pub const NOT_IN_MEMPOOL: &str = "This transaction is not waiting in your node's mempool, so a child could not pull it into a block. If it was not mined, send it again instead.";
/// R1: SPV and Explorer modes have no mempool to read.
pub const NEEDS_NODE: &str =
    "Speeding up needs a full or remote node: SPV and Explorer modes cannot read the mempool.";
/// A watch-only profile cannot sign the child.
pub const WATCH_ONLY: &str =
    "This wallet is watch-only and cannot sign, so it cannot speed up a transaction.";
/// R2: no change output of ours.
pub const NO_CHANGE: &str = "This transaction has no change to spend, so it cannot be sped up.";
/// R2: the change is spent by another transaction of ours.
pub const CHANGE_SPENT: &str =
    "Its change is already spent by another of your transactions, which pays its own fee.";
/// R2: the change is reserved by another draft of ours.
pub const CHANGE_HELD: &str =
    "Its change is held by another of your drafts; send or discard that draft first.";
/// R2: the change is not in the wallet's coins yet.
pub const CHANGE_NOT_SYNCED: &str =
    "Its change is not in your wallet yet; refresh your wallet and try again.";
/// R3: a live child already spends or holds the change.
pub const ALREADY_SPED_UP: &str = "This transaction is already being sped up.";
/// R3: a child is not sped up itself.
pub const IS_CHILD: &str = "This transaction speeds up another one and cannot be sped up itself.";
/// R4: a purchase is sped up only while it is an Unconfirmed purchase.
pub const PURCHASE_NOT_UNCONFIRMED: &str = "Only an Unconfirmed purchase can be sped up.";
/// R5.
pub const EXPIRES_SOON: &str = "This transaction leaves your node's mempool in less than 6 hours; speeding it up now would not keep it there.";
/// R5: the parent waits on another unconfirmed transaction.
pub const HAS_ANCESTOR: &str = "This transaction waits on another unconfirmed transaction of the mempool; speed that one up, or wait for it to confirm.";
/// R6.
pub const BELOW_RELAY: &str = "This transaction pays less than the network's minimum relay fee; a child cannot pull it into a block.";
/// R9.
pub const RATE_NOT_HIGHER: &str =
    "Your transaction already pays this rate or more; choose a higher one.";
/// R10: the change left after the fee would be dust.
pub const CHANGE_TOO_SMALL: &str =
    "The change of this transaction is too small to pay for speeding it up";
/// R14: the parent left the mempool before the child was sent.
pub const PARENT_GONE: &str =
    "The transaction it speeds up is no longer waiting in your node's mempool; nothing was sent.";
/// R9: no estimate from the node, and the minimum rate it falls back to is
/// what every transaction the mempool took already pays.
pub const NO_ESTIMATE: &str =
    "Your node has no fee estimate yet, and this transaction already pays the minimum rate; choose a higher one.";

/// R10: the child's fee is past hsd's absurd-fee cap of `cap` doos.
fn absurd_fee(cap: u64) -> AppError {
    AppError::InvalidInput(format!(
        "This would pay more in fee than your node accepts from one transaction (about {}); choose a lower rate.",
        doos_to_hns_string(cap)
    ))
}

fn refuse(msg: &str) -> AppError {
    AppError::InvalidInput(msg.to_string())
}

/// R15: why a child is dropped once its parent died.
pub fn parent_died_reason(parent_txid: &str) -> String {
    format!(
        "The transaction it sped up ({parent_txid}) did not confirm, so this one can never confirm. Nothing was paid by either."
    )
}

// --- hsd's arithmetic ----------------------------------------------------------

/// hsd `policy.getMinFee(size)` at `MIN_RELAY`: `floor(rate * size / 1000)`,
/// at least the rate itself for a non-empty transaction.
pub fn get_min_fee(size: u64) -> u64 {
    if size == 0 {
        return 0;
    }
    let fee = (MIN_RELAY as u128 * size as u128 / 1000) as u64;
    if fee == 0 {
        MIN_RELAY
    } else {
        fee
    }
}

/// hsd `policy.getRate(size, fee)`: `floor(fee * 1000 / size)` doos per kB.
pub fn get_rate(size: u64, fee: u64) -> u64 {
    if size == 0 {
        return 0;
    }
    u64::try_from(fee as u128 * 1000 / size as u128).unwrap_or(u64::MAX)
}

/// A rate in doos per kB as the send form shows one, doos per vbyte.
pub fn rate_text(per_kb: u64) -> String {
    format!("{}.{:02} doos/vB", per_kb / 1000, (per_kb % 1000) / 10)
}

// --- the parent's entry ------------------------------------------------------

/// The parent's mempool entry as `getmempoolentry` reports it (hsd 8.0.0
/// `lib/node/rpc.js` `entryToJSON`). `fee` is the parent's own fee, which
/// hsd sends in HNS; `descendant_fees` is the package's, which it sends in
/// doos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolEntry {
    /// Sigop-adjusted virtual size (`entry.size`).
    pub size: u64,
    /// Own fee in doos.
    pub fee: u64,
    /// When it entered this node's mempool, unix seconds (`entry.time`).
    pub time: i64,
    /// The parent and every mempool descendant, vbytes (`entry.descSize`).
    pub descendant_size: u64,
    /// The same package's fees, doos (`entry.descFee`).
    pub descendant_fees: u64,
    /// Its unconfirmed ancestors in this mempool (`ancestorcount`, or the
    /// count of `depends` when that is higher).
    pub ancestors: u64,
}

/// Read hsd's `getmempoolentry` reply. A reply without one of the fields
/// read here is not hsd's answer: an error, never a guess.
pub fn parse_mempool_entry(v: &serde_json::Value) -> Result<MempoolEntry, AppError> {
    let missing = |key: &str| {
        AppError::Rpc(format!(
            "the node's mempool entry is not hsd's answer: no readable {key}"
        ))
    };
    let uint = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_u64())
            .ok_or_else(|| missing(key))
    };
    let fee_hns = v
        .get("fee")
        .and_then(|x| x.as_f64())
        .filter(|f| f.is_finite() && *f >= 0.0)
        .ok_or_else(|| missing("fee"))?;
    let entry = MempoolEntry {
        size: uint("size")?,
        fee: (fee_hns * 1_000_000.0).round() as u64,
        time: v
            .get("time")
            .and_then(|x| x.as_i64())
            .ok_or_else(|| missing("time"))?,
        descendant_size: uint("descendantsize")?,
        descendant_fees: uint("descendantfees")?,
        // hsd counts mempool ancestors (`countAncestors`) and lists the
        // mempool parents (`getDepends`); either one non-empty is a parent.
        ancestors: uint("ancestorcount")?.max(
            v.get("depends")
                .and_then(|d| d.as_array())
                .ok_or_else(|| missing("depends"))?
                .len() as u64,
        ),
    };
    if entry.size == 0 || entry.descendant_size < entry.size {
        return Err(missing("size"));
    }
    Ok(entry)
}

/// The parent's mempool deadline: `time` plus hsd's 72 hours.
pub fn expires_at(entry: &MempoolEntry) -> i64 {
    entry.time.saturating_add(MEMPOOL_EXPIRY_SECS)
}

/// The refusals that rest on the parent's entry alone: close to its mempool
/// expiry by the wallet's clock (R5), or below hsd's relay floor (R6).
pub fn refuse_parent_entry(entry: &MempoolEntry, now: i64) -> Result<(), AppError> {
    // hsd expires only a transaction without a mempool parent (its chain
    // goes with the oldest ancestor) and queues one only after its parents
    // (`mempool/mempool.js` `limitSize`, `mining/miner.js` `assemble`): the
    // deadline and the rate shown would not be this transaction's.
    if entry.ancestors > 0 {
        return Err(refuse(HAS_ANCESTOR));
    }
    if expires_at(entry).saturating_sub(now) < EXPIRY_MARGIN_SECS {
        return Err(refuse(EXPIRES_SOON));
    }
    if entry.fee < get_min_fee(entry.size) {
        return Err(refuse(BELOW_RELAY));
    }
    Ok(())
}

// --- the child ----------------------------------------------------------------

/// R8: the fee that lifts the package to `rate` doos per vbyte with a child
/// of `child_size` vbytes, and never less than the child's own relay floor
/// (hsd has no package acceptance).
pub fn child_fee(rate: u64, package_size: u64, package_fees: u64, child_size: u64) -> u64 {
    let want = rate.saturating_mul(package_size.saturating_add(child_size));
    want.saturating_sub(package_fees)
        .max(get_min_fee(child_size))
}

/// Where the target rate came from (R9), shown beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetSource {
    /// The node's `estimatesmartfee`.
    Estimate,
    /// The user's fee-rate setting.
    Setting,
    /// No estimate from the node: the relay-floor default.
    Fallback,
    /// The rate the user typed.
    Typed,
}

impl TargetSource {
    /// How the confirmation names it.
    pub fn text(self) -> &'static str {
        match self {
            Self::Estimate => "your node's estimate",
            Self::Setting => "your fee rate setting",
            Self::Fallback => "your node has no fee estimate yet; the minimum rate",
            Self::Typed => "the rate you chose",
        }
    }
}

/// The summary a Speed up child's draft stores. It reads as a plain
/// `TxSummary` too (the fee, the change, the txid), so lists and Activity show
/// it as any draft; the rest is the build's numbers, which the confirmation
/// and Activity read instead of a later node read (R8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedUpSummary {
    pub action: String,
    pub parent_txid: String,
    pub parent_draft_id: String,
    /// The parent is a Shakedex purchase (the confirmation says the price is
    /// unchanged).
    pub parent_is_purchase: bool,
    pub txid: String,
    /// Always 0: the child pays no one.
    pub send_total_doos: i64,
    pub fee_doos: i64,
    /// The child's one output, back to our change address.
    pub change_doos: i64,
    pub input_total_doos: i64,
    pub num_inputs: i64,
    /// Always `None`: the child pays no one.
    pub recipient_address: Option<String>,
    pub target_rate_per_vbyte: u64,
    pub target_source: TargetSource,
    pub parent_size: u64,
    pub parent_fee_doos: u64,
    /// `descendantsize` at build time.
    pub package_size: u64,
    /// `descendantfees` at build time.
    pub package_fees_doos: u64,
    pub child_size: u64,
    /// hsd `getRate` of the package before the child, doos per kB.
    pub package_rate_before: u64,
    /// The same with the child, doos per kB.
    pub package_rate_after: u64,
    /// The parent's mempool deadline, unix seconds (R5, R11).
    pub expires_at: i64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// What a child is built from.
pub struct ChildRequest<'a> {
    pub network: Network,
    pub account: u32,
    /// The parent's change, the child's one input.
    pub change: &'a SpendableCoin,
    /// The profile's change address, the child's one output.
    pub change_address: &'a str,
    pub entry: &'a MempoolEntry,
    /// The target, doos per vbyte.
    pub target_rate: u64,
    pub target_source: TargetSource,
    /// The wallet's clock, unix seconds.
    pub now: i64,
    pub parent_txid: &'a str,
    pub parent_draft_id: &'a str,
    pub parent_is_purchase: bool,
}

/// A planned child: its sign-ready plan, unsigned bytes and summary.
#[derive(Debug)]
pub struct ChildPlan {
    pub plan: DraftPlan,
    pub unsigned_tx_hex: String,
    pub txid: String,
    pub summary: SpeedUpSummary,
}

/// The child's plan with `fee`: one input, the parent's change, final
/// sequence and no lock time; one output, the rest, to our change (R7).
fn child_plan_with_fee(r: &ChildRequest<'_>, fee: u64) -> DraftPlan {
    DraftPlan {
        version: 0,
        locktime: 0,
        account: r.account,
        network: r.network.as_str().to_string(),
        inputs: vec![own_input(r.change)],
        outputs: vec![plain(
            r.change.value.saturating_sub(fee),
            r.change_address.to_string(),
        )],
        change_output_index: Some(0),
    }
}

/// Plan the child (R5-R10): the parent's entry refusals, the target above
/// the package rate, the fee on hsd's sizes, dust, the absurd-fee cap, and
/// the package rate after it checked again in hsd's arithmetic.
pub fn plan_child(r: &ChildRequest<'_>) -> Result<ChildPlan, AppError> {
    refuse_parent_entry(r.entry, r.now)?;
    let target_per_kb = r.target_rate.saturating_mul(1000);
    let rate_before = get_rate(r.entry.descendant_size, r.entry.descendant_fees);
    if target_per_kb <= rate_before {
        return Err(refuse(RATE_NOT_HIGHER));
    }
    // The child's size does not depend on its output's value: measure it on
    // the plan with the witness it will carry.
    let child_size = signed_vsize(&child_plan_with_fee(r, 0), r.network)?;
    check_standard_weight(child_size, 1)?;
    let fee = child_fee(
        r.target_rate,
        r.entry.descendant_size,
        r.entry.descendant_fees,
        child_size,
    );
    let cap = get_min_fee(child_size).saturating_mul(ABSURD_FEE_FACTOR);
    if fee > cap {
        return Err(absurd_fee(cap));
    }
    if r.change.value < fee.saturating_add(DUST_THRESHOLD) {
        return Err(refuse(CHANGE_TOO_SMALL));
    }
    let package_size = r.entry.descendant_size + child_size;
    let rate_after = get_rate(package_size, r.entry.descendant_fees.saturating_add(fee));
    if rate_after < target_per_kb {
        return Err(AppError::InvalidInput(format!(
            "the fee would leave the package at {}, below the target {}; nothing was built",
            rate_text(rate_after),
            rate_text(target_per_kb)
        )));
    }
    let plan = child_plan_with_fee(r, fee);
    let unsigned = rebuild_unsigned(&plan, r.network)?;
    let txid = unsigned.txid();
    let summary = SpeedUpSummary {
        action: SPEED_UP_ACTION.to_string(),
        parent_txid: r.parent_txid.to_string(),
        parent_draft_id: r.parent_draft_id.to_string(),
        parent_is_purchase: r.parent_is_purchase,
        txid: txid.clone(),
        send_total_doos: 0,
        fee_doos: fee as i64,
        change_doos: plan.outputs[0].value as i64,
        input_total_doos: r.change.value as i64,
        num_inputs: 1,
        recipient_address: None,
        target_rate_per_vbyte: r.target_rate,
        target_source: r.target_source,
        parent_size: r.entry.size,
        parent_fee_doos: r.entry.fee,
        package_size: r.entry.descendant_size,
        package_fees_doos: r.entry.descendant_fees,
        child_size,
        package_rate_before: rate_before,
        package_rate_after: rate_after,
        expires_at: expires_at(r.entry),
        warnings: Vec::new(),
    };
    Ok(ChildPlan {
        plan,
        unsigned_tx_hex: unsigned.to_hex(),
        txid,
        summary,
    })
}

/// R8, R10 on the signed child: one input and one output, the fee the summary
/// says, a size no larger than the one it was priced on, the package at the
/// target in hsd's arithmetic, no dust and no fee past the absurd cap.
pub fn recheck_signed_child(signed_hex: &str, s: &SpeedUpSummary) -> Result<(), AppError> {
    let tx = hex::decode(signed_hex)
        .ok()
        .and_then(|b| crate::noncustodial::tx::Transaction::decode(&b).ok())
        .ok_or_else(|| AppError::Other("the signed child does not parse".into()))?;
    if tx.inputs.len() != 1 || tx.outputs.len() != 1 || tx.txid() != s.txid {
        return Err(AppError::Other(
            "the signed child is not the one reviewed; nothing was sent".into(),
        ));
    }
    let out = tx.outputs[0].value;
    let fee = u64::try_from(s.input_total_doos)
        .ok()
        .and_then(|input| input.checked_sub(out))
        .filter(|fee| i64::try_from(*fee).ok() == Some(s.fee_doos))
        .ok_or_else(|| {
            AppError::Other(
                "the signed child's fee is not the one reviewed; nothing was sent".into(),
            )
        })?;
    let size = tx.vsize();
    if size > s.child_size {
        return Err(refuse(
            "the signed child is larger than the one reviewed, so it would pay less than the target rate; nothing was sent",
        ));
    }
    let cap = get_min_fee(size).saturating_mul(ABSURD_FEE_FACTOR);
    if fee > cap {
        return Err(absurd_fee(cap));
    }
    if out < DUST_THRESHOLD {
        return Err(refuse(CHANGE_TOO_SMALL));
    }
    let after = get_rate(
        s.package_size.saturating_add(size),
        s.package_fees_doos.saturating_add(fee),
    );
    if after < s.target_rate_per_vbyte.saturating_mul(1000) {
        return Err(refuse(
            "the signed child would leave the package below the target rate; nothing was sent",
        ));
    }
    Ok(())
}

/// The output of a parent draft that is its change (R2): a plan's
/// `change_output_index` when that output exists and carries no covenant; a
/// plain send, which stores its build parameters instead of a plan, has its
/// change at output 1 after the recipient (`send::build_send`) when it has
/// any. `None`: no change of ours to spend.
pub fn parent_change_index(
    action: &str,
    signing_inputs_json: &str,
    summary_json: &str,
) -> Result<Option<usize>, AppError> {
    if action == "send_hns" {
        let summary: crate::noncustodial::types::TxSummary = serde_json::from_str(summary_json)
            .map_err(|e| AppError::Other(format!("corrupted draft: unreadable summary: {e}")))?;
        return Ok((summary.change_doos > 0).then_some(1));
    }
    let plan: DraftPlan = serde_json::from_str(signing_inputs_json)
        .map_err(|e| AppError::Other(format!("corrupted draft: unreadable signing plan: {e}")))?;
    Ok(plan
        .change_output_index
        .filter(|i| plan.outputs.get(*i).is_some_and(|o| o.covenant_type == 0)))
}
