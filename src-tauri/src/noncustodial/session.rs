//! In-memory signer session: holds unlocked key material for the active wallet.
//!
//! Security model:
//!   - The unlocked master key lives ONLY in memory, never on disk. The only
//!     persisted form of the secret is the encrypted vault blob (see `vault`).
//!   - A session expires `ttl` after its unlock or its last authorized use
//!     (`signer_session_timeout_seconds`, clamped to
//!     `MIN_SESSION_SECS..=MAX_SESSION_SECS`), and never later than
//!     `MAX_SESSION_SECS` after the unlock, however often it is used: a
//!     command that authorizes without asking the user cannot keep it open.
//!   - The first check that finds a session expired (`is_unlocked`,
//!     `authorize`, `master`) drops the key there and then. The app checks
//!     the slot every second (`secure_wallet::sweep_expired_signer`, spawned
//!     in `lib.rs`), so an expired key does not wait for the next command.
//!   - `lock()` and `Drop` drop the key in place, and `ExtendedPrivKey`'s
//!     `Drop` overwrites its secret key and chain code.
//!
//! This module deliberately does NOT touch the database or filesystem. Callers
//! decrypt a vault blob, derive the master key, and hand the resulting
//! `ExtendedPrivKey` to `unlock()`.

use crate::error::AppError;
use crate::noncustodial::hd::ExtendedPrivKey;
use crate::noncustodial::network::Network;
use std::collections::HashMap;

/// Unlock lifetime when `signer_session_timeout_seconds` is unset, empty,
/// zero or unreadable: 15 minutes.
const DEFAULT_SESSION_SECS: u64 = 900;

/// The shortest timeout honoured; a shorter one is raised to it.
pub const MIN_SESSION_SECS: u64 = 60;

/// The longest timeout honoured, and the longest a session lasts after its
/// unlock however often it is used. The setting is written by the renderer
/// (SECURITY.md §1), so without a bound a compromised webview could store a
/// timeout that never ends for every later unlock.
pub const MAX_SESSION_SECS: u64 = 3600;

const MAX_SESSION_MS: u128 = MAX_SESSION_SECS as u128 * 1000;

