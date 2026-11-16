//! The BTC account's coins, from a Bitcoin source, by scanning its receive
//! and change addresses up to the last handed out plus the gap limit.

use crate::error::AppError;
use crate::exchange_io::btc_source::BtcSource;
use crate::exchange_io::store::Account;
use crate::noncustodial::exchange::btc_account::{self, BtcCoin, GAP_LIMIT};
use crate::noncustodial::network::Network;

pub async fn coins(
    src: &dyn BtcSource,
    acct: &Account,
    network: Network,
) -> Result<Vec<BtcCoin>, AppError> {
    let mut out = Vec::new();
    for (branch, next) in [
        (btc_account::BRANCH_RECEIVE, acct.next_btc_receive),
        (btc_account::BRANCH_CHANGE, acct.next_btc_change),
    ] {
        for index in 0..next.saturating_add(GAP_LIMIT) {
            let addr = btc_account::address_at(&acct.btc_xpub, network, branch, index)?;
            for u in src.utxos(&addr).await? {
                out.push(BtcCoin {
                    txid: u.outpoint.txid.to_string(),
                    vout: u.outpoint.vout,
                    value: u.value,
                    branch,
                    index,
                    height: u.height,
                });
            }
        }
    }
    Ok(out)
}

/// The BTC account's coins from the first source that answers for every
/// scanned address, in the order the settings list them. One source refusing
/// (a public server's rate limit, an outage) no longer leaves the account
/// unread while another is set; the error names every source that failed.
pub async fn coins_from_any(
    srcs: &[Box<dyn BtcSource>],
    acct: &Account,
    network: Network,
) -> Result<Vec<BtcCoin>, AppError> {
    let mut errors: Vec<AppError> = Vec::new();
    for src in srcs {
        match coins(src.as_ref(), acct, network).await {
            Ok(c) => return Ok(c),
            Err(e) => errors.push(e),
        }
    }
    if errors.len() > 1 {
        let all: Vec<String> = errors.iter().map(ToString::to_string).collect();
        return Err(AppError::Other(format!(
            "no Bitcoin source could be read: {}",
            all.join("; ")
        )));
    }
    Err(errors.pop().unwrap_or_else(|| {
        AppError::InvalidInput("no Bitcoin source is set: add one in Exchange settings".into())
    }))
}
