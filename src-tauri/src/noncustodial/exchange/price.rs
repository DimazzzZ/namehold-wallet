//! Prices and amounts. A price is in millisatoshis per whole HNS: 1 HNS =
//! 1 000 000 doos, 1 sat = 1 000 msat, so `btc = floor(doos × price /
//! 10^9)`. Integer arithmetic only, in u128, and both parties compute the
//! same number.

use crate::error::AppError;

pub const DOOS_PER_HNS: u128 = 1_000_000;
pub const MSAT_PER_SAT: u128 = 1_000;

/// The satoshis `hns_doos` cost at `msat_per_hns`, rounded down.
pub fn btc_for(hns_doos: u64, msat_per_hns: u64) -> Result<u64, AppError> {
    let v = u128::from(hns_doos) * u128::from(msat_per_hns) / (DOOS_PER_HNS * MSAT_PER_SAT);
    u64::try_from(v).map_err(|_| AppError::InvalidInput("the BTC amount overflows".into()))
}

/// Parse plain decimal digits with at most `decimals` places ("12", "12.5",
/// ".125") into integer units of 10^-`decimals`. No sign, exponent or
/// separator; never rounds. `bad` makes the refusal.
pub fn parse_fixed(text: &str, decimals: u32, bad: impl Fn() -> AppError) -> Result<u64, AppError> {
    let t = text.trim();
    let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
    if whole.is_empty() && frac.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
        || frac.len() > decimals as usize
    {
        return Err(bad());
    }
    let w: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| bad())?
    };
    let mut f: u64 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| bad())?
    };
    for _ in frac.len()..decimals as usize {
        f *= 10;
    }
    w.checked_mul(10u64.pow(decimals))
        .and_then(|x| x.checked_add(f))
        .ok_or_else(bad)
}

/// Parse a price typed as satoshis per HNS with up to three decimals
/// ("12", "12.5", "0.125") into msat per HNS. Refuses anything else, never
/// rounds.
pub fn parse_sats_per_hns(text: &str) -> Result<u64, AppError> {
    let v = parse_fixed(text, 3, || {
        AppError::InvalidInput(format!("'{text}' is not a price in sats per HNS"))
    })?;
    if v == 0 {
        return Err(AppError::InvalidInput("a price must be above zero".into()));
    }
    Ok(v)
}

/// msat per HNS as sats per HNS text, e.g. 12_500 → "12.5".
pub fn format_sats_per_hns(msat_per_hns: u64) -> String {
    let w = msat_per_hns / 1000;
    let f = msat_per_hns % 1000;
    if f == 0 {
        w.to_string()
    } else {
        format!("{w}.{}", format!("{f:03}").trim_end_matches('0'))
    }
}
