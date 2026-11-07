//! The watchtower (spec §2, §8, R13): every minute, one engine step on each
//! job with the server's own HNS node and Bitcoin sources, sending what the
//! step returns. It runs exactly `exchange_io::runner::run_step`, the code
//! the daemon runs, on `WatchJob::holdings`: committed, no secret, no claim
//! of the Buyer's. So it can send only the transactions the party signed,
//! with at most the public secret filled in; it can fail to send, never
//! redirect a coin.
//!
//! The node must be on the job's network: a node on another chain would
//! show no lock at all, and the Buyer's refund would be decided on that. A
//! node whose network is wrong or unknown is not used for that tick (the
//! step then decides nothing on the HNS chain).

use std::collections::HashMap;

use crate::error::AppError;
use crate::exchange_io::btc_source::BtcSource;
use crate::exchange_io::runner;
use crate::noncustodial::exchange::engine::{SentLog, SpendKind, Status};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::watch::WatchJob;
use crate::noncustodial::network::{network_check, Network};
use crate::noncustodial::node_rpc::NodeRpc;
use crate::relay_server::store::{self, JobUpdate, Store};

/// A final status is re-checked for this long before the job stops running
/// (a reorganisation, or a funding mined late), then kept until
/// `handlers::FINISHED_JOB_TTL_SECS`.
pub const FINISHED_GRACE_SECS: u64 = 24 * 3600;

/// What the watchtower reads and sends through on one network.
pub struct NetworkIo<'a> {
    /// `None`: no node; the step decides nothing on the HNS chain.
    pub node: Option<&'a dyn NodeRpc>,
    pub sources: &'a [Box<dyn BtcSource>],
}

/// One job's step, for the server's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutcome {
    pub id: String,
    pub status: Option<Status>,
    pub sent: Vec<SpendKind>,
    pub errors: Vec<String>,
}

/// Whether a status ends the trade for this party.
pub fn is_final(s: &Status) -> bool {
    matches!(
        s,
        Status::Completed | Status::Refunded { .. } | Status::Expired
    )
}

/// `Ok` when `node` reports itself on `network`.
pub async fn node_on_network(node: &dyn NodeRpc, network: Network) -> Result<(), String> {
    let info = node
        .get_blockchain_info()
        .await
        .map_err(|e| format!("HNS: the node could not be read: {e}"))?;
    match network_check(Some(network.as_str()), info.chain.as_deref()) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "HNS: the node is on '{}', not '{}'",
            info.chain.unwrap_or_default(),
            network.as_str()
        )),
        None => Err("HNS: the node did not report its network".into()),
    }
}

/// One watchtower round over every job still running.
pub async fn tick(
    store: &Store,
    io: &HashMap<Network, NetworkIo<'_>>,
    now: u64,
) -> Result<Vec<JobOutcome>, AppError> {
    let jobs = {
        let c = store.lock()?;
        store::jobs_to_run(&c, now.saturating_sub(FINISHED_GRACE_SECS))?
    };
    // Each network's node is checked once per round.
    let mut node_ok: HashMap<Network, Result<(), String>> = HashMap::new();
    let mut outcomes = Vec::with_capacity(jobs.len());
    for row in jobs {
        let mut out = JobOutcome {
            id: row.id.clone(),
            status: None,
            sent: Vec::new(),
            errors: Vec::new(),
        };
        let parsed = serde_json::from_str::<WatchJob>(&row.json)
            .map_err(|e| format!("the stored job is corrupted: {e}"))
            .and_then(|j| {
                serde_json::from_str::<SentLog>(&row.sent_json)
                    .map(|s| (j, s))
                    .map_err(|e| format!("the stored sent log is corrupted: {e}"))
            });
        let (job, sent) = match parsed {
            Ok(v) => v,
            Err(e) => {
                out.errors.push(e);
                outcomes.push(out);
                continue;
            }
        };
        let Some(network) = Network::from_str_opt(&row.network) else {
            out.errors
                .push(format!("unknown network '{}'", row.network));
            outcomes.push(out);
            continue;
        };
        let Some(nio) = io.get(&network) else {
            out.errors.push(format!(
                "this server runs no watchtower for '{}'",
                network.as_str()
            ));
            outcomes.push(out);
            continue;
        };
        let node = match nio.node {
            Some(n) => {
                let checked = match node_ok.get(&network) {
                    Some(c) => c.clone(),
                    None => {
                        let c = node_on_network(n, network).await;
                        node_ok.insert(network, c.clone());
                        c
                    }
                };
                match checked {
                    Ok(()) => Some(n),
                    Err(e) => {
                        out.errors.push(e);
                        None
                    }
                }
            }
            None => None,
        };
        let mut h = job.holdings(sent);
        let report = runner::run_step(
            node,
            nio.sources,
            &mut h,
            now,
            &Policy::for_network(network),
        )
        .await;
        out.errors.extend(report.errors);
        out.sent = report.sent;
        out.status = report.step.map(|s| s.status);

        let status_json = match &out.status {
            Some(s) => Some(serde_json::to_string(s)?),
            None => None,
        };
        let sent_json = serde_json::to_string(&h.sent)?;
        let last_error = (!out.errors.is_empty()).then(|| out.errors.join("; "));
        {
            let c = store.lock()?;
            store::save_job_step(
                &c,
                &row.id,
                &JobUpdate {
                    status_json: status_json.as_deref(),
                    sent_json: &sent_json,
                    // A step that decided nothing leaves the job as it was.
                    finished: match &out.status {
                        Some(s) => is_final(s),
                        None => row.finished_at.is_some(),
                    },
                    last_error: last_error.as_deref(),
                },
                now,
            )?;
        }
        outcomes.push(out);
    }
    Ok(outcomes)
}
