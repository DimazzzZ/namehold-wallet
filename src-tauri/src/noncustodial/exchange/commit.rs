//! The two Commits (spec §3.2): everything one party signs for a trade, in
//! one go, from the unlocked master key. Pure: the caller gives the coins,
//! the rates and the addresses; nothing here reads a node or a database.
//!
//! - [`buyer_commit`] signs the BTC lock's funding from the BTC account,
//!   the BTC refund ladder, and the HNS claim (`0xc1`) for the HNS lock the
//!   Seller has not made yet.
//! - [`seller_commit`] takes the HNS lock's funding the wallet signed
//!   (`actions::sign_plan`), and signs the HNS refund and the BTC claim
//!   ladder for the BTC lock that already exists.
//!
//! Both refuse terms whose keys or hash are not this profile's for the
//! trade index: a Commit never signs for a lock it could not spend.

use bitcoin::{Address, Amount, OutPoint, TxOut};

use crate::error::AppError;
use crate::noncustodial::exchange::btc_account::{self, BtcCoin};
use crate::noncustodial::exchange::btc_htlc::{self, PresignedSpend};
use crate::noncustodial::exchange::engine::{Holdings, LockRef, Role, SentLog};
use crate::noncustodial::exchange::fees;
use crate::noncustodial::exchange::hns_htlc::{self, PresignedClaim};
use crate::noncustodial::exchange::keys;
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::hd::ExtendedPrivKey;
use crate::noncustodial::tx::{Outpoint, Transaction};

pub struct BuyerCommitInput<'a> {
    pub master: &'a ExtendedPrivKey,
    pub account: u32,
    pub trade_index: u32,
    /// The nonce of the Buyer's secret, kept with the trade.
    pub secret_nonce: &'a [u8; 32],
    pub terms: &'a Terms,
    pub policy: &'a Policy,
    pub btc_coins: &'a [BtcCoin],
    /// The BTC account's change address and the refund's destination.
    pub btc_change_to: &'a Address,
    pub btc_refund_to: &'a Address,
    /// The Bitcoin source's fastest rate, sat/vbyte.
    pub btc_rate: u64,
    /// The HNS node's rate, doos/vbyte, and the wallet address the HNS goes
    /// to.
    pub hns_rate: u64,
    pub hns_to: &'a str,
}

#[derive(Debug)]
pub struct BuyerCommit {
    pub funding: bitcoin::Transaction,
    pub funding_fee: u64,
    pub btc_lock: LockRef,
    pub hns_claim: PresignedClaim,
    pub btc_refunds: Vec<PresignedSpend>,
    pub holdings: Holdings,
}

fn refuse(m: &str) -> AppError {
    AppError::InvalidInput(format!("this trade cannot be signed: {m}"))
}

