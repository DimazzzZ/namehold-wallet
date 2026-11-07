//! The terms of a trade (spec §2, R7): everything both lock scripts are
//! made from, agreed before any coin moves, with every time derived from
//! the start by the policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::script::HtlcParams;
use crate::noncustodial::exchange::{btc_htlc, hns_htlc};
use crate::noncustodial::network::Network;

pub const TERMS_VERSION: u8 = 1;

/// The terms. Field order is the canonical encoding's order (serde writes
/// struct fields in declaration order); do not reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Terms {
    pub version: u8,
    /// The HNS network ("main", "testnet", "regtest", "simnet"); Bitcoin is
    /// the paired network (`btc_htlc::btc_network`).
    pub network: String,
    /// The offer this trade takes (64 hex).
    pub offer_id: String,
    /// Chosen by the taker (32 hex).
    pub trade_id: String,
    pub hns_amount: u64,
    pub btc_amount: u64,
    /// The price the amounts were computed at, msat per HNS.
    pub price_msat_per_hns: u64,
    /// `H` (64 hex).
    pub hash: String,
    pub seller_hns_refund_pubkey: String,
    pub buyer_hns_claim_pubkey: String,
    pub buyer_btc_refund_pubkey: String,
    pub seller_btc_claim_pubkey: String,
    /// The relay keys the two parties sign messages with (66 hex).
    pub seller_relay_pubkey: String,
    pub buyer_relay_pubkey: String,
    /// Unix seconds.
    pub start: u64,
    pub buyer_fund_deadline: u64,
    pub seller_fund_deadline: u64,
    pub reveal_deadline: u64,
    pub hns_refund_time: u64,
    pub btc_refund_time: u64,
    pub btc_confirmations: u32,
    pub hns_confirmations: u32,
}

/// The times of a trade that starts at `start` under `p`.
pub struct Timeline {
    pub buyer_fund_deadline: u64,
    pub seller_fund_deadline: u64,
    pub reveal_deadline: u64,
    pub hns_refund_time: u64,
    pub btc_refund_time: u64,
}

pub fn timeline(p: &Policy, start: u64) -> Timeline {
    let hns_refund_time = start + p.hns_refund_after;
    Timeline {
        buyer_fund_deadline: start + p.buyer_fund_window,
        seller_fund_deadline: start + p.seller_fund_window,
        reveal_deadline: hns_refund_time - p.reveal_margin,
        hns_refund_time,
        btc_refund_time: hns_refund_time + p.btc_refund_after_hns,
    }
}

fn hex32(s: &str, what: &str) -> Result<[u8; 32], AppError> {
    let mut out = [0u8; 32];
    if s.len() != 64
        || hex::decode_to_slice(s, &mut out).is_err()
        || s.chars().any(|c| c.is_ascii_uppercase())
    {
        return Err(AppError::InvalidInput(format!(
            "{what} must be 64 lowercase hex characters"
        )));
    }
    Ok(out)
}

fn pubkey(s: &str, what: &str) -> Result<[u8; 33], AppError> {
    let mut out = [0u8; 33];
    if s.len() != 66
        || hex::decode_to_slice(s, &mut out).is_err()
        || s.chars().any(|c| c.is_ascii_uppercase())
    {
        return Err(AppError::InvalidInput(format!(
            "{what} must be a compressed public key"
        )));
    }
    secp256k1::PublicKey::from_slice(&out)
        .map_err(|_| AppError::InvalidInput(format!("{what} is not a valid public key")))?;
    Ok(out)
}

/// What a party needs to accept the terms: the offer it trades on.
pub struct OfferView<'a> {
    pub offer_id: &'a str,
    pub network: &'a str,
    pub price_msat_per_hns: u64,
    pub min_hns: u64,
    pub max_hns: u64,
}

