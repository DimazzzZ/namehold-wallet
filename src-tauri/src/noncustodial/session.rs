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
use std::collections::HashMap;

/// Unlock lifetime when `signer_session_timeout_seconds` is unset, empty,
/// zero or unreadable: 15 minutes.
const DEFAULT_SESSION_SECS: u64 = 900;

/// How long an unlock lasts, in milliseconds, read from the
/// `signer_session_timeout_seconds` setting. A zero, empty or unreadable value
/// gets the default rather than a session that is over at once.
pub fn session_ttl_ms(settings: &HashMap<String, String>) -> u128 {
    let secs = settings
        .get("signer_session_timeout_seconds")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SESSION_SECS);
    u128::from(secs) * 1000
}

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
    /// Identifier of the wallet profile this session unlocks. Matches the TEXT
    /// `wallet_profiles.id`.
    wallet_profile_id: String,
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
        wallet_profile_id: String,
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

    pub fn wallet_profile_id(&self) -> &str {
        &self.wallet_profile_id
    }

    /// Absolute expiry in epoch milliseconds (0 once locked).
    pub fn unlocked_until_ms(&self) -> u128 {
        self.unlocked_until_ms
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

    /// The gate every use of the unlocked key passes: the session is
    /// unlocked and not expired, it is `profile_id`'s (one wallet's unlocked
    /// signer never acts for another), and its expiry moves `ttl_ms` ahead.
    pub fn authorize(&mut self, profile_id: &str, ttl_ms: u128) -> Result<(), AppError> {
        if !self.is_unlocked() {
            return Err(AppError::WalletLocked);
        }
        if self.wallet_profile_id != profile_id {
            return Err(AppError::InvalidInput(
                "the unlocked signer is for a different wallet profile".to_string(),
            ));
        }
        self.touch(ttl_ms);
        Ok(())
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
    use std::collections::HashMap;

    fn test_master() -> ExtendedPrivKey {
        // BIP32 vector-1 seed.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        ExtendedPrivKey::from_seed(&seed).expect("master")
    }

    #[test]
    fn unlocked_session_exposes_master() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        assert!(s.is_unlocked());
        assert_eq!(s.wallet_profile_id(), "p1");
        assert!(s.master().is_ok());
    }

    #[test]
    fn locked_session_denies_master() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        s.lock();
        assert!(!s.is_unlocked());
        assert!(matches!(s.master(), Err(AppError::WalletLocked)));
    }

    #[test]
    fn expired_session_denies_master() {
        // ttl_ms = 0 means already expired.
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 0);
        assert!(!s.is_unlocked());
        assert!(matches!(s.master(), Err(AppError::WalletLocked)));
    }

    #[test]
    fn touch_extends_expiry() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 0);
        assert!(!s.is_unlocked());
        s.touch(60_000);
        assert!(s.is_unlocked());
        assert!(s.master().is_ok());
    }

    /// `touch()` is a no-op on a locked session (does not panic or error).
    #[test]
    fn touch_on_locked_session_is_noop() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        s.lock();
        assert!(!s.is_unlocked());
        let expiry_before = s.unlocked_until_ms();
        s.touch(60_000);
        // Expiry should remain 0 (locked).
        assert_eq!(s.unlocked_until_ms(), expiry_before);
        assert_eq!(s.unlocked_until_ms(), 0);
    }

    #[test]
    fn authorize_refuses_a_locked_session() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        s.lock();
        assert!(matches!(
            s.authorize("p1", 60_000),
            Err(AppError::WalletLocked)
        ));
    }

    #[test]
    fn authorize_refuses_another_profiles_session() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        match s.authorize("p2", 60_000) {
            Err(AppError::InvalidInput(m)) => {
                assert!(m.contains("different wallet profile"), "{m}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn authorize_moves_the_expiry_ahead() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 1_000);
        let before = s.unlocked_until_ms();
        s.authorize("p1", 600_000).unwrap();
        assert!(s.unlocked_until_ms() >= before + 590_000);
    }

    // ---------- session_ttl_ms --------------------------------------------

    #[test]
    fn session_ttl_ms_default_when_setting_absent() {
        let settings: HashMap<String, String> = HashMap::new();
        // Default 900 seconds → 900_000 ms.
        assert_eq!(session_ttl_ms(&settings), 900_000u128);
    }

    #[test]
    fn session_ttl_ms_reads_valid_numeric_setting() {
        let mut settings = HashMap::new();
        settings.insert(
            "signer_session_timeout_seconds".to_string(),
            "60".to_string(),
        );
        assert_eq!(session_ttl_ms(&settings), 60_000u128);
    }

    #[test]
    fn session_ttl_ms_falls_back_when_setting_is_non_numeric() {
        let mut settings = HashMap::new();
        settings.insert(
            "signer_session_timeout_seconds".to_string(),
            "not-a-number".to_string(),
        );
        assert_eq!(session_ttl_ms(&settings), 900_000u128);
    }

    #[test]
    fn session_ttl_ms_falls_back_when_setting_is_empty_string() {
        let mut settings = HashMap::new();
        settings.insert("signer_session_timeout_seconds".to_string(), String::new());
        assert_eq!(session_ttl_ms(&settings), 900_000u128);
    }

    #[test]
    fn session_ttl_ms_falls_back_when_setting_is_zero() {
        // Zero is filtered out (`filter(|n| *n > 0)`), so we still get the
        // default rather than a 0-ms TTL that would time out immediately.
        let mut settings = HashMap::new();
        settings.insert(
            "signer_session_timeout_seconds".to_string(),
            "0".to_string(),
        );
        assert_eq!(session_ttl_ms(&settings), 900_000u128);
    }
}