pub fn buyer_commit(i: &BuyerCommitInput<'_>) -> Result<BuyerCommit, AppError> {
    let t = i.terms;
    let network = t.network()?;
    let k = keys::trade_keys(i.master, network, i.account, i.trade_index)?;
    if hex::encode(k.hns_pubkey) != t.buyer_hns_claim_pubkey
        || hex::encode(k.btc_pubkey) != t.buyer_btc_refund_pubkey
    {
        return Err(refuse("its keys are not this wallet's"));
    }
    if hex::encode(k.hash(i.secret_nonce)) != t.hash {
        return Err(refuse("its hash is not this wallet's secret"));
    }
    let hp = t.hns_params()?;
    let bp = t.btc_params()?;

    // The BTC lock's funding: the lock is output 0.
    let lock_out = TxOut {
        value: Amount::from_sat(t.btc_amount),
        script_pubkey: btc_htlc::lock_script_pubkey(&bp),
    };
    let rate = i.btc_rate.max(i.policy.btc_min_rate);
    let f = btc_account::fund(i.btc_coins, vec![lock_out], i.btc_change_to, rate)?;
    let funding = btc_account::sign_funding(&f, i.master, network, i.account)?;
    let lock = OutPoint {
        txid: funding.compute_txid(),
        vout: 0,
    };

    // The refund ladder.
    let vsize = btc_htlc::spend_vsize(&bp, i.btc_refund_to, false);
    let rungs = fees::btc_ladder(i.btc_rate, vsize, t.btc_amount, i.policy)?;
    let btc_refunds = rungs
        .iter()
        .map(|r| {
            btc_htlc::sign_refund(
                &bp,
                lock,
                t.btc_amount,
                &k.btc,
                i.btc_refund_to,
                r.fee,
                r.rate,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    // The HNS claim, for a lock that does not exist yet.
    let hns_rate = fees::hns_presign_rate(i.hns_rate, i.policy);
    let claim_fee = hns_htlc::claim_fee(network, &hp, i.hns_to, hns_rate)?;
    let hns_claim = hns_htlc::sign_claim(network, &hp, t.hns_amount, &k.hns, i.hns_to, claim_fee)?;
    if !hns_claim.verify(network, &hp) {
        return Err(refuse("the HNS claim did not verify"));
    }

    let btc_lock = LockRef {
        txid: lock.txid.to_string(),
        vout: 0,
    };
    let funding_hex = btc_htlc::encode_tx(&funding);
    let holdings = Holdings {
        role: Role::Buyer,
        terms: t.clone(),
        committed: true,
        secret: Some(k.secret(i.secret_nonce)),
        hns_claim: Some(hns_claim.clone()),
        btc_funding_hex: Some(funding_hex),
        btc_refunds: btc_refunds.clone(),
        hns_funding_hex: None,
        hns_refund_hex: None,
        btc_claims: Vec::new(),
        btc_lock: Some(btc_lock.clone()),
        hns_lock: None,
        sent: SentLog::default(),
    };
    Ok(BuyerCommit {
        funding,
        funding_fee: f.fee,
        btc_lock,
        hns_claim,
        btc_refunds,
        holdings,
    })
}

pub struct SellerCommitInput<'a> {
    pub master: &'a ExtendedPrivKey,
    pub account: u32,
    pub trade_index: u32,
    pub terms: &'a Terms,
    pub policy: &'a Policy,
    /// The HNS lock's funding, already signed by the wallet.
    pub hns_funding: &'a Transaction,
    /// The BTC lock, checked (`engine::seller_may_commit`).
    pub btc_lock: &'a LockRef,
    pub hns_rate: u64,
    pub hns_refund_to: &'a str,
    pub btc_rate: u64,
    pub btc_claim_to: &'a Address,
}

#[derive(Debug)]
pub struct SellerCommit {
    pub hns_lock: LockRef,
    pub hns_refund: Transaction,
    pub btc_claims: Vec<PresignedSpend>,
    pub holdings: Holdings,
}

/// The index of the HNS lock's output in `funding`, which must hold exactly
/// one output that is the lock of `terms` worth exactly its HNS amount.
pub fn hns_lock_vout(funding: &Transaction, terms: &Terms) -> Result<u32, AppError> {
    let hp = terms.hns_params()?;
    let found: Vec<usize> = funding
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.address.hash == hns_htlc::program(&hp.script()))
        .map(|(n, _)| n)
        .collect();
    match found.as_slice() {
        [n] if hns_htlc::is_lock_output(&funding.outputs[*n], &hp, terms.hns_amount) => {
            Ok(*n as u32)
        }
        [] => Err(refuse("the HNS funding does not pay the lock")),
        [_] => Err(refuse("the HNS funding pays the lock another amount")),
        _ => Err(refuse("the HNS funding pays the lock twice")),
    }
}

pub fn seller_commit(i: &SellerCommitInput<'_>) -> Result<SellerCommit, AppError> {
    let t = i.terms;
    let network = t.network()?;
    let k = keys::trade_keys(i.master, network, i.account, i.trade_index)?;
    if hex::encode(k.hns_pubkey) != t.seller_hns_refund_pubkey
        || hex::encode(k.btc_pubkey) != t.seller_btc_claim_pubkey
    {
        return Err(refuse("its keys are not this wallet's"));
    }
    let hp = t.hns_params()?;
    let bp = t.btc_params()?;
    let vout = hns_lock_vout(i.hns_funding, t)?;
    let txid = i.hns_funding.txid();
    let mut hash = [0u8; 32];
    hex::decode_to_slice(&txid, &mut hash).map_err(|_| refuse("bad funding txid"))?;

    let hns_rate = fees::hns_presign_rate(i.hns_rate, i.policy);
    let refund_fee = hns_htlc::refund_fee(network, &hp, i.hns_refund_to, hns_rate)?;
    let hns_refund = hns_htlc::sign_refund(
        network,
        &hp,
        Outpoint { hash, index: vout },
        t.hns_amount,
        &k.hns,
        i.hns_refund_to,
        refund_fee,
    )?;

    let lock = OutPoint {
        txid: btc_htlc::parse_txid(&i.btc_lock.txid)?,
        vout: i.btc_lock.vout,
    };
    let vsize = btc_htlc::spend_vsize(&bp, i.btc_claim_to, true);
    let rungs = fees::btc_ladder(i.btc_rate, vsize, t.btc_amount, i.policy)?;
    let btc_claims = rungs
        .iter()
        .map(|r| {
            btc_htlc::sign_claim(
                &bp,
                lock,
                t.btc_amount,
                &k.btc,
                i.btc_claim_to,
                r.fee,
                r.rate,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let hns_lock = LockRef { txid, vout };
    let holdings = Holdings {
        role: Role::Seller,
        terms: t.clone(),
        committed: true,
        secret: None,
        hns_claim: None,
        btc_funding_hex: None,
        btc_refunds: Vec::new(),
        hns_funding_hex: Some(i.hns_funding.to_hex()),
        hns_refund_hex: Some(hns_refund.to_hex()),
        btc_claims: btc_claims.clone(),
        btc_lock: Some(i.btc_lock.clone()),
        hns_lock: Some(hns_lock.clone()),
        sent: SentLog::default(),
    };
    Ok(SellerCommit {
        hns_lock,
        hns_refund,
        btc_claims,
        holdings,
    })
}