impl Terms {
    /// The canonical encoding: compact JSON in declaration order.
    pub fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("terms always serialize")
    }

    /// SHA-256 of the canonical encoding, hex.
    pub fn id(&self) -> String {
        hex::encode(Sha256::digest(self.canonical()))
    }

    pub fn parse(json: &str) -> Result<Terms, AppError> {
        serde_json::from_str(json).map_err(|e| AppError::InvalidInput(format!("bad terms: {e}")))
    }

    pub fn network(&self) -> Result<Network, AppError> {
        Network::from_str_opt(&self.network)
            .ok_or_else(|| AppError::InvalidInput(format!("unknown network '{}'", self.network)))
    }

    pub fn hash_bytes(&self) -> Result<[u8; 32], AppError> {
        hex32(&self.hash, "the hash")
    }

    /// The HNS lock: the Buyer claims, the Seller refunds.
    pub fn hns_params(&self) -> Result<HtlcParams, AppError> {
        Ok(HtlcParams {
            hash: self.hash_bytes()?,
            claim_pubkey: pubkey(&self.buyer_hns_claim_pubkey, "the Buyer's HNS claim key")?,
            refund_pubkey: pubkey(
                &self.seller_hns_refund_pubkey,
                "the Seller's HNS refund key",
            )?,
            refund_lock_time: hns_htlc::encode_time_lock(self.hns_refund_time)?,
        })
    }

    /// The BTC lock: the Seller claims, the Buyer refunds.
    pub fn btc_params(&self) -> Result<HtlcParams, AppError> {
        Ok(HtlcParams {
            hash: self.hash_bytes()?,
            claim_pubkey: pubkey(&self.seller_btc_claim_pubkey, "the Seller's BTC claim key")?,
            refund_pubkey: pubkey(&self.buyer_btc_refund_pubkey, "the Buyer's BTC refund key")?,
            refund_lock_time: btc_htlc::encode_time_lock(self.btc_refund_time)?,
        })
    }

    pub fn seller_relay_key(&self) -> Result<[u8; 33], AppError> {
        pubkey(&self.seller_relay_pubkey, "the Seller's relay key")
    }

    pub fn buyer_relay_key(&self) -> Result<[u8; 33], AppError> {
        pubkey(&self.buyer_relay_pubkey, "the Buyer's relay key")
    }

    /// Every rule a party checks before it signs anything for these terms
    /// (R7). `now` is the local clock; `offer` the offer the trade takes.
    pub fn check(&self, policy: &Policy, offer: &OfferView<'_>, now: u64) -> Result<(), AppError> {
        let bad = |m: &str| {
            Err(AppError::InvalidInput(format!(
                "the trade's terms are refused: {m}"
            )))
        };
        if self.version != TERMS_VERSION {
            return bad("unknown version");
        }
        let network = self.network()?;
        if offer.network != self.network || network.as_str() != self.network {
            return bad("another network");
        }
        if self.offer_id != offer.offer_id {
            return bad("another offer");
        }
        hex32(&self.offer_id, "the offer id")?;
        if self.trade_id.len() != 32
            || hex::decode(&self.trade_id).is_err()
            || self.trade_id.chars().any(|c| c.is_ascii_uppercase())
        {
            return bad("the trade id must be 32 lowercase hex characters");
        }
        if self.price_msat_per_hns != offer.price_msat_per_hns {
            return bad("not the offer's price");
        }
        if self.hns_amount < offer.min_hns
            || self.hns_amount > offer.max_hns
            || self.hns_amount == 0
        {
            return bad("an amount outside the offer");
        }
        if self.btc_amount
            != crate::noncustodial::exchange::price::btc_for(
                self.hns_amount,
                self.price_msat_per_hns,
            )?
        {
            return bad("the BTC amount is not the HNS amount at the offer's price");
        }
        if self.btc_amount < policy.min_btc_sats {
            return bad("below the smallest trade");
        }
        if self.btc_amount > policy.max_btc_sats {
            return bad("above the largest trade");
        }
        let t = timeline(policy, self.start);
        if self.buyer_fund_deadline != t.buyer_fund_deadline
            || self.seller_fund_deadline != t.seller_fund_deadline
            || self.reveal_deadline != t.reveal_deadline
            || self.hns_refund_time != t.hns_refund_time
            || self.btc_refund_time != t.btc_refund_time
        {
            return bad("its times are not this network's windows");
        }
        if self.start + policy.clock_skew < now || self.start > now + policy.clock_skew {
            return bad("its start is too far from this computer's clock");
        }
        // Exactly the policy's counts (review L6): more would let one party
        // keep the other's coins locked until the refund times.
        if self.btc_confirmations != policy.btc_confirmations(self.btc_amount)
            || self.hns_confirmations != policy.hns_confirmations(self.btc_amount)
        {
            return bad("not this network's confirmation counts");
        }
        let keys = [
            pubkey(
                &self.seller_hns_refund_pubkey,
                "the Seller's HNS refund key",
            )?,
            pubkey(&self.buyer_hns_claim_pubkey, "the Buyer's HNS claim key")?,
            pubkey(&self.buyer_btc_refund_pubkey, "the Buyer's BTC refund key")?,
            pubkey(&self.seller_btc_claim_pubkey, "the Seller's BTC claim key")?,
            self.seller_relay_key()?,
            self.buyer_relay_key()?,
        ];
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                if keys[i] == keys[j] {
                    return bad("two of its keys are the same");
                }
            }
        }
        self.hns_params()?.check()?;
        self.btc_params()?.check()?;
        Ok(())
    }
}

