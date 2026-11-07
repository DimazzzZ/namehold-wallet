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
