//! A watchtower's job (spec §2, R13): the presigned transactions of one
//! party's side of a trade, and nothing secret. The Buyer's secret and its
//! HNS claim are not fields of a job, so no code path can hand them to a
//! watchtower. Every transaction in a job pays the party that signed it; a
//! watchtower can only send it at the right moment, or fail to.
//!
//! A watchtower runs [`engine::step`] on [`WatchJob::holdings`]: the Seller's
//! job claims the BTC lock once the secret is public and refunds the HNS
//! lock when its time comes; the Buyer's job refunds the BTC lock when its
//! time comes and the HNS lock was never claimed.

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::noncustodial::exchange::btc_htlc::{self, PresignedSpend};
use crate::noncustodial::exchange::engine::{Holdings, LockRef, Role, SentLog};
use crate::noncustodial::exchange::script;
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::tx::Transaction;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchJob {
    pub role: Role,
    pub terms: Terms,
    pub btc_lock: Option<LockRef>,
    pub btc_refunds: Vec<PresignedSpend>,
    pub hns_refund_hex: Option<String>,
    pub btc_claims: Vec<PresignedSpend>,
}

impl WatchJob {
    /// The job of `h`'s side, after its Commit.
    pub fn from_holdings(h: &Holdings) -> WatchJob {
        WatchJob {
            role: h.role,
            terms: h.terms.clone(),
            btc_lock: h.btc_lock.clone(),
            btc_refunds: h.btc_refunds.clone(),
            hns_refund_hex: h.hns_refund_hex.clone(),
            btc_claims: h.btc_claims.clone(),
        }
    }

    /// What the watchtower runs the engine on: committed, nothing secret.
    pub fn holdings(&self, sent: SentLog) -> Holdings {
        Holdings {
            role: self.role,
            terms: self.terms.clone(),
            committed: true,
            secret: None,
            hns_claim: None,
            btc_funding_hex: None,
            btc_refunds: self.btc_refunds.clone(),
            hns_funding_hex: None,
            hns_refund_hex: self.hns_refund_hex.clone(),
            btc_claims: self.btc_claims.clone(),
            btc_lock: self.btc_lock.clone(),
            sent,
        }
    }

    /// The job's shape: its transactions are spends of the terms' locks of
    /// the right kind for its role. A watchtower refuses a job that fails.
    pub fn check(&self) -> Result<(), AppError> {
        let bad = |m: &str| {
            Err(AppError::InvalidInput(format!(
                "the watch job is refused: {m}"
            )))
        };
        let hp = self.terms.hns_params()?;
        let bp = self.terms.btc_params()?;
        match self.role {
            Role::Buyer => {
                if self.btc_refunds.is_empty() || self.btc_lock.is_none() {
                    return bad("a Buyer's job holds the BTC lock and its refunds");
                }
                if self.hns_refund_hex.is_some() || !self.btc_claims.is_empty() {
                    return bad("a Buyer's job holds only BTC refunds");
                }
                for r in &self.btc_refunds {
                    btc_htlc::refund_tx(r, &bp)?;
                }
            }
            Role::Seller => {
                let Some(r) = &self.hns_refund_hex else {
                    return bad("a Seller's job holds the HNS refund");
                };
                if self.btc_claims.is_empty() || !self.btc_refunds.is_empty() {
                    return bad("a Seller's job holds the HNS refund and the BTC claims");
                }
                let raw = hex::decode(r)
                    .map_err(|_| AppError::InvalidInput("refund is not hex".into()))?;
                let tx = Transaction::decode(&raw)?;
                let w = tx
                    .inputs
                    .first()
                    .map(|i| i.witness.clone())
                    .unwrap_or_default();
                if tx.inputs.len() != 1 || w.len() != 3 || !w[1].is_empty() || !hp.matches(&w[2]) {
                    return bad("its HNS refund is not a refund of the HNS lock");
                }
                for c in &self.btc_claims {
                    let tx = btc_htlc::decode(&c.tx_hex)?;
                    let items = tx
                        .input
                        .first()
                        .map(|i| i.witness.to_vec())
                        .unwrap_or_default();
                    if tx.input.len() != 1
                        || items.len() != 4
                        || !bp.matches(&items[3])
                        || items[1] != [0u8; 32]
                    {
                        return bad(
                            "a BTC claim is not a claim of the BTC lock with an empty secret slot",
                        );
                    }
                }
            }
        }
        let _ = script::SECRET_LEN;
        Ok(())
    }
}