/// How long an unlock lasts after its last use, in milliseconds, read from
/// the `signer_session_timeout_seconds` setting and clamped to
/// `MIN_SESSION_SECS..=MAX_SESSION_SECS`. A zero, empty or unreadable value
/// gets the default rather than a session that is over at once.
pub fn session_ttl_ms(settings: &HashMap<String, String>) -> u128 {
    let secs = settings
        .get("signer_session_timeout_seconds")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        // Why: unset, zero or not a number is no timeout the user chose.
        .unwrap_or(DEFAULT_SESSION_SECS)
        .clamp(MIN_SESSION_SECS, MAX_SESSION_SECS);
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
pub struct SignerSession {
    /// Identifier of the wallet profile this session unlocks. Matches the TEXT
    /// `wallet_profiles.id`.
    wallet_profile_id: String,
    /// The network this wallet operates on.
    network: Network,
    /// The unlocked BIP32 master key. `None` once locked.
    master: Option<ExtendedPrivKey>,
    /// Expiry in epoch milliseconds; at or after it the session is locked.
    /// 0 once locked.
    unlocked_until_ms: u128,
    /// The latest expiry any use can move `unlocked_until_ms` to: the unlock
    /// time plus `MAX_SESSION_SECS`.
    hard_expiry_ms: u128,
}

impl SignerSession {
    /// Create an unlocked session valid for `ttl_ms` from now (at most
    /// `MAX_SESSION_SECS`).
    pub fn unlock(
        wallet_profile_id: String,
        network: Network,
        master: ExtendedPrivKey,
        ttl_ms: u128,
    ) -> Self {
        let now = now_ms();
        let hard_expiry_ms = now.saturating_add(MAX_SESSION_MS);
        Self {
            wallet_profile_id,
            network,
            master: Some(master),
            unlocked_until_ms: now.saturating_add(ttl_ms).min(hard_expiry_ms),
            hard_expiry_ms,
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

    /// Whether the session is unlocked and not expired. An expired session is
    /// locked here, so its key is dropped at the first check that sees it.
    pub fn is_unlocked(&mut self) -> bool {
        if self.master.is_some() && now_ms() < self.unlocked_until_ms {
            return true;
        }
        self.lock();
        false
    }

    /// Borrow the unlocked master key, or `WalletLocked` if locked/expired
    /// (an expired session is locked by the check).
    pub fn master(&mut self) -> Result<&ExtendedPrivKey, AppError> {
        if !self.is_unlocked() {
            return Err(AppError::WalletLocked);
        }
        self.master.as_ref().ok_or(AppError::WalletLocked)
    }

    /// Move the expiry to `ttl_ms` from now, never past the hard expiry. An
    /// expired session is locked instead, never revived.
    fn touch(&mut self, ttl_ms: u128) {
        if self.is_unlocked() {
            self.unlocked_until_ms = now_ms().saturating_add(ttl_ms).min(self.hard_expiry_ms);
        }
    }

    /// Whether this session may act for `profile_id` now: unlocked, not
    /// expired (an expired one is locked by the check), and `profile_id`'s
    /// (one wallet's unlocked signer never acts for another). Moves nothing.
    pub fn ensure_for(&mut self, profile_id: &str) -> Result<(), AppError> {
        if !self.is_unlocked() {
            return Err(AppError::WalletLocked);
        }
        if self.wallet_profile_id != profile_id {
            return Err(AppError::InvalidInput(
                "the unlocked signer is for a different wallet profile".to_string(),
            ));
        }
        Ok(())
    }

    /// The gate every use of the unlocked key passes: [`Self::ensure_for`],
    /// then the expiry moves `ttl_ms` ahead, never past `MAX_SESSION_SECS`
    /// after the unlock.
    pub fn authorize(&mut self, profile_id: &str, ttl_ms: u128) -> Result<(), AppError> {
        self.ensure_for(profile_id)?;
        self.touch(ttl_ms);
        Ok(())
    }

    /// Lock the session: the key is dropped in place, which overwrites its
    /// secret key and chain code (`ExtendedPrivKey`'s `Drop`).
    pub fn lock(&mut self) {
        self.master = None;
        self.unlocked_until_ms = 0;
    }
}

impl Drop for SignerSession {
    fn drop(&mut self) {
        self.lock();
    }
}

/// Empty `slot` when its session has expired, dropping the key; returns
/// whether it did. What `get_signer_session` and the app's once-a-second
/// sweep run.
pub fn expire_slot(slot: &mut Option<SignerSession>) -> bool {
    if slot.as_mut().is_some_and(|s| !s.is_unlocked()) {
        *slot = None;
        return true;
    }
    false
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
    fn touch_extends_a_live_session() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 1_000);
        let before = s.unlocked_until_ms();
        s.touch(60_000);
        assert!(s.unlocked_until_ms() >= before + 59_000);
        assert!(s.is_unlocked());
    }

    /// An elapsed time box stays elapsed: touching an expired session locks
    /// it (dropping the key) instead of bringing it back.
    #[test]
    fn touch_does_not_revive_an_expired_session() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 0);
        s.touch(60_000);
        assert!(s.master.is_none(), "the expired key must be dropped");
        assert_eq!(s.unlocked_until_ms(), 0);
        assert!(!s.is_unlocked());
        assert!(matches!(s.master(), Err(AppError::WalletLocked)));
    }

    #[test]
    fn is_unlocked_on_an_expired_session_drops_the_key() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 0);
        let before = crate::noncustodial::hd::wipes_on_this_thread();
        assert!(!s.is_unlocked());
        assert!(s.master.is_none(), "the expired key must be dropped");
        assert_eq!(crate::noncustodial::hd::wipes_on_this_thread(), before + 1);
        assert_eq!(s.unlocked_until_ms(), 0);
    }

    #[test]
    fn authorize_on_an_expired_session_drops_the_key() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 0);
        assert!(matches!(
            s.authorize("p1", 60_000),
            Err(AppError::WalletLocked)
        ));
        assert!(s.master.is_none(), "the expired key must be dropped");
        assert_eq!(s.unlocked_until_ms(), 0);
    }

    #[test]
    fn ensure_for_checks_without_moving_the_expiry() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 1_000);
        let before = s.unlocked_until_ms();
        s.ensure_for("p1").unwrap();
        assert_eq!(s.unlocked_until_ms(), before);
        assert!(matches!(s.ensure_for("p2"), Err(AppError::InvalidInput(_))));
        let mut expired = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 0);
        assert!(matches!(
            expired.ensure_for("p1"),
            Err(AppError::WalletLocked)
        ));
        assert!(expired.master.is_none());
    }

    /// However large the timeout handed in, an unlock lasts at most
    /// `MAX_SESSION_SECS`.
    #[test]
    fn unlock_never_sets_an_expiry_past_the_cap() {
        let s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), u128::MAX);
        assert_eq!(s.unlocked_until_ms(), s.hard_expiry_ms);
        assert!(s.hard_expiry_ms <= now_ms() + MAX_SESSION_MS);
    }

    /// A command that authorizes without asking the user (a Shakedex lock
    /// search, say) moves the expiry, but never past `MAX_SESSION_SECS`
    /// after the unlock: polling it cannot keep the key in memory for ever.
    #[test]
    fn authorize_never_slides_past_the_absolute_cap() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        // As if the unlock was nearly an hour ago: 5 s of the cap are left.
        s.hard_expiry_ms = now_ms() + 5_000;
        s.authorize("p1", MAX_SESSION_MS).unwrap();
        assert_eq!(s.unlocked_until_ms(), s.hard_expiry_ms);
        // Once the cap has passed, the next use finds the session expired.
        s.hard_expiry_ms = now_ms().saturating_sub(1);
        s.unlocked_until_ms = s.hard_expiry_ms;
        assert!(matches!(
            s.authorize("p1", MAX_SESSION_MS),
            Err(AppError::WalletLocked)
        ));
        assert!(s.master.is_none());
    }

    /// `lock()` drops the key through `ExtendedPrivKey`'s wiping `Drop`
    /// (`hd::tests::dropping_an_extended_priv_key_erases_the_secret` shows
    /// what that overwrites), never only forgets it.
    #[test]
    fn lock_wipes_the_master_key() {
        let mut s = SignerSession::unlock("p1".to_string(), Network::Main, test_master(), 60_000);
        let before = crate::noncustodial::hd::wipes_on_this_thread();
        s.lock();
        assert!(s.master.is_none());
        assert_eq!(crate::noncustodial::hd::wipes_on_this_thread(), before + 1);
    }

    #[test]
    fn expire_slot_empties_an_expired_slot_only() {
        let mut none: Option<SignerSession> = None;
        assert!(!expire_slot(&mut none));
        let mut live = Some(SignerSession::unlock(
            "p1".to_string(),
            Network::Main,
            test_master(),
            60_000,
        ));
        assert!(!expire_slot(&mut live));
        assert!(live.is_some());
        let mut expired = Some(SignerSession::unlock(
            "p1".to_string(),
            Network::Main,
            test_master(),
            0,
        ));
        assert!(expire_slot(&mut expired));
        assert!(expired.is_none());
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

    fn ttl_for(value: &str) -> u128 {
        let mut settings = HashMap::new();
        settings.insert(
            "signer_session_timeout_seconds".to_string(),
            value.to_string(),
        );
        session_ttl_ms(&settings)
    }

    /// The setting is renderer-writable: a huge value is cut to the cap, so a
    /// compromised webview cannot store a session that never ends.
    #[test]
    fn session_ttl_ms_clamps_to_the_cap() {
        assert_eq!(
            ttl_for("18446744073709551615"),
            u128::from(MAX_SESSION_SECS) * 1000
        );
        assert_eq!(ttl_for("86400"), 3_600_000);
        assert_eq!(ttl_for("3601"), 3_600_000);
        assert_eq!(ttl_for("3600"), 3_600_000);
    }

    /// Settings accepts the value with stray spaces and saves it as typed.
    #[test]
    fn session_ttl_ms_reads_a_value_with_surrounding_spaces() {
        assert_eq!(ttl_for(" 1800 "), 1_800_000);
    }

    #[test]
    fn session_ttl_ms_raises_a_short_timeout_to_the_floor() {
        assert_eq!(ttl_for("1"), 60_000);
        assert_eq!(ttl_for("59"), 60_000);
        assert_eq!(ttl_for("60"), u128::from(MIN_SESSION_SECS) * 1000);
    }

    #[test]
    fn session_ttl_ms_falls_back_when_setting_is_negative_or_too_long_to_read() {
        assert_eq!(ttl_for("-5"), 900_000);
        assert_eq!(ttl_for("99999999999999999999999"), 900_000);
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
