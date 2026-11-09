//! The timing, confirmation, limit and fee rules of a trade (spec §3.5).
//!
//! Every time in the terms is derived from the trade's start by these
//! windows, and both parties recompute them: a counterparty that proposes
//! other times is refused (`terms::Terms::check`). Mainnet and testnet use
//! hours (10-minute blocks on both chains); regtest and simnet use minutes,
//! for tests.

use crate::noncustodial::network::Network;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// The Buyer must have funded the BTC lock within this many seconds of
    /// the start, or the Seller does not wait for it.
    pub buyer_fund_window: u64,
    /// The Seller's funding deadline: start + this.
    pub seller_fund_window: u64,
    /// The HNS lock's refund time: start + this.
    pub hns_refund_after: u64,
    /// The Buyer never reveals later than the HNS refund time − this.
    pub reveal_margin: u64,
    /// The BTC lock's refund time: the HNS refund time + this. It is the
    /// least time the Seller has to claim once the secret is public.
    pub btc_refund_after_hns: u64,
    /// How far apart the two parties' clocks may be about the start.
    pub clock_skew: u64,
    /// Per-trade limits, in satoshis.
    pub min_btc_sats: u64,
    pub max_btc_sats: u64,
    /// Confirmations: below `large_from_sats` the small counts, from it the
    /// large ones.
    pub large_from_sats: u64,
    pub btc_confirmations_small: u32,
    pub btc_confirmations_large: u32,
    pub hns_confirmations_small: u32,
    pub hns_confirmations_large: u32,
    /// Bitcoin fee ladder: the next rung after this many seconds without
    /// the previous one mined (3 blocks)…
    pub ladder_escalate_after: u64,
    /// …and the top rung once fewer than this many seconds are left before
    /// the competing path opens.
    pub ladder_top_before: u64,
    /// The ladder's multipliers of the rate at Commit.
    pub ladder_multipliers: &'static [u64],
    /// No rung pays more than this percentage of the coin it spends.
    pub max_fee_percent: u64,
    /// The least Bitcoin rate a rung starts from, sat/vbyte.
    pub btc_min_rate: u64,
    /// HNS presigned spends pay this multiple of the node's rate, at least
    /// `hns_min_rate` doos/vbyte (hsd has no replace-by-fee).
    pub hns_rate_multiplier: u64,
    pub hns_min_rate: u64,
    /// Refuse a Commit when the local clock and the chain's median time
    /// disagree by more than this (None: do not check, regtest).
    pub max_clock_drift: Option<u64>,
    /// A BTC lock is proven only with at least this many Bitcoin sources
    /// (spec §1, §3.6: "two independent sources"), every one agreeing.
    pub min_btc_sources: usize,
}

const HOUR: u64 = 3600;
const MINUTE: u64 = 60;

pub const LADDER: &[u64] = &[1, 2, 4, 8, 16, 32];

impl Policy {
    pub fn for_network(network: Network) -> Policy {
        match network {
            Network::Main | Network::Testnet => Policy {
                buyer_fund_window: HOUR,
                seller_fund_window: 4 * HOUR,
                hns_refund_after: 12 * HOUR,
                reveal_margin: 3 * HOUR,
                btc_refund_after_hns: 12 * HOUR,
                clock_skew: 10 * MINUTE,
                // The fee cap is 20 % of the coin: at 50 000 sats a claim may
                // pay up to ~66 sat/vB through a fee spike (review M4).
                min_btc_sats: 50_000,
                max_btc_sats: 10_000_000,
                large_from_sats: 2_000_000,
                btc_confirmations_small: 2,
                btc_confirmations_large: 3,
                hns_confirmations_small: 6,
                hns_confirmations_large: 12,
                ladder_escalate_after: 30 * MINUTE,
                ladder_top_before: 6 * HOUR,
                ladder_multipliers: LADDER,
                max_fee_percent: 20,
                btc_min_rate: 2,
                hns_rate_multiplier: 20,
                hns_min_rate: 20,
                max_clock_drift: Some(3 * HOUR),
                min_btc_sources: 2,
            },
            Network::Regtest | Network::Simnet => Policy {
                buyer_fund_window: 10 * MINUTE,
                seller_fund_window: 20 * MINUTE,
                hns_refund_after: 60 * MINUTE,
                reveal_margin: 15 * MINUTE,
                btc_refund_after_hns: 60 * MINUTE,
                clock_skew: 5 * MINUTE,
                min_btc_sats: 1_000,
                max_btc_sats: 1_000_000_000,
                large_from_sats: 100_000_000,
                btc_confirmations_small: 1,
                btc_confirmations_large: 1,
                hns_confirmations_small: 1,
                hns_confirmations_large: 1,
                ladder_escalate_after: 5 * MINUTE,
                ladder_top_before: 10 * MINUTE,
                ladder_multipliers: LADDER,
                max_fee_percent: 20,
                btc_min_rate: 1,
                hns_rate_multiplier: 20,
                hns_min_rate: 20,
                max_clock_drift: None,
                // Regtest has no public source: one local bitcoind.
                min_btc_sources: 1,
            },
        }
    }

    pub fn btc_confirmations(&self, btc_sats: u64) -> u32 {
        if btc_sats >= self.large_from_sats {
            self.btc_confirmations_large
        } else {
            self.btc_confirmations_small
        }
    }

    pub fn hns_confirmations(&self, btc_sats: u64) -> u32 {
        if btc_sats >= self.large_from_sats {
            self.hns_confirmations_large
        } else {
            self.hns_confirmations_small
        }
    }

    /// The safety invariants the windows must keep; checked by a test for
    /// every network. Returns the first broken one.
    pub fn broken_invariant(&self) -> Option<&'static str> {
        if self.buyer_fund_window == 0 || self.seller_fund_window <= self.buyer_fund_window {
            return Some("the Seller must have longer to fund than the Buyer");
        }
        if self.reveal_margin == 0 || self.reveal_margin >= self.hns_refund_after {
            return Some("the reveal margin must be inside the HNS lock's life");
        }
        if self.hns_refund_after <= self.seller_fund_window + self.reveal_margin {
            return Some(
                "the HNS lock needs time for its confirmations before the reveal deadline",
            );
        }
        if self.btc_refund_after_hns <= self.reveal_margin {
            return Some("the Seller's claim window must outlast the reveal margin");
        }
        if self.ladder_top_before >= self.btc_refund_after_hns {
            return Some("the top rung must start inside the claim window");
        }
        if self.ladder_multipliers.is_empty() || self.ladder_multipliers[0] != 1 {
            return Some("the ladder starts at the rate at Commit");
        }
        if !self.ladder_multipliers.windows(2).all(|w| w[0] < w[1]) {
            return Some("the ladder's rates must rise");
        }
        if self.min_btc_sats == 0 || self.max_btc_sats < self.min_btc_sats {
            return Some("the limits are empty");
        }
        None
    }
}
