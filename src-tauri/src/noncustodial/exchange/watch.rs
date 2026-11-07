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
use crate::noncustodial::exchange::hns_htlc;
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::tx::Transaction;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchJob {
    pub role: Role,
    pub terms: Terms,
    pub btc_lock: Option<LockRef>,
    /// The Seller's own HNS lock coin.
    #[serde(default)]
    pub hns_lock: Option<LockRef>,
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
            hns_lock: h.hns_lock.clone(),
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
            hns_lock: self.hns_lock.clone(),
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
                    let tx = btc_htlc::refund_tx(r, &bp)?;
                    if !self.spends_btc_lock(&tx)
                        || !btc_htlc::spend_signed_by(
                            &tx,
                            &bp,
                            self.terms.btc_amount,
                            &bp.refund_pubkey,
                        )
                    {
                        return bad(
                            "a BTC refund is not signed by the lock's refund key for this lock",
                        );
                    }
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
                let sig: Option<[u8; 65]> = w[0].as_slice().try_into().ok();
                let signed = sig.is_some_and(|sig| {
                    hns_htlc::verify_input_signature(
                        &tx,
                        &w[2],
                        self.terms.hns_amount,
                        &sig,
                        hns_htlc::REFUND_SIGHASH,
                        &hp.refund_pubkey,
                    )
                });
                if !signed || tx.locktime != hp.refund_lock_time {
                    return bad("its HNS refund is not signed by the lock's refund key");
                }
                if self.btc_lock.is_none() {
                    return bad("a Seller's job names the BTC lock it claims");
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
                    if !self.spends_btc_lock(&tx)
                        || !btc_htlc::spend_signed_by(
                            &tx,
                            &bp,
                            self.terms.btc_amount,
                            &bp.claim_pubkey,
                        )
                    {
                        return bad(
                            "a BTC claim is not signed by the lock's claim key for this lock",
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// [`check`](Self::check), and the job's terms are a trade the server
    /// can still have to act on (review M2): their times follow the
    /// network's policy windows from their start, and the start lies between
    /// the longest a trade lives ago and the allowed clock skew ahead. Junk
    /// terms with far-future times are refused.
    pub fn check_fresh(&self, now: u64) -> Result<(), AppError> {
        self.check()?;
        let p = crate::noncustodial::exchange::policy::Policy::for_network(self.terms.network()?);
        let t = crate::noncustodial::exchange::terms::timeline(&p, self.terms.start);
        let tt = &self.terms;
        if tt.buyer_fund_deadline != t.buyer_fund_deadline
            || tt.seller_fund_deadline != t.seller_fund_deadline
            || tt.reveal_deadline != t.reveal_deadline
            || tt.hns_refund_time != t.hns_refund_time
            || tt.btc_refund_time != t.btc_refund_time
        {
            return Err(AppError::InvalidInput(
                "the watch job is refused: its times are not the network's windows".into(),
            ));
        }
        let life = p.hns_refund_after + p.btc_refund_after_hns + 24 * 3600;
        if tt.start > now + p.clock_skew || tt.start + life < now {
            return Err(AppError::InvalidInput(
                "the watch job is refused: its trade starts too far from now".into(),
            ));
        }
        Ok(())
    }

    /// Whether `tx` spends exactly the job's BTC lock.
    fn spends_btc_lock(&self, tx: &bitcoin::Transaction) -> bool {
        let Some(l) = &self.btc_lock else {
            return false;
        };
        tx.input.len() == 1
            && tx.input[0].previous_output.txid.to_string() == l.txid
            && tx.input[0].previous_output.vout == l.vout
    }
}