/// Everything the two parties contribute to the terms; the rest (the BTC
/// amount, the times, the confirmations) follows from the policy.
pub struct TermsDraft<'a> {
    pub network: &'a str,
    pub offer_id: &'a str,
    pub trade_id: &'a str,
    pub hns_amount: u64,
    pub price_msat_per_hns: u64,
    pub hash: [u8; 32],
    pub seller_hns_refund_pubkey: [u8; 33],
    pub buyer_hns_claim_pubkey: [u8; 33],
    pub buyer_btc_refund_pubkey: [u8; 33],
    pub seller_btc_claim_pubkey: [u8; 33],
    pub seller_relay_pubkey: [u8; 33],
    pub buyer_relay_pubkey: [u8; 33],
    pub start: u64,
}

impl Terms {
    pub fn from_draft(d: &TermsDraft<'_>, p: &Policy) -> Result<Terms, AppError> {
        let btc_amount =
            crate::noncustodial::exchange::price::btc_for(d.hns_amount, d.price_msat_per_hns)?;
        let t = timeline(p, d.start);
        Ok(Terms {
            version: TERMS_VERSION,
            network: d.network.to_owned(),
            offer_id: d.offer_id.to_owned(),
            trade_id: d.trade_id.to_owned(),
            hns_amount: d.hns_amount,
            btc_amount,
            price_msat_per_hns: d.price_msat_per_hns,
            hash: hex::encode(d.hash),
            seller_hns_refund_pubkey: hex::encode(d.seller_hns_refund_pubkey),
            buyer_hns_claim_pubkey: hex::encode(d.buyer_hns_claim_pubkey),
            buyer_btc_refund_pubkey: hex::encode(d.buyer_btc_refund_pubkey),
            seller_btc_claim_pubkey: hex::encode(d.seller_btc_claim_pubkey),
            seller_relay_pubkey: hex::encode(d.seller_relay_pubkey),
            buyer_relay_pubkey: hex::encode(d.buyer_relay_pubkey),
            start: d.start,
            buyer_fund_deadline: t.buyer_fund_deadline,
            seller_fund_deadline: t.seller_fund_deadline,
            reveal_deadline: t.reveal_deadline,
            hns_refund_time: t.hns_refund_time,
            btc_refund_time: t.btc_refund_time,
            btc_confirmations: p.btc_confirmations(btc_amount),
            hns_confirmations: p.hns_confirmations(btc_amount),
        })
    }
}
