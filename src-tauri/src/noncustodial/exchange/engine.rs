//! What to do next in a trade (spec §3.3, §3.4, R8–R11, R13): a pure
//! function of what a party holds, what the two chains show, and the clock.
//! It decides the status a screen shows and which already-signed
//! transactions to send. It never signs; the daemon, the app and the
//! watchtower all run it, the watchtower with less in hand (no secret, no
//! claim of the Buyer's).
//!
//! The chains are the source of truth: nothing here trusts a stored stage.
//! A chain that could not be read (`None`) is never read as "nothing there":
//! no reveal, no refund and no commit is decided on it.
//!
//! The rules that keep money safe:
//!
//! - The Buyer reveals only for an HNS lock with exactly the terms' script
//!   and value (the IO layer reports only such a coin as `lock`), with the
//!   terms' confirmations, before the reveal deadline by both the local
//!   clock and the HNS chain's median time, while the BTC lock is unspent,
//!   and never after a BTC refund was sent (R8).
//! - Once the HNS lock is claimed (the secret is the Buyer's and public),
//!   the Buyer never refunds BTC: the BTC is the Seller's, even past the
//!   refund time, whatever the Seller's delay (an honest wallet does not
//!   take both coins).
//! - The Seller never refunds HNS once a claim of it is seen, even in the
//!   mempool; it claims BTC instead (R10: only with a secret that hashes to
//!   `H`, which `script::secret_from_claim_witness` already checked).
//! - Refunds go out only once the chain's median time is past the refund
//!   time (R11).

use crate::noncustodial::exchange::btc_htlc::{self, PresignedSpend};
use crate::noncustodial::exchange::fees;
use crate::noncustodial::exchange::hns_htlc::{self, PresignedClaim};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::tx::Outpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Buyer,
    Seller,
}

/// A coin by its outpoint. `txid` in the chain's own display form: natural
/// order hex on HNS, byte-reversed on Bitcoin.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRef {
    pub txid: String,
    pub vout: u32,
}

/// When this party (or its watchtower) last sent each kind of spend.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentLog {
    pub hns_claim_at: Option<u64>,
    pub hns_refund_at: Option<u64>,
    pub btc_claim_first_at: Option<u64>,
    pub btc_refund_first_at: Option<u64>,
}

/// What a party holds for a trade. A watchtower holds the same minus the
/// Buyer's secret and claim.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Holdings {
    pub role: Role,
    pub terms: Terms,
    /// The party committed (signed and sent its lock's funding).
    pub committed: bool,
    /// Buyer: the secret, from the Commit until it is public.
    pub secret: Option<[u8; 32]>,
    /// Buyer: the presigned claim of the HNS lock.
    pub hns_claim: Option<PresignedClaim>,
    /// Buyer: the BTC funding, to send again while unconfirmed.
    pub btc_funding_hex: Option<String>,
    /// Buyer: the BTC refund ladder.
    pub btc_refunds: Vec<PresignedSpend>,
    /// Seller: the HNS funding, to send again while unconfirmed.
    pub hns_funding_hex: Option<String>,
    /// Seller: the HNS refund.
    pub hns_refund_hex: Option<String>,
    /// Seller: the BTC claim ladder, secret slot empty.
    pub btc_claims: Vec<PresignedSpend>,
    /// The BTC lock, once known (the Buyer's Commit, or its announcement).
    pub btc_lock: Option<LockRef>,
    /// Seller: its own HNS lock coin, the only coin it watches (review C1).
    #[serde(default)]
    pub hns_lock: Option<LockRef>,
    pub sent: SentLog,
}

/// A spend of a lock as a chain shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendSeen {
    pub txid: String,
    /// 0 while in the mempool.
    pub confirmations: u32,
    /// `Some` for a claim of the lock (its secret, checked against `H`);
    /// `None` for a refund.
    pub secret: Option<[u8; 32]>,
}

