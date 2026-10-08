//! Funding a Shakedex transaction from our own coins. The caller fixes input
//! 0 (the lead: a seller's lock coin, the TRANSFER out of a lock, our own
//! name coin or our lock coin) and the outputs; the fee is sized on the vsize
//! of the transaction as it will be signed, every witness included (R4).

use crate::error::AppError;
use crate::noncustodial::actions::{
    rebuild_unsigned, DraftPlan, PlanInput, PlanOutput, PlanResult, FINAL_SEQUENCE,
};
use crate::noncustodial::network::Network;
use crate::noncustodial::send::{SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::shakedex::purchase::SHAKEDEX_MIN_RATE_PER_BYTE;
use crate::noncustodial::tx::{sighash, Covenant};
use crate::noncustodial::types::doos_to_hns_string;

/// Witness item sizes of a signed P2WPKH input: signature + sighash byte,
/// compressed public key.
const DUMMY_SIG_AND_PUBKEY: [usize; 2] = [65, 33];

pub(super) fn plain(value: u64, address: String) -> PlanOutput {
    PlanOutput {
        value,
        address,
        covenant_type: 0,
        covenant_items_hex: vec![],
    }
}

pub(super) fn cov_out(value: u64, address: String, c: &Covenant) -> PlanOutput {
    PlanOutput {
        value,
        address,
        covenant_type: c.covenant_type,
        covenant_items_hex: c.items.iter().map(hex::encode).collect(),
    }
}

pub(super) fn own_input(c: &SpendableCoin) -> PlanInput {
    PlanInput {
        txid: c.txid.clone(),
        vout: c.vout,
        value: c.value,
        branch: c.branch,
        child_index: c.child_index,
        sighash_type: sighash::ALL,
        sequence: FINAL_SEQUENCE,
        foreign_witness_hex: None,
        lock_key_name: None,
    }
}

/// Exact vsize of `plan` once our inputs carry their witnesses; foreign
/// inputs already carry their finished witnesses.
fn signed_vsize(plan: &DraftPlan, network: Network) -> Result<u64, AppError> {
    let mut tx = rebuild_unsigned(plan, network)?;
    for (i, inp) in plan.inputs.iter().enumerate() {
        if inp.foreign_witness_hex.is_none() {
            tx.inputs[i].witness = DUMMY_SIG_AND_PUBKEY.iter().map(|n| vec![0u8; *n]).collect();
        }
    }
    Ok(tx.vsize())
}

fn overflow() -> AppError {
    AppError::InvalidInput("amounts overflow: this transaction cannot be built".into())
}

/// Sum money values, refusing (never wrapping or panicking) on overflow.
fn checked_sum(values: impl IntoIterator<Item = u64>) -> Result<u64, AppError> {
    values
        .into_iter()
        .try_fold(0u64, |acc, v| acc.checked_add(v))
        .ok_or_else(overflow)
}

/// Choose funding in the order given (callers pass load_spendable_coins
/// order, largest-first) until `spend + fee` is covered, sizing the fee on
/// the real transaction. `lead` is input 0, worth `lead_value`; the name
/// output at index 0 carries that value back. `before_change`/`after_change`
/// are the outputs on either side of the change slot.
#[allow(clippy::too_many_arguments)]
pub(super) fn fund(
    network: Network,
    account: u32,
    locktime: u32,
    lead: PlanInput,
    lead_value: u64,
    before_change: Vec<PlanOutput>,
    after_change: Vec<PlanOutput>,
    funding: &[SpendableCoin],
    change_address: &str,
    rate: u64,
    fixed_fee: Option<u64>,
) -> Result<PlanResult, AppError> {
    let rate = rate.max(SHAKEDEX_MIN_RATE_PER_BYTE);
    let out_total = checked_sum(
        before_change
            .iter()
            .chain(after_change.iter())
            .map(|o| o.value),
    )?;
    let have = checked_sum(funding.iter().map(|c| c.value))?;
    for taken in 0..=funding.len() {
        let mut inputs = vec![lead.clone()];
        inputs.extend(funding[..taken].iter().map(own_input));
        let in_total = checked_sum(
            std::iter::once(lead_value).chain(funding[..taken].iter().map(|c| c.value)),
        )?;
        for with_change in [true, false] {
            let mut outputs = before_change.clone();
            let change_index = with_change.then_some(outputs.len());
            if with_change {
                outputs.push(plain(0, change_address.to_owned()));
            }
            outputs.extend(after_change.iter().cloned());
            let mut plan = DraftPlan {
                version: 0,
                locktime,
                account,
                network: network.as_str().into(),
                inputs: inputs.clone(),
                outputs,
                change_output_index: change_index,
            };
            let vsize = signed_vsize(&plan, network)?;
            crate::noncustodial::send::check_standard_weight(vsize, plan.inputs.len() as u64)?;
            let fee = match fixed_fee {
                Some(f) => f,
                None => vsize.checked_mul(rate).ok_or_else(overflow)?,
            };
            let spend = out_total.checked_add(fee).ok_or_else(overflow)?;
            let Some(rest) = in_total.checked_sub(spend) else {
                continue;
            };
            if let Some(idx) = change_index {
                if rest < DUST_THRESHOLD {
                    continue;
                }
                plan.outputs[idx].value = rest;
            }
            let (fee, change) = if with_change {
                (fee, rest)
            } else {
                // rest < in_total - spend, so fee + rest <= in_total.
                (fee + rest, 0)
            };
            let tx = rebuild_unsigned(&plan, network)?;
            return Ok(PlanResult {
                unsigned_tx_hex: tx.to_hex(),
                txid: tx.txid(),
                fee,
                change,
                input_total: in_total,
                plan,
            });
        }
    }
    // The name output carries the lead's value back, so what we add is
    // everything else.
    let need = out_total.saturating_sub(lead_value);
    Err(AppError::InvalidInput(format!(
        "not enough HNS: need {} plus fees, have {}",
        doos_to_hns_string(need),
        doos_to_hns_string(have)
    )))
}
