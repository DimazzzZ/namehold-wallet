//! In-memory signer session: holds unlocked key material for the active wallet.
//!
//! Security model:
//!   - The unlocked master key lives ONLY in memory, never on disk. The only
//!     persisted form of the secret is the encrypted vault blob (see `vault`).
//!   - The session has an absolute expiry (`unlocked_until_ms`); once elapsed,
//!     `master()` returns `WalletLocked` and the material should be wiped.
//!   - On `lock()` / `Drop`, the secret bytes are zeroized.
//!
//! This module deliberately does NOT touch the database or filesystem. Callers
//! decrypt a vault blob, derive the master key, and hand the resulting
//! `ExtendedPrivKey` to `unlock()`. Locking is the caller's/timer's job.

use crate::error::AppError;
use crate::noncustodial::hd::ExtendedPrivKey;
use crate::noncustodial::network::Network;

/// Current wall-clock time in milliseconds since the Unix epoch.
fn now_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// An unlocked signer session for exactly one wallet profile.
///
/// `ExtendedPrivKey` carries the secp256k1 `SecretKey` (which zeroizes itself on
/// drop) and a `chain_code` that its explicit `Drop` impl zeroizes, so dropping
/// the session wipes all key material.
pub struct SignerSession {
    /// Identifier of the wallet profile this session unlocks.
    wallet_profile_id: i64,
    /// The network this wallet operates on.
    network: Network,
    /// The unlocked BIP32 master key. `None` once locked.
    master: Option<ExtendedPrivKey>,
    /// Absolute expiry in epoch milliseconds. After this, the session is locked.
    unlocked_until_ms: u128,
}

impl SignerSession {
    /// Create an unlocked session valid for `ttl_ms` from now.
    pub fn unlock(
        wallet_profile_id: i64,
        network: Network,
        master: ExtendedPrivKey,
        ttl_ms: u128,
    ) -> Self {
        Self {
            wallet_profile_id,
            network,
            master: Some(master),
            unlocked_until_ms: now_ms().saturating_add(ttl_ms),
        }
    }

    pub fn wallet_profile_id(&self) -> i64 {
        self.wallet_profile_id
    }

    pub fn network(&self) -> Network {
        self.network
    }

    /// Whether the session is currently unlocked AND not expired.
    pub fn is_unlocked(&self) -> bool {
        self.master.is_some() && now_ms() < self.unlocked_until_ms
    }

    /// Borrow the unlocked master key, or `WalletLocked` if locked/expired.
    ///
    /// If the session has expired, this also wipes the key material as a side
    /// effect so it cannot be used afterward.
    pub fn master(&mut self) -> Result<&ExtendedPrivKey, AppError> {
        if now_ms() >= self.unlocked_until_ms {
            self.lock();
            return Err(AppError::WalletLocked);
        }
        self.master.as_ref().ok_or(AppError::WalletLocked)
    }

    /// Extend the session expiry to `ttl_ms` from now (e.g. on user activity).
    /// No-op if already locked.
    pub fn touch(&mut self, ttl_ms: u128) {
        if self.master.is_some() {
            self.unlocked_until_ms = now_ms().saturating_add(ttl_ms);
        }
    }

    /// Lock the session, dropping (and thereby zeroizing) the key material.
    pub fn lock(&mut self) {
        // Dropping the ExtendedPrivKey zeroizes secret + chain code.
        self.master = None;
        self.unlocked_until_ms = 0;
    }
}

impl Drop for SignerSession {
    fn drop(&mut self) {
        self.lock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::ExtendedPrivKey;

    fn test_master() -> ExtendedPrivKey {
        // BIP32 vector-1 seed.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        ExtendedPrivKey::from_seed(&seed).expect("master")
    }

    #[test]
    fn unlocked_session_exposes_master() {
        let mut s = SignerSession::unlock(1, Network::Main, test_master(), 60_000);
        assert!(s.is_unlocked());
        assert_eq!(s.wallet_profile_id(), 1);
        assert!(s.master().is_ok());
    }

    #[test]
    fn locked_session_denies_master() {
        let mut s = SignerSession::unlock(1, Network::Main, test_master(), 60_000);
        s.lock();
        assert!(!s.is_unlocked());
        assert!(matches!(s.master(), Err(AppError::WalletLocked)));
    }

    #[test]
    fn expired_session_denies_master() {
        // ttl_ms = 0 means already expired.
        let mut s = SignerSession::unlock(1, Network::Main, test_master(), 0);
        assert!(!s.is_unlocked());
        assert!(matches!(s.master(), Err(AppError::WalletLocked)));
    }

    #[test]
    fn touch_extends_expiry() {
        let mut s = SignerSession::unlock(1, Network::Main, test_master(), 0);
        assert!(!s.is_unlocked());
        s.touch(60_000);
        assert!(s.is_unlocked());
        assert!(s.master().is_ok());
    }
}