/// The HNS lock as the profile's node shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HnsLockSeen {
    pub outpoint: LockRef,
    pub confirmations: u32,
    pub spend: Option<SpendSeen>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HnsSeen {
    /// The tip's median time past.
    pub mtp: u64,
    /// A coin with exactly the lock's script and value.
    pub lock: Option<HnsLockSeen>,
    /// A payment to the lock's address of another value, or with a covenant.
    pub mismatched_payment: bool,
    /// A secret revealed by a claim of any coin with the lock's script,
    /// checked against `H`.
    pub revealed: Option<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcLockSeen {
    pub outpoint: LockRef,
    /// Confirmations proven by proof of work and agreed by every source
    /// (`btc_spv`); 0 while unconfirmed or not proven.
    pub confirmations: u32,
    /// Exactly the lock's script and value.
    pub matches: bool,
    pub spend: Option<SpendSeen>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcSeen {
    pub mtp: u64,
    pub lock: Option<BtcLockSeen>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpendKind {
    HnsFunding,
    HnsClaim,
    HnsRefund,
    BtcFunding,
    BtcClaim,
    BtcRefund,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SendHns {
        kind: SpendKind,
        tx_hex: String,
    },
    SendBtc {
        kind: SpendKind,
        tx_hex: String,
        rung: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Chain {
    Hns,
    Btc,
}

/// Why a trade needs a person to look at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Attention {
    /// The Seller paid the HNS lock's address another amount: no reveal;
    /// the BTC comes back at its refund time.
    SellerLockedWrongAmount,
    /// The Buyer paid the BTC lock another amount or script: the Seller
    /// never commits.
    BuyerLockedWrongAmount,
    /// The BTC lock was refunded after the secret was public: the Seller
    /// claimed too late. Should never happen with the daemon or a
    /// watchtower running.
    BtcRefundedAfterReveal,
    /// The HNS lock was refunded although the secret was out: the Buyer
    /// refunds BTC as soon as it can.
    HnsRefundedAfterReveal,
    /// Our record is missing what this step needs (a corrupted trade).
    MissingPresigned,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Status {
    /// The Buyer has to commit by `deadline` (for the Seller: waits for it).
    AwaitingBuyerCommit {
        deadline: u64,
    },
    AwaitingBtcLock {
        confirmations: u32,
        needed: u32,
    },
    /// The Seller has to commit by `deadline` (for the Buyer: waits for it).
    AwaitingSellerCommit {
        deadline: u64,
    },
    AwaitingHnsLock {
        confirmations: u32,
        needed: u32,
    },
    /// The Buyer's claim of the HNS lock is out (the secret is public).
    Revealing,
    /// The Seller waits for the Buyer's reveal.
    AwaitingReveal {
        refund_opens_at: u64,
    },
    /// The Seller's claim of the BTC lock is out.
    Claiming {
        rung: usize,
    },
    Completed,
    /// Own lock, waiting for its refund time.
    AwaitingRefund {
        chain: Chain,
        opens_at: u64,
    },
    Refunding {
        chain: Chain,
    },
    Refunded {
        chain: Chain,
    },
    /// Nothing was locked and the deadline passed.
    Expired,
    CannotCheck {
        chain: Chain,
    },
    NeedsAttention {
        reason: Attention,
        chain: Chain,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub status: Status,
    pub actions: Vec<Action>,
}

impl Step {
    fn just(status: Status) -> Step {
        Step {
            status,
            actions: Vec::new(),
        }
    }
}

fn outpoint_of(l: &LockRef) -> Option<Outpoint> {
    let mut hash = [0u8; 32];
    hex::decode_to_slice(&l.txid, &mut hash).ok()?;
    Some(Outpoint {
        hash,
        index: l.vout,
    })
}

/// One step of the trade for whoever holds `h`.
pub fn step(
    h: &Holdings,
    hns: Option<&HnsSeen>,
    btc: Option<&BtcSeen>,
    now: u64,
    p: &Policy,
) -> Step {
    match h.role {
        Role::Buyer => buyer_step(h, hns, btc, now, p),
        Role::Seller => seller_step(h, hns, btc, now, p),
    }
}

/// May the Buyer reveal now? Every condition of R8, except the presence of
/// the secret and the claim, which the caller checks.
pub fn buyer_may_reveal(
    h: &Holdings,
    hns: &HnsSeen,
    btc: Option<&BtcSeen>,
    now: u64,
    p: &Policy,
) -> bool {
    let t = &h.terms;
    let Some(lock) = &hns.lock else { return false };
    lock.spend.is_none()
        && lock.confirmations >= t.hns_confirmations
        && now < t.reveal_deadline
        && hns.mtp.saturating_add(p.reveal_margin) < t.hns_refund_time
        && h.sent.btc_refund_first_at.is_none()
        // The BTC lock must not be spent already (a refund under way).
        && btc.is_none_or(|b| b.lock.as_ref().is_none_or(|l| l.spend.is_none()))
}

fn btc_refund_action(h: &Holdings, now: u64, p: &Policy) -> Option<Action> {
    let rung = fees::rung_now(
        h.btc_refunds.len(),
        h.sent.btc_refund_first_at,
        now,
        None,
        p,
    );
    h.btc_refunds.get(rung).map(|r| Action::SendBtc {
        kind: SpendKind::BtcRefund,
        tx_hex: r.tx_hex.clone(),
        rung,
    })
}

fn buyer_step(
    h: &Holdings,
    hns: Option<&HnsSeen>,
    btc: Option<&BtcSeen>,
    now: u64,
    p: &Policy,
) -> Step {
    let t = &h.terms;
    if !h.committed {
        if now > t.buyer_fund_deadline {
            return Step::just(Status::Expired);
        }
        return Step::just(Status::AwaitingBuyerCommit {
            deadline: t.buyer_fund_deadline,
        });
    }
    let (Ok(hp), Ok(bp)) = (t.hns_params(), t.btc_params()) else {
        return Step::just(Status::NeedsAttention {
            reason: Attention::MissingPresigned,
            chain: Chain::Hns,
        });
    };

    // 1. The HNS lock claimed: done, whatever else happens. Never refund.
    let hns_spend = hns
        .and_then(|v| v.lock.as_ref())
        .and_then(|l| l.spend.as_ref());
    if let Some(s) = hns_spend {
        if s.secret.is_some() {
            if s.confirmations >= 1 {
                return Step::just(Status::Completed);
            }
            let mut actions = Vec::new();
            if let (Some(c), Some(x), Some(lock)) =
                (&h.hns_claim, h.secret, hns.and_then(|v| v.lock.as_ref()))
            {
                if let (Some(op), Ok(n)) = (outpoint_of(&lock.outpoint), t.network()) {
                    if let Ok(tx) = c.finish(n, &hp, op, &x) {
                        actions.push(Action::SendHns {
                            kind: SpendKind::HnsClaim,
                            tx_hex: tx.to_hex(),
                        });
                    }
                }
            }
            return Step {
                status: Status::Revealing,
                actions,
            };
        }
    }

    let btc_lock = btc.and_then(|b| b.lock.as_ref());
    let btc_spend = btc_lock.and_then(|l| l.spend.as_ref());

    // 2. The BTC lock spent.
    if let Some(s) = btc_spend {
        match s.secret {
            None => {
                // Our refund.
                if s.confirmations >= 1 {
                    return Step::just(Status::Refunded { chain: Chain::Btc });
                }
                let actions = btc_refund_action(h, now, p).into_iter().collect();
                return Step {
                    status: Status::Refunding { chain: Chain::Btc },
                    actions,
                };
            }
            Some(_) => {
                // The Seller claimed: the secret is public, so claiming the
                // HNS lock now costs nothing, whatever the deadline.
                let mut actions = Vec::new();
                if let (Some(c), Some(x), Some(v)) = (&h.hns_claim, h.secret, hns) {
                    if let Some(lock) = v.lock.as_ref().filter(|l| l.spend.is_none()) {
                        if let (Some(op), Ok(n)) = (outpoint_of(&lock.outpoint), t.network()) {
                            if let Ok(tx) = c.finish(n, &hp, op, &x) {
                                actions.push(Action::SendHns {
                                    kind: SpendKind::HnsClaim,
                                    tx_hex: tx.to_hex(),
                                });
                            }
                        }
                    }
                }
                return Step {
                    status: Status::Revealing,
                    actions,
                };
            }
        }
    }

    let Some(hns) = hns else {
        // Without the HNS chain nothing is decided, not even a refund of
        // BTC we believe we never revealed for: the record of a reveal can
        // be lost in a crash between the send and the store (review L5),
        // and a refund after a reveal would take the Seller's BTC.
        return Step::just(Status::CannotCheck { chain: Chain::Hns });
    };

    // A secret is public on the HNS chain (our claim of this or any coin
    // with the lock's script): the HNS is ours to take and the BTC is the
    // Seller's. Never refund; keep claiming while the lock is unspent.
    if hns.revealed.is_some() {
        let mut actions = Vec::new();
        if let (Some(c), Some(x), Some(lock)) = (
            &h.hns_claim,
            h.secret,
            hns.lock.as_ref().filter(|l| l.spend.is_none()),
        ) {
            if let (Some(op), Ok(n)) = (outpoint_of(&lock.outpoint), t.network()) {
                if let Ok(tx) = c.finish(n, &hp, op, &x) {
                    actions.push(Action::SendHns {
                        kind: SpendKind::HnsClaim,
                        tx_hex: tx.to_hex(),
                    });
                }
            }
        }
        return Step {
            status: Status::Revealing,
            actions,
        };
    }

    // The HNS lock refunded by the Seller.
    if hns.lock.as_ref().and_then(|l| l.spend.as_ref()).is_some() {
        // (A claim was handled above, so this is the Seller's refund.)
        let leaked = h.sent.hns_claim_at.is_some();
        return refund_btc_when_open(h, btc, &bp, now, p, leaked);
    }

    // 3. Reveal when every condition holds.
    if let (Some(c), Some(x)) = (&h.hns_claim, h.secret) {
        if buyer_may_reveal(h, hns, btc, now, p) {
            let lock = hns.lock.as_ref().expect("checked by buyer_may_reveal");
            let tx = outpoint_of(&lock.outpoint)
                .and_then(|op| t.network().ok().and_then(|n| c.finish(n, &hp, op, &x).ok()));
            if let Some(tx) = tx {
                return Step {
                    status: Status::Revealing,
                    actions: vec![Action::SendHns {
                        kind: SpendKind::HnsClaim,
                        tx_hex: tx.to_hex(),
                    }],
                };
            }
        }
    } else if h.secret.is_none() && h.hns_claim.is_none() {
        // A watchtower: it can only refund.
    } else {
        return Step::just(Status::NeedsAttention {
            reason: Attention::MissingPresigned,
            chain: Chain::Hns,
        });
    }

    // We revealed but the claim is not on chain yet: keep sending it while
    // it may still be mined.
    if h.sent.hns_claim_at.is_some() {
        if let (Some(c), Some(x), Some(lock)) = (&h.hns_claim, h.secret, &hns.lock) {
            if let (Some(op), Ok(n)) = (outpoint_of(&lock.outpoint), t.network()) {
                if let Ok(tx) = c.finish(n, &hp, op, &x) {
                    return Step {
                        status: Status::Revealing,
                        actions: vec![Action::SendHns {
                            kind: SpendKind::HnsClaim,
                            tx_hex: tx.to_hex(),
                        }],
                    };
                }
            }
        }
        // Our claim is out but no lock is seen (a reorg?): wait, never
        // refund BTC while the secret may be public and the HNS claimable.
        return Step::just(Status::Revealing);
    }

    if hns.mismatched_payment && hns.lock.is_none() {
        let step = refund_btc_when_open(h, btc, &bp, now, p, false);
        if matches!(
            step.status,
            Status::Refunding { .. } | Status::Refunded { .. }
        ) {
            return step;
        }
        return Step::just(Status::NeedsAttention {
            reason: Attention::SellerLockedWrongAmount,
            chain: Chain::Hns,
        });
    }

    // Past the reveal deadline, or past the Seller's deadline with no lock:
    // the trade will not happen; refund when the time comes.
    let no_more_reveal =
        now >= t.reveal_deadline || (hns.lock.is_none() && now > t.seller_fund_deadline);
    if no_more_reveal {
        return refund_btc_when_open(h, btc, &bp, now, p, false);
    }

    // Waiting.
    if let Some(lock) = &hns.lock {
        return Step::just(Status::AwaitingHnsLock {
            confirmations: lock.confirmations,
            needed: t.hns_confirmations,
        });
    }
    let btc_confs = btc_lock.map(|l| l.confirmations).unwrap_or(0);
    if btc_confs < t.btc_confirmations {
        let mut actions = Vec::new();
        if btc_confs == 0 {
            if let Some(f) = &h.btc_funding_hex {
                actions.push(Action::SendBtc {
                    kind: SpendKind::BtcFunding,
                    tx_hex: f.clone(),
                    rung: 0,
                });
            }
        }
        return Step {
            status: Status::AwaitingBtcLock {
                confirmations: btc_confs,
                needed: t.btc_confirmations,
            },
            actions,
        };
    }
    Step::just(Status::AwaitingSellerCommit {
        deadline: t.seller_fund_deadline,
    })
}

fn refund_btc_when_open(
    h: &Holdings,
    btc: Option<&BtcSeen>,
    bp: &crate::noncustodial::exchange::script::HtlcParams,
    now: u64,
    p: &Policy,
    leaked: bool,
) -> Step {
    let Some(b) = btc else {
        return Step::just(Status::CannotCheck { chain: Chain::Btc });
    };
    if b.lock.is_none() {
        // No source shows the BTC lock. That is not "it does not exist"
        // (review H2: a lagging source, a node without -txindex): a
        // committed trade is never given up on it. Send the funding again
        // and keep looking; the refund is presigned whatever happens.
        let actions = h
            .btc_funding_hex
            .iter()
            .map(|f| Action::SendBtc {
                kind: SpendKind::BtcFunding,
                tx_hex: f.clone(),
                rung: 0,
            })
            .collect();
        return Step {
            status: Status::CannotCheck { chain: Chain::Btc },
            actions,
        };
    }
    if btc_htlc::time_lock_open(bp.refund_lock_time, b.mtp) {
        let actions: Vec<Action> = btc_refund_action(h, now, p).into_iter().collect();
        if actions.is_empty() && !h.btc_refunds.is_empty() {
            return Step::just(Status::NeedsAttention {
                reason: Attention::MissingPresigned,
                chain: Chain::Btc,
            });
        }
        return Step {
            status: Status::Refunding { chain: Chain::Btc },
            actions,
        };
    }
    if leaked {
        return Step::just(Status::NeedsAttention {
            reason: Attention::HnsRefundedAfterReveal,
            chain: Chain::Btc,
        });
    }
    Step::just(Status::AwaitingRefund {
        chain: Chain::Btc,
        opens_at: h.terms.btc_refund_time,
    })
}

/// May the Seller commit now? R9: the BTC lock is exactly the terms', has
/// the terms' proven confirmations, is unspent, and the funding deadline
/// has not passed. `Err` is the sentence the screen shows.
pub fn seller_may_commit(
    h: &Holdings,
    btc: Option<&BtcSeen>,
    now: u64,
) -> Result<(), &'static str> {
    let t = &h.terms;
    if h.role != Role::Seller {
        return Err("only the Seller locks HNS in this trade");
    }
    if h.committed {
        return Err("the HNS is already locked for this trade");
    }
    if now > t.seller_fund_deadline {
        return Err("the time to lock HNS for this trade is over");
    }
    let Some(b) = btc else {
        return Err("the Bitcoin chain could not be checked");
    };
    let Some(l) = &b.lock else {
        return Err("the buyer's BTC is not locked yet");
    };
    if !l.matches {
        return Err("the buyer locked BTC that does not match the trade");
    }
    if l.spend.is_some() {
        return Err("the buyer's BTC lock is already spent");
    }
    if l.confirmations < t.btc_confirmations {
        return Err("the buyer's BTC lock does not have its confirmations yet");
    }
    Ok(())
}

fn seller_step(
    h: &Holdings,
    hns: Option<&HnsSeen>,
    btc: Option<&BtcSeen>,
    now: u64,
    p: &Policy,
) -> Step {
    let t = &h.terms;
    let (Ok(hp), Ok(bp)) = (t.hns_params(), t.btc_params()) else {
        return Step::just(Status::NeedsAttention {
            reason: Attention::MissingPresigned,
            chain: Chain::Hns,
        });
    };
    if !h.committed {
        let Some(b) = btc else {
            if now > t.seller_fund_deadline {
                return Step::just(Status::Expired);
            }
            return Step::just(Status::CannotCheck { chain: Chain::Btc });
        };
        if now > t.seller_fund_deadline {
            return Step::just(Status::Expired);
        }
        return match &b.lock {
            None if now > t.buyer_fund_deadline => Step::just(Status::Expired),
            None => Step::just(Status::AwaitingBuyerCommit {
                deadline: t.buyer_fund_deadline,
            }),
            Some(l) if !l.matches => Step::just(Status::NeedsAttention {
                reason: Attention::BuyerLockedWrongAmount,
                chain: Chain::Btc,
            }),
            Some(l) if l.spend.is_some() => Step::just(Status::Expired),
            Some(l) if l.confirmations < t.btc_confirmations => {
                Step::just(Status::AwaitingBtcLock {
                    confirmations: l.confirmations,
                    needed: t.btc_confirmations,
                })
            }
            Some(_) => Step::just(Status::AwaitingSellerCommit {
                deadline: t.seller_fund_deadline,
            }),
        };
    }

    let Some(hns) = hns else {
        return Step::just(Status::CannotCheck { chain: Chain::Hns });
    };
    let lock = hns.lock.as_ref();
    let spend = lock.and_then(|l| l.spend.as_ref());

    // The Buyer claimed (even in the mempool): the secret is ours to use.
    if let Some(x) = spend.and_then(|s| s.secret).or(hns.revealed) {
        // The secret came from a claim of another coin at the lock's
        // script (a decoy, review C1) while our own coin is still unspent:
        // once its refund time is open it is ours to take back, or it
        // would stay stranded. Until then the Buyer may still claim it.
        let own_back: Vec<Action> = match (lock, &h.hns_refund_hex) {
            (Some(l), Some(r))
                if l.spend.is_none() && hns_htlc::time_lock_open(hp.refund_lock_time, hns.mtp) =>
            {
                vec![Action::SendHns {
                    kind: SpendKind::HnsRefund,
                    tx_hex: r.clone(),
                }]
            }
            _ => Vec::new(),
        };
        let btc_spend = btc
            .and_then(|b| b.lock.as_ref())
            .and_then(|l| l.spend.as_ref());
        match btc_spend {
            Some(s) if s.secret.is_some() && s.confirmations >= 1 => {
                return Step {
                    status: Status::Completed,
                    actions: own_back,
                };
            }
            Some(s) if s.secret.is_none() && s.confirmations >= 1 => {
                // The Buyer's refund, proven mined: we were too late.
                return Step::just(Status::NeedsAttention {
                    reason: Attention::BtcRefundedAfterReveal,
                    chain: Chain::Btc,
                });
            }
            _ => {}
        }
        // A competing spend in the mempool (the Buyer's refund racing our
        // claim): go straight to the top rung and keep it there (review L7).
        let competing = btc_spend.is_some_and(|s| s.secret.is_none() && s.confirmations == 0);
        let rung = if competing {
            h.btc_claims.len().saturating_sub(1)
        } else {
            fees::rung_now(
                h.btc_claims.len(),
                h.sent.btc_claim_first_at,
                now,
                Some(t.btc_refund_time),
                p,
            )
        };
        let Some(presigned) = h.btc_claims.get(rung) else {
            return Step::just(Status::NeedsAttention {
                reason: Attention::MissingPresigned,
                chain: Chain::Btc,
            });
        };
        return match btc_htlc::finish_claim(presigned, &bp, &x) {
            Ok(tx) => Step {
                status: Status::Claiming { rung },
                actions: std::iter::once(Action::SendBtc {
                    kind: SpendKind::BtcClaim,
                    tx_hex: btc_htlc::encode_tx(&tx),
                    rung,
                })
                .chain(own_back)
                .collect(),
            },
            Err(_) => Step::just(Status::NeedsAttention {
                reason: Attention::MissingPresigned,
                chain: Chain::Btc,
            }),
        };
    }

    // Our refund of the HNS lock.
    if let Some(s) = spend {
        if s.confirmations >= 1 {
            return Step::just(Status::Refunded { chain: Chain::Hns });
        }
        let actions = h
            .hns_refund_hex
            .iter()
            .map(|r| Action::SendHns {
                kind: SpendKind::HnsRefund,
                tx_hex: r.clone(),
            })
            .collect();
        return Step {
            status: Status::Refunding { chain: Chain::Hns },
            actions,
        };
    }

    let Some(lock) = lock else {
        // Our funding is not seen yet: send it again.
        let actions = h
            .hns_funding_hex
            .iter()
            .map(|f| Action::SendHns {
                kind: SpendKind::HnsFunding,
                tx_hex: f.clone(),
            })
            .collect();
        return Step {
            status: Status::AwaitingHnsLock {
                confirmations: 0,
                needed: t.hns_confirmations,
            },
            actions,
        };
    };

    if hns_htlc::time_lock_open(hp.refund_lock_time, hns.mtp) {
        let Some(r) = &h.hns_refund_hex else {
            return Step::just(Status::NeedsAttention {
                reason: Attention::MissingPresigned,
                chain: Chain::Hns,
            });
        };
        return Step {
            status: Status::Refunding { chain: Chain::Hns },
            actions: vec![Action::SendHns {
                kind: SpendKind::HnsRefund,
                tx_hex: r.clone(),
            }],
        };
    }
    if lock.confirmations < t.hns_confirmations {
        return Step::just(Status::AwaitingHnsLock {
            confirmations: lock.confirmations,
            needed: t.hns_confirmations,
        });
    }
    if now >= t.reveal_deadline {
        return Step::just(Status::AwaitingRefund {
            chain: Chain::Hns,
            opens_at: t.hns_refund_time,
        });
    }
    Step::just(Status::AwaitingReveal {
        refund_opens_at: t.hns_refund_time,
    })
}
