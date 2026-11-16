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

type Answer = Option<(std::time::Instant, Result<Vec<BtcCoin>, String>)>;

/// One read per account at a time. A read that starts while another of the
/// same account is under way waits for it and takes its answer, instead of
/// asking every scanned address again: each read is one request per address
/// (40 for a fresh account), and a public source rate-limits overlapping ones.
///
/// `sources` names the configured sources (their URLs), so answers from
/// different sources are never shared.
pub async fn coins_shared(
    srcs: &[Box<dyn BtcSource>],
    sources: &str,
    acct: &Account,
    network: Network,
) -> Result<Vec<BtcCoin>, AppError> {
    static READS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<Answer>>>,
        >,
    > = std::sync::OnceLock::new();
    let arrived = std::time::Instant::now();
    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        sources,
        acct.profile_id,
        network.as_str(),
        acct.account,
        acct.next_btc_receive,
        acct.next_btc_change
    );
    let slot = {
        let mut m = READS
            .get_or_init(Default::default)
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        m.entry(key).or_default().clone()
    };
    let mut last = slot.lock().await;
    if let Some((done, answer)) = last.as_ref() {
        // Finished after this call arrived: that read was under way, so its
        // answer is as fresh as a new one.
        if *done >= arrived {
            return answer.clone().map_err(AppError::Other);
        }
    }
    let r = coins_from_any(srcs, acct, network).await;
    *last = Some((
        std::time::Instant::now(),
        r.as_ref().map(Clone::clone).map_err(ToString::to_string),
    ));
    r
}
