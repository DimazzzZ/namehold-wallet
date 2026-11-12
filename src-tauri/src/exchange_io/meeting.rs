//! Where offers meet takers and the two sides of a trade talk (spec
//! 2026-11-12 R11): the Nostr relays by default, or a custom
//! `namehold-relay`. The job and the commands reach it only through
//! [`MeetingPoint`]; the watchtower, which only a `namehold-relay` offers, is
//! its own trait.
//!
//! Every method that writes takes the relay key that signed what it writes:
//! a Nostr event must be signed by it, and a `namehold-relay` ignores it.

use async_trait::async_trait;
use secp256k1::SecretKey;

use crate::error::AppError;
use crate::noncustodial::exchange::relay_msg::{Envelope, SignedOffer};
use crate::noncustodial::exchange::watch::WatchJob;

#[async_trait]
pub trait MeetingPoint: Send + Sync {
    /// Publish, or renew (the heartbeat), an offer `key` signed.
    async fn publish_offer(&self, o: &SignedOffer, key: &SecretKey) -> Result<(), AppError>;
    /// Withdraw an offer; `signature` is `relay_msg::sign_withdraw`'s.
    async fn withdraw_offer(
        &self,
        o: &SignedOffer,
        signature: &str,
        key: &SecretKey,
    ) -> Result<(), AppError>;
    /// Live offers of `network`, each verified; the ones that fail are
    /// dropped and counted.
    async fn offers(&self, network: &str, now: u64) -> Result<(Vec<SignedOffer>, usize), AppError>;
    /// Send a message `key` signed.
    async fn send(&self, e: &Envelope, key: &SecretKey) -> Result<(), AppError>;
    /// Messages to `key`'s public key after `cursor`, each verified; returns
    /// the cursor to store.
    async fn inbox(
        &self,
        key: &SecretKey,
        network: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Envelope>, Option<String>), AppError>;
}

#[async_trait]
pub trait Watchtower: Send + Sync {
    async fn watch(&self, job: &WatchJob) -> Result<String, AppError>;
}

/// Cursors the Nostr meeting point writes start with this, so a cursor of
/// one meeting point is never read as the other's after the user switches.
pub const NOSTR_CURSOR_PREFIX: &str = "nostr:";
