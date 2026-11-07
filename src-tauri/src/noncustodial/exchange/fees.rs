//! Fees of presigned spends (spec §3.5, R12).
//!
//! - HNS: hsd 8.0.0 has no replace-by-fee, so an HNS spend is signed once at
//!   a rate that will be mined: the node's rate × the policy multiplier, at
//!   least the policy floor.
//! - Bitcoin: a ladder of rungs at rising rates. Each rung replaces the one
//!   below (BIP 125: same input, higher fee and rate). No rung pays more than
//!   the policy's share of the coin; a spend whose first rung cannot fit is
//!   refused before anything is signed.

use crate::error::AppError;
use crate::noncustodial::exchange::policy::Policy;

pub fn hns_presign_rate(node_rate_per_vbyte: u64, p: &Policy) -> u64 {
    node_rate_per_vbyte
        .saturating_mul(p.hns_rate_multiplier)
        .max(p.hns_min_rate)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rung {
    pub rate: u64,
    pub fee: u64,
}

/// The ladder for a spend of `vsize` vbytes of a coin worth `coin_value`,
/// from the source's rate `base_rate` (sat/vbyte).
pub fn btc_ladder(
    base_rate: u64,
    vsize: u64,
    coin_value: u64,
    p: &Policy,
) -> Result<Vec<Rung>, AppError> {
    let base = base_rate.max(p.btc_min_rate);
    let cap = u128::from(coin_value) * u128::from(p.max_fee_percent) / 100;
    let rungs: Vec<Rung> = p
        .ladder_multipliers
        .iter()
        .filter_map(|m| {
            let rate = base.checked_mul(*m)?;
            let fee = rate.checked_mul(vsize)?;
            (u128::from(fee) <= cap).then_some(Rung { rate, fee })
        })
        .collect();
    if rungs.first().map(|r| r.rate) != Some(base) {
        return Err(AppError::InvalidInput(format!(
            "this trade is too small for the Bitcoin fees: a spend would cost more than {}% of it",
            p.max_fee_percent
        )));
    }
    Ok(rungs)
}

/// Which rung to have in the mempool now: the first at the first send, one
/// more every `ladder_escalate_after` seconds since, and the top once fewer
/// than `ladder_top_before` seconds are left before `deadline` (when the
/// competing path opens; `None` when nothing competes).
pub fn rung_now(
    rungs: usize,
    first_sent_at: Option<u64>,
    now: u64,
    deadline: Option<u64>,
    p: &Policy,
) -> usize {
    if rungs == 0 {
        return 0;
    }
    let top = rungs - 1;
    if deadline.is_some_and(|d| now.saturating_add(p.ladder_top_before) >= d) {
        return top;
    }
    let Some(first) = first_sent_at else {
        return 0;
    };
    let steps = now.saturating_sub(first) / p.ladder_escalate_after.max(1);
    usize::try_from(steps).unwrap_or(top).min(top)
}
