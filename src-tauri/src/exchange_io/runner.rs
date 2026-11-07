//! One engine step with its IO (spec R13): read both chains, ask the engine,
//! send what it returns, record when each kind was first sent. The daemon,
//! the app and the watchtower all call [`run_step`]; none of them signs.
//!
//! Sending is honest (spec 2026-10-07): hsd answers `sendrawtransaction`
//! with the txid whatever its mempool does, so an HNS send is reported sent
//! only once the node has the transaction (`taken_by_node_with_client`).
//! A Bitcoin send goes to every source, and is sent once one accepts it.

use crate::error::AppError;
use crate::exchange_io::btc_source::BtcSource;
use crate::exchange_io::{btc_observe, hns_observe};
use crate::noncustodial::exchange::engine::{self, Action, Holdings, SpendKind, Step};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::tx_evidence::{taken_by_node_with_client, Taken};

/// What a step did, for the trade's record and the screen.
#[derive(Debug, Default)]
pub struct StepReport {
    pub step: Option<Step>,
    /// Why a chain could not be read, or a send failed.
    pub errors: Vec<String>,
    /// Kinds sent and taken in this step.
    pub sent: Vec<SpendKind>,
}

/// Send one HNS transaction; `Ok(true)` when the node took it.
pub async fn send_hns(node: &dyn NodeRpc, tx_hex: &str) -> Result<bool, AppError> {
    let raw = hex::decode(tx_hex).map_err(|_| AppError::InvalidInput("tx is not hex".into()))?;
    let txid = crate::noncustodial::tx::Transaction::decode(&raw)?.txid();
    // Already known (mempool or chain): nothing to send.
    if node.get_raw_transaction(&txid).await.is_ok() {
        return Ok(true);
    }
    node.send_raw_transaction(tx_hex).await?;
    Ok(matches!(
        taken_by_node_with_client(node, tx_hex, &txid).await,
        Taken::Yes | Taken::Mined(_)
    ))
}

/// Send one Bitcoin transaction to every source; `Ok` once one accepts it.
pub async fn send_btc(sources: &[Box<dyn BtcSource>], tx_hex: &str) -> Result<(), AppError> {
    let mut last = None;
    let mut ok = false;
    for s in sources {
        match s.broadcast(tx_hex).await {
            Ok(_) => ok = true,
            Err(e) => last = Some(e),
        }
    }
    if ok {
        Ok(())
    } else {
        Err(last
            .unwrap_or_else(|| AppError::InvalidInput("no Bitcoin source is configured".into())))
    }
}

/// Read, decide, send. `h.sent` is updated for every kind the chains took;
/// the caller stores `h` and the report.
pub async fn run_step(
    node: Option<&dyn NodeRpc>,
    sources: &[Box<dyn BtcSource>],
    h: &mut Holdings,
    now: u64,
    p: &Policy,
) -> StepReport {
    let mut report = StepReport::default();
    let hns = match node {
        Some(n) => match hns_observe::observe(n, &h.terms, h.hns_lock.as_ref()).await {
            Ok(v) => Some(v),
            Err(e) => {
                report.errors.push(format!("HNS: {e}"));
                None
            }
        },
        None => None,
    };
    let btc = match btc_observe::observe(sources, h).await {
        Ok(v) => Some(v),
        Err(e) => {
            report.errors.push(format!("Bitcoin: {e}"));
            None
        }
    };
    let step = engine::step(h, hns.as_ref(), btc.as_ref(), now, p);
    for a in &step.actions {
        match a {
            Action::SendHns { kind, tx_hex } => {
                let Some(n) = node else { continue };
                // The secret may be public the moment the claim leaves: record
                // the reveal before sending, so no later step refunds BTC on
                // the belief that it never left.
                if *kind == SpendKind::HnsClaim {
                    record(h, *kind, now);
                }
                match send_hns(n, tx_hex).await {
                    Ok(true) => {
                        record(h, *kind, now);
                        report.sent.push(*kind);
                    }
                    Ok(false) => report
                        .errors
                        .push(format!("HNS: the node did not take the {kind:?}")),
                    Err(e) => report
                        .errors
                        .push(format!("HNS: sending the {kind:?} failed: {e}")),
                }
            }
            Action::SendBtc { kind, tx_hex, .. } => match send_btc(sources, tx_hex).await {
                Ok(()) => {
                    record(h, *kind, now);
                    report.sent.push(*kind);
                }
                Err(e) => report
                    .errors
                    .push(format!("Bitcoin: sending the {kind:?} failed: {e}")),
            },
        }
    }
    report.step = Some(step);
    report
}

fn record(h: &mut Holdings, kind: SpendKind, now: u64) {
    match kind {
        SpendKind::HnsClaim => {
            h.sent.hns_claim_at.get_or_insert(now);
        }
        SpendKind::HnsRefund => {
            h.sent.hns_refund_at.get_or_insert(now);
        }
        SpendKind::BtcClaim => {
            h.sent.btc_claim_first_at.get_or_insert(now);
        }
        SpendKind::BtcRefund => {
            h.sent.btc_refund_first_at.get_or_insert(now);
        }
        SpendKind::HnsFunding | SpendKind::BtcFunding => {}
    }
}
