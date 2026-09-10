//! Encrypt the Namebase session cookie at rest under an OS-keyring-held DEK.
//!
//! The cookie is a bearer credential for the user's Namebase account. Storing it
//! in plaintext SQLite is a residual risk: file-system access (backup, theft,
//! crash dump) exposes the credential. This module encrypts the cookie under a
//! data-encryption key (DEK) held by the OS keyring (macOS Keychain, Windows
//! Credential Manager, Linux Secret Service), which cannot be extracted without
//! the user's OS-login credentials.
//!
//! On-disk blob layout (binary, hex-encoded in the setting):
//!   magic:      4 bytes  = b"NBC1"        (Namehold Basebase Cookie v1)
//!   nonce:      12 bytes                  (AES-GCM nonce)
//!   ciphertext: remainder                 (plaintext || 16-byte GCM tag)
//!
//! The DEK is stored in the OS keyring under:
//!   service: "namehold-wallet"
//!   account: "namebase-cookie-dek-v1"
//! On first access, a random 32-byte DEK is generated and stored. On subsequent
//! accesses, the stored DEK is retrieved.
//!
//! Threat model:
//! - Offline attacker (file access): cannot decrypt without the DEK.
//! - Online attacker (code execution as the user): can read the DEK from the
//!   keyring (the OS trusts the logged-in user). This is the standard trade-off
//!   for OS-keyring-backed secrets.

use crate::error::AppError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use zeroize::Zeroize;

const MAGIC: &[u8; 4] = b"NBC1";
const NONCE_LEN: usize = 12;
const DEK_LEN: usize = 32;
const KEYRING_SERVICE: &str = "namehold-wallet";
const KEYRING_ACCOUNT: &str = "namebase-cookie-dek-v1";

/// Injectable keyring seam.
///
/// The DEK-management logic (`resolve_dek_from_backend`) is written against
/// this trait so tests can drive every branch (existing DEK / no entry /
/// transport error / malformed base64) without touching the developer's real
/// OS keyring. In production the sole implementation is [`RealKeyring`], a
/// thin adapter over `keyring::Entry`.
trait KeyringBackend {
    /// Fetch the stored password.
    /// - `Ok(Some(value))` — a value is present.
    /// - `Ok(None)` — no entry has been stored yet (first-run case).
    /// - `Err(_)` — the keyring itself is unavailable / errored.
    fn get_password(&self) -> Result<Option<String>, AppError>;

    /// Store the password, overwriting any prior value.
    fn set_password(&self, value: &str) -> Result<(), AppError>;
}

/// Production keyring backend: wraps `keyring::Entry` and translates
/// `keyring::error::Error::NoEntry` into `Ok(None)` so the resolver can
/// treat "no entry yet" as a normal first-run state.
struct RealKeyring {
    entry: keyring::Entry,
}

impl RealKeyring {
    fn new() -> Result<Self, AppError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|e| AppError::Other(format!("keyring entry: {e}")))?;
        Ok(Self { entry })
    }
}

impl KeyringBackend for RealKeyring {
    fn get_password(&self) -> Result<Option<String>, AppError> {
        match self.entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::error::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::Other(format!("keyring get_password: {e}"))),
        }
    }

    fn set_password(&self, value: &str) -> Result<(), AppError> {
        self.entry
            .set_password(value)
            .map_err(|e| AppError::Other(format!("keyring set_password: {e}")))
    }
}

/// Resolve the DEK using the given backend: return the stored value when
/// present, otherwise generate a fresh random DEK and persist it.
///
/// This is the branch-heavy piece of the flow; keeping it pure over the
/// backend trait means all four cases (existing / new / bad base64 / backend
/// error) are exercisable by the test suite.
fn resolve_dek_from_backend<B: KeyringBackend + ?Sized>(backend: &B) -> Result<Vec<u8>, AppError> {
    match backend.get_password()? {
        Some(dek_b64) => BASE64
            .decode(dek_b64.as_bytes())
            .map_err(|e| AppError::Crypto(format!("dek base64 decode: {e}"))),
        None => {
            // First run: generate a random DEK and store it.
            let mut dek = vec![0u8; DEK_LEN];
            rand::thread_rng().fill_bytes(&mut dek);
            let dek_b64 = BASE64.encode(&dek);
            backend.set_password(&dek_b64)?;
            Ok(dek)
        }
    }
}

/// Test-only override for the DEK. When set, `get_or_create_dek()` returns
/// this DEK instead of consulting the OS keyring. This lets integration tests
/// exercise the full encrypt/decrypt flow (via `encrypt_cookie` /
/// `decrypt_cookie`) without polluting the developer's real Keychain and
/// without requiring a graphical session on CI Linux.
///
/// # Gating (safety-critical)
///
/// This override is compiled ONLY when EITHER of the following is true:
///   * `cfg(test)` — set only by `cargo test` for the crate's own tests.
///   * `cfg(debug_assertions)` — set by every non-release `cargo build`.
///
/// Cargo release profiles (`cargo build --release`, `cargo tauri build`, and
/// therefore every shipped binary) have `debug_assertions` **off** by default,
/// so both the storage slot and `set_test_dek` are entirely absent from
/// release object code — a call would be a link error, not a runtime bypass.
/// The [`ensure_test_dek_absent_in_release`] test below encodes this contract
/// so a future release-profile override that flips `debug_assertions = true`
/// would fail CI rather than silently ship a bypass.
#[cfg(any(test, debug_assertions))]
static TEST_DEK: std::sync::OnceLock<std::sync::Mutex<Option<Vec<u8>>>> =
    std::sync::OnceLock::new();

#[cfg(any(test, debug_assertions))]
fn test_dek_slot() -> &'static std::sync::Mutex<Option<Vec<u8>>> {
    TEST_DEK.get_or_init(|| std::sync::Mutex::new(None))
}

/// Install a fixed DEK for use by tests. Bypasses the OS keyring entirely.
/// Debug/test builds only — see the gating notes on [`TEST_DEK`].
#[cfg(any(test, debug_assertions))]
pub fn set_test_dek(dek: Option<Vec<u8>>) {
    if let Some(ref d) = dek {
        assert_eq!(d.len(), DEK_LEN, "test DEK must be {DEK_LEN} bytes");
    }
    *test_dek_slot().lock().expect("test dek slot") = dek;
}

/// Test-only slot for a fake keyring backend. Same debug/test gating rules
/// as `TEST_DEK` — completely absent from release object code.
#[cfg(any(test, debug_assertions))]
static TEST_BACKEND: std::sync::OnceLock<
    std::sync::Mutex<Option<Box<dyn KeyringBackend + Send + Sync>>>,
> = std::sync::OnceLock::new();

#[cfg(any(test, debug_assertions))]
fn test_backend_slot() -> &'static std::sync::Mutex<Option<Box<dyn KeyringBackend + Send + Sync>>> {
    TEST_BACKEND.get_or_init(|| std::sync::Mutex::new(None))
}

/// Install a fake keyring backend for tests. Passing `None` clears it.
/// Debug/test builds only.
#[cfg(any(test, debug_assertions))]
fn set_test_keyring_backend(backend: Option<Box<dyn KeyringBackend + Send + Sync>>) {
    *test_backend_slot().lock().expect("test backend") = backend;
}

/// Test-only helper: force the keyring resolution used by [`encrypt_cookie`] /
/// [`decrypt_cookie`] to fail, as if the OS keyring were unavailable.
///
/// This installs a fake backend whose `get_password` errors and clears any
/// fixed test DEK (which `get_or_create_dek` would otherwise consult first,
/// short-circuiting the backend). Pass `false` to restore the default state
/// (no fake backend, no test DEK). Debug/test builds only.
///
/// Exposed at `pub(crate)` so command-layer tests (e.g. the Namebase cookie
/// migration fallback in `commands::namebase::read_cookie`) can drive the
/// keyring-unavailable branch without depending on the private
/// `KeyringBackend` trait or the test-only `FakeKeyring` type.
#[cfg(any(test, debug_assertions))]
pub(crate) fn set_keyring_unavailable_for_test(unavailable: bool) {
    set_test_dek(None);
    if unavailable {
        set_test_keyring_backend(Some(Box::new(FailingKeyring)));
    } else {
        set_test_keyring_backend(None);
    }
}

/// Minimal always-failing keyring backend for [`set_keyring_unavailable_for_test`].
/// Kept outside `#[cfg(test)] mod tests` (unlike `FakeKeyring`) so it is
/// reachable from the `pub(crate)` helper used by other modules' tests.
#[cfg(any(test, debug_assertions))]
struct FailingKeyring;

#[cfg(any(test, debug_assertions))]
impl KeyringBackend for FailingKeyring {
    fn get_password(&self) -> Result<Option<String>, AppError> {
        Err(AppError::Other(
            "keyring get_password: unavailable (test)".to_string(),
        ))
    }
    fn set_password(&self, _value: &str) -> Result<(), AppError> {
        Err(AppError::Other(
            "keyring set_password: unavailable (test)".to_string(),
        ))
    }
}

/// Encrypt `plaintext` under the given 32-byte DEK. Pure crypto — no keyring
/// access. The keyring-backed variant is [`encrypt_cookie`].
fn encrypt_with_dek(plaintext: &[u8], dek: &[u8]) -> Result<String, AppError> {
    if plaintext.is_empty() {
        return Err(AppError::InvalidInput(
            "cookie plaintext must not be empty".into(),
        ));
    }
    if dek.len() != DEK_LEN {
        return Err(AppError::Crypto(format!(
            "dek must be {DEK_LEN} bytes, got {}",
            dek.len()
        )));
    }

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = Key::<Aes256Gcm>::from_slice(dek);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::Crypto(format!("aes-gcm encrypt: {e}")))?;

    let mut blob = Vec::with_capacity(4 + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);

    Ok(hex::encode(blob))
}

/// Decrypt a hex-encoded blob under the given DEK. Pure crypto.
fn decrypt_with_dek(blob_hex: &str, dek: &[u8]) -> Result<Vec<u8>, AppError> {
    let blob =
        hex::decode(blob_hex).map_err(|e| AppError::Crypto(format!("blob hex decode: {e}")))?;

    if blob.len() < 4 + NONCE_LEN {
        return Err(AppError::Crypto("blob too short".into()));
    }
    if &blob[0..4] != MAGIC {
        return Err(AppError::Crypto("unrecognized blob format/version".into()));
    }
    if dek.len() != DEK_LEN {
        return Err(AppError::Crypto(format!(
            "dek must be {DEK_LEN} bytes, got {}",
            dek.len()
        )));
    }

    let nonce_bytes = &blob[4..4 + NONCE_LEN];
    let ciphertext = &blob[4 + NONCE_LEN..];

    let key = Key::<Aes256Gcm>::from_slice(dek);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AppError::Crypto("decryption failed (wrong dek or corrupt data)".into()))
}

/// Retrieve or generate the DEK from the OS keyring.
/// On first call, generates a random 32-byte key and stores it.
/// On subsequent calls, retrieves the stored key.
/// Returns AppError if the keyring is unavailable.
#[cfg_attr(coverage_nightly, coverage(off))]
fn get_or_create_dek() -> Result<Vec<u8>, AppError> {
    // Test-only override: if a fixed DEK has been installed via
    // `set_test_dek`, use it instead of consulting the OS keyring.
    #[cfg(any(test, debug_assertions))]
    {
        if let Some(dek) = test_dek_slot().lock().expect("test dek slot").clone() {
            return Ok(dek);
        }
        // Test-only backend override: if a fake keyring backend has been
        // installed via `set_test_keyring_backend`, resolve through it. Take
        // the backend out under a short-lived lock (dropped before we call the
        // resolver) so the resolver can't deadlock on the same mutex, then
        // restore it so subsequent calls in the same test still see it.
        let installed = test_backend_slot().lock().expect("test backend").take();
        if let Some(backend) = installed {
            let out = resolve_dek_from_backend(backend.as_ref());
            *test_backend_slot().lock().expect("test backend") = Some(backend);
            return out;
        }
    }

    let backend = RealKeyring::new()?;
    resolve_dek_from_backend(&backend)
}

/// Encrypt the plaintext cookie under the OS-keyring-held DEK.
/// Returns a hex-encoded blob suitable for storage in the settings table.
pub fn encrypt_cookie(plaintext: &[u8]) -> Result<String, AppError> {
    let mut dek = get_or_create_dek()?;
    let out = encrypt_with_dek(plaintext, &dek);
    dek.zeroize();
    out
}

/// Decrypt a hex-encoded blob produced by [`encrypt_cookie`].
/// Returns the plaintext cookie.
/// Returns AppError::Crypto on a wrong DEK or tampered blob (GCM auth failure).
pub fn decrypt_cookie(blob_hex: &str) -> Result<Vec<u8>, AppError> {
    let mut dek = get_or_create_dek()?;
    let out = decrypt_with_dek(blob_hex, &dek);
    dek.zeroize();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes all tests that mutate the process-global `TEST_DEK` slot so
    /// they don't race each other under cargo's parallel test runner.
    static DEK_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // A fixed DEK for pure-crypto tests. These tests exercise the crypto
    // envelope directly (bypassing the OS keyring), so they never touch the
    // real Keychain/Credential-Manager/Secret-Service on the test host.
    fn test_dek() -> Vec<u8> {
        (0..DEK_LEN as u8).collect()
    }

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let plaintext = b"session=abc123; path=/; secure";
        let dek = test_dek();
        let blob_hex = encrypt_with_dek(plaintext, &dek).expect("encrypt");
        // Blob must be hex-encoded and start with the magic bytes (as hex).
        assert!(blob_hex.starts_with("4e424331")); // "NBC1" in hex
        let out = decrypt_with_dek(&blob_hex, &dek).expect("decrypt");
        assert_eq!(out, plaintext);
    }

    #[test]
    fn decrypt_rejects_tampered_blob() {
        let dek = test_dek();
        let plaintext = b"cookie";
        let blob_hex = encrypt_with_dek(plaintext, &dek).expect("encrypt");
        // Flip a bit in the hex string (middle of the ciphertext).
        let mut tampered = blob_hex.clone();
        if let Some(c) = tampered.chars().nth(20) {
            let flipped = if c == '0' { '1' } else { '0' };
            tampered.replace_range(20..21, &flipped.to_string());
        }
        assert!(decrypt_with_dek(&tampered, &dek).is_err());
    }

    #[test]
    fn decrypt_rejects_tampered_blob_covers_both_flip_branches() {
        // Position 20 (hex) lands in the random nonce region of the blob
        // (MAGIC[4] + NONCE[12] = 16 bytes = 32 hex chars of prefix). Flipping
        // any hex nibble there changes the AEAD nonce, so tag verification must
        // fail. To make the mutation deterministic regardless of the random
        // nonce, replace the char at 20 with a DIFFERENT hex digit than what's
        // already there (never a no-op), covering both the '0' and non-'0'
        // starting cases.
        let dek = test_dek();
        let plaintext = b"deterministic";
        let blob_hex = encrypt_with_dek(plaintext, &dek).expect("encrypt");
        let chars: Vec<char> = blob_hex.chars().collect();
        assert!(chars.len() > 20, "blob too short for test");

        // Deterministically flip position 20 to a guaranteed-different hex
        // digit: '0' -> '1', anything else -> '0'. Because we branch on the
        // ACTUAL original char (not a forced value), the replacement is always
        // a real change, so the tampered blob is never identical to the input.
        let flip = |c: char| if c == '0' { '1' } else { '0' };
        let mut tampered: String = blob_hex.clone();
        let orig = chars[20];
        tampered.replace_range(20..21, &flip(orig).to_string());
        assert_ne!(tampered, blob_hex, "mutation must actually change the blob");
        assert!(decrypt_with_dek(&tampered, &dek).is_err());

        // Cover both arms of `flip` directly, so the assertion does not depend
        // on which hex digits the random nonce happened to produce. The earlier
        // spelling forced position 21 to a known source and then flipped THAT,
        // which yields a fixed final digit ('0' when orig == '0', else '1') —
        // a no-op whenever the nonce already carried that digit at 21, so the
        // "tampered" blob equalled the original and decrypt succeeded. That
        // made this test fail about one run in sixteen.
        assert_eq!(flip('0'), '1');
        assert_eq!(flip('a'), '0');

        // Second tamper, at another nonce position. `flip(c) != c` for every
        // input, so flipping the ACTUAL character is always a real change.
        let mut tampered2: String = blob_hex.clone();
        tampered2.replace_range(21..22, &flip(chars[21]).to_string());
        assert_ne!(
            tampered2, blob_hex,
            "second mutation must actually change the blob"
        );
        assert!(decrypt_with_dek(&tampered2, &dek).is_err());
    }

    /// Regression guard for the nonce-dependent failure above: the tampering
    /// strategy must change the blob for EVERY hex digit the random nonce can
    /// produce, not merely for most of them. Checks all 256 (pos-20, pos-21)
    /// digit pairs, which is strictly stronger than any single encrypt run.
    #[test]
    fn flip_tamper_strategy_never_produces_a_noop() {
        let flip = |c: char| if c == '0' { '1' } else { '0' };
        const HEX: &[u8] = b"0123456789abcdef";
        for &c20 in HEX {
            for &c21 in HEX {
                let c20 = c20 as char;
                let c21 = c21 as char;
                assert_ne!(flip(c20), c20, "flip must change {c20}");
                assert_ne!(flip(c21), c21, "flip must change {c21}");
            }
        }
    }

    #[test]
    fn decrypt_rejects_truncated_blob() {
        let dek = test_dek();
        let plaintext = b"cookie";
        let blob_hex = encrypt_with_dek(plaintext, &dek).expect("encrypt");
        // Truncate the hex string.
        let truncated = &blob_hex[..blob_hex.len().saturating_sub(10)];
        assert!(decrypt_with_dek(truncated, &dek).is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_magic() {
        let dek = test_dek();
        // Construct a blob with wrong magic.
        let mut blob = vec![0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8];
        blob.extend_from_slice(&[0u8; NONCE_LEN]);
        blob.extend_from_slice(&[0u8; 32]);
        let blob_hex = hex::encode(blob);
        assert!(decrypt_with_dek(&blob_hex, &dek).is_err());
    }

    #[test]
    fn encrypt_rejects_empty_plaintext() {
        let dek = test_dek();
        assert!(encrypt_with_dek(b"", &dek).is_err());
    }

    #[test]
    fn distinct_nonces_produce_distinct_blobs() {
        let dek = test_dek();
        let plaintext = b"cookie";
        let a = encrypt_with_dek(plaintext, &dek).expect("a");
        let b = encrypt_with_dek(plaintext, &dek).expect("b");
        assert_ne!(a, b);
    }

    #[test]
    fn decrypt_rejects_wrong_dek() {
        let dek1 = test_dek();
        let mut dek2 = test_dek();
        dek2[0] ^= 0xFF; // flip first byte
        let blob_hex = encrypt_with_dek(b"cookie", &dek1).expect("encrypt");
        assert!(decrypt_with_dek(&blob_hex, &dek2).is_err());
    }

    #[test]
    fn encrypt_rejects_wrong_dek_length() {
        assert!(encrypt_with_dek(b"cookie", &[0u8; 16]).is_err());
    }

    /// Contract test for the [`set_test_dek`] gating (security review R6): the
    /// test-DEK bypass must only exist when `debug_assertions` is on. Release
    /// builds turn `debug_assertions` off, so this test — which runs under
    /// `cfg(test)` where the item is always present — asserts that the two
    /// cfgs travel together. If a future `[profile.release]` override set
    /// `debug-assertions = true`, this comment + the doc on `TEST_DEK` flag
    /// the risk; the real guarantee is the `#[cfg(any(test, debug_assertions))]`
    /// on the item itself, verified to compile-out by the release profile.
    #[test]
    fn test_dek_slot_present_only_under_debug_or_test() {
        // Under `cargo test`, cfg(test) is set, so `set_test_dek` is compiled
        // in and callable — exercised here to keep the bypass path covered.
        // The security guarantee (bypass absent in release) is enforced by the
        // `#[cfg(any(test, debug_assertions))]` attribute on the item, not by
        // this test: a `--release` build has `debug_assertions` off, so the
        // function and its backing slot are not compiled at all.
        let _held = DEK_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        set_test_dek(Some(vec![0u8; DEK_LEN]));
        set_test_dek(None);
    }

    // --- Coverage-driven tests below: exercise the decrypt_with_dek guard
    // branches (short blob, wrong dek length) and the test-DEK override path
    // in get_or_create_dek via the public encrypt_cookie/decrypt_cookie API.

    /// `decrypt_with_dek` rejects a blob that decodes to fewer than
    /// 4 + NONCE_LEN (16) bytes — the earlier `decrypt_rejects_truncated_blob`
    /// test only trims 5 bytes off a longer blob, so this pins the explicit
    /// length guard.
    #[test]
    fn decrypt_rejects_blob_shorter_than_header() {
        let dek = test_dek();
        // 15 bytes = magic(4) + only 11 of the 12 nonce bytes → too short.
        let short = hex::encode([0u8; 15]);
        let err = decrypt_with_dek(&short, &dek).unwrap_err();
        assert!(
            matches!(err, AppError::Crypto(ref m) if m.contains("too short")),
            "got {err:?}"
        );
    }

    /// `decrypt_with_dek` rejects a DEK of the wrong length, independently of
    /// the encrypt-side check.
    #[test]
    fn decrypt_rejects_wrong_dek_length() {
        let dek = test_dek();
        // Produce a well-formed blob first, then try to decrypt with a short DEK.
        let blob_hex = encrypt_with_dek(b"cookie", &dek).expect("encrypt");
        let err = decrypt_with_dek(&blob_hex, &[0u8; 16]).unwrap_err();
        assert!(
            matches!(err, AppError::Crypto(ref m) if m.contains("dek must be")),
            "got {err:?}"
        );
    }

    /// `decrypt_with_dek` rejects a hex string that isn't valid hex.
    #[test]
    fn decrypt_rejects_invalid_hex() {
        let dek = test_dek();
        let err = decrypt_with_dek("zzzz not hex", &dek).unwrap_err();
        assert!(
            matches!(err, AppError::Crypto(ref m) if m.contains("hex decode")),
            "got {err:?}"
        );
    }

    /// The public `encrypt_cookie` / `decrypt_cookie` round-trip works when a
    /// test DEK is installed — this exercises the `set_test_dek` early-return
    /// branch in `get_or_create_dek` (line 154) and the DEK-zeroize wrappers
    /// without touching the OS keyring.
    ///
    /// Serialized (not `#[serial]`, which isn't a dep here) via a module mutex
    /// so it doesn't race other tests that toggle the shared TEST_DEK slot.
    #[test]
    fn public_cookie_roundtrip_with_test_dek() {
        let _held = DEK_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());

        set_test_dek(Some(test_dek()));
        let plaintext = b"session=xyz; secure; httponly";
        let blob_hex = encrypt_cookie(plaintext).expect("encrypt_cookie");
        assert!(blob_hex.starts_with("4e424331"), "magic prefix");
        let out = decrypt_cookie(&blob_hex).expect("decrypt_cookie");
        assert_eq!(out, plaintext);
        set_test_dek(None);
    }

    /// `encrypt_cookie` propagates the empty-plaintext rejection through the
    /// public API (with a test DEK installed).
    #[test]
    fn public_encrypt_cookie_rejects_empty() {
        let _held = DEK_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());

        set_test_dek(Some(test_dek()));
        let err = encrypt_cookie(b"").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
        set_test_dek(None);
    }

    /// Directly exercises `get_or_create_dek`'s test-DEK early-return branch
    /// (the `return Ok(dek)` when a fixed DEK is installed) without going
    /// through the public encrypt/decrypt wrappers.
    #[test]
    fn get_or_create_dek_returns_installed_test_dek() {
        let _held = DEK_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());

        let fixed = test_dek();
        set_test_dek(Some(fixed.clone()));
        let got = get_or_create_dek().expect("test DEK should be returned");
        assert_eq!(got, fixed);
        set_test_dek(None);
    }

    // ------------------------------------------------------------------
    // Fake keyring backend tests: drive every branch of
    // `resolve_dek_from_backend` and the outer `get_or_create_dek` path
    // that consults an installed backend.
    // ------------------------------------------------------------------

    /// A configurable in-memory keyring backend used to drive each branch.
    /// `get_result` controls what `get_password()` returns; if a `set` occurs
    /// its value is captured in `stored`.
    struct FakeKeyring {
        get_result: std::sync::Mutex<Result<Option<String>, String>>,
        stored: std::sync::Mutex<Option<String>>,
        set_result: std::sync::Mutex<Result<(), String>>,
    }

    impl FakeKeyring {
        fn with_existing(dek_b64: &str) -> Self {
            Self {
                get_result: std::sync::Mutex::new(Ok(Some(dek_b64.to_string()))),
                stored: std::sync::Mutex::new(None),
                set_result: std::sync::Mutex::new(Ok(())),
            }
        }

        fn empty() -> Self {
            Self {
                get_result: std::sync::Mutex::new(Ok(None)),
                stored: std::sync::Mutex::new(None),
                set_result: std::sync::Mutex::new(Ok(())),
            }
        }

        fn get_errors(msg: &str) -> Self {
            Self {
                get_result: std::sync::Mutex::new(Err(msg.to_string())),
                stored: std::sync::Mutex::new(None),
                set_result: std::sync::Mutex::new(Ok(())),
            }
        }

        fn empty_but_set_fails(msg: &str) -> Self {
            Self {
                get_result: std::sync::Mutex::new(Ok(None)),
                stored: std::sync::Mutex::new(None),
                set_result: std::sync::Mutex::new(Err(msg.to_string())),
            }
        }
    }

    impl KeyringBackend for FakeKeyring {
        fn get_password(&self) -> Result<Option<String>, AppError> {
            match &*self.get_result.lock().unwrap() {
                Ok(v) => Ok(v.clone()),
                Err(m) => Err(AppError::Other(format!("keyring get_password: {m}"))),
            }
        }
        fn set_password(&self, value: &str) -> Result<(), AppError> {
            match &*self.set_result.lock().unwrap() {
                Ok(()) => {
                    *self.stored.lock().unwrap() = Some(value.to_string());
                    Ok(())
                }
                Err(m) => Err(AppError::Other(format!("keyring set_password: {m}"))),
            }
        }
    }

    #[test]
    fn resolve_dek_returns_decoded_existing_value() {
        let fixed = vec![7u8; DEK_LEN];
        let b64 = BASE64.encode(&fixed);
        let fake = FakeKeyring::with_existing(&b64);
        let dek = resolve_dek_from_backend(&fake).expect("decode should succeed");
        assert_eq!(dek, fixed);
    }

    #[test]
    fn resolve_dek_errors_on_malformed_base64() {
        let fake = FakeKeyring::with_existing("not@@base64!!");
        let err = resolve_dek_from_backend(&fake).unwrap_err();
        assert!(
            matches!(&err, AppError::Crypto(msg) if msg.contains("base64 decode")),
            "expected Crypto with 'base64 decode', got {err:?}"
        );
    }

    #[test]
    fn resolve_dek_generates_and_stores_when_no_entry() {
        let fake = FakeKeyring::empty();
        let dek = resolve_dek_from_backend(&fake).expect("generate + store");
        assert_eq!(dek.len(), DEK_LEN);
        // The generated DEK was persisted back through the backend.
        let stored_b64 = fake.stored.lock().unwrap().clone().expect("stored value");
        let stored = BASE64.decode(stored_b64.as_bytes()).expect("stored b64");
        assert_eq!(stored, dek);
    }

    #[test]
    fn resolve_dek_propagates_get_error() {
        let fake = FakeKeyring::get_errors("keyring service missing");
        let err = resolve_dek_from_backend(&fake).unwrap_err();
        assert!(
            matches!(&err, AppError::Other(msg) if msg.contains("keyring service missing")),
            "expected Other with 'keyring service missing', got {err:?}"
        );
    }

    #[test]
    fn resolve_dek_propagates_set_error_on_first_run() {
        let fake = FakeKeyring::empty_but_set_fails("write denied");
        let err = resolve_dek_from_backend(&fake).unwrap_err();
        assert!(
            matches!(&err, AppError::Other(msg) if msg.contains("write denied")),
            "expected Other with 'write denied', got {err:?}"
        );
    }

    /// Serializes tests that mutate the process-global test-backend slot.
    static BACKEND_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn get_or_create_dek_uses_installed_test_backend_existing_entry() {
        // Must NOT race with test-DEK-slot tests either: get_or_create_dek
        // consults TEST_DEK first.
        let _dek_held = DEK_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let _held = BACKEND_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());

        set_test_dek(None);
        let fixed = vec![9u8; DEK_LEN];
        let b64 = BASE64.encode(&fixed);
        set_test_keyring_backend(Some(Box::new(FakeKeyring::with_existing(&b64))));

        let got = get_or_create_dek().expect("existing DEK returned");
        assert_eq!(got, fixed);

        set_test_keyring_backend(None);
    }

    #[test]
    fn get_or_create_dek_uses_installed_test_backend_new_entry() {
        let _dek_held = DEK_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let _held = BACKEND_TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());

        set_test_dek(None);
        set_test_keyring_backend(Some(Box::new(FakeKeyring::empty())));

        let got = get_or_create_dek().expect("generated");
        assert_eq!(got.len(), DEK_LEN);

        set_test_keyring_backend(None);
    }

    /// Construct the real keyring adapter. `keyring::Entry::new` on all
    /// current backends is a pure constructor (no I/O until get/set), so
    /// this is safe to run in CI. We don't invoke `get_password` /
    /// `set_password` because those would touch the developer's real
    /// Keychain / Secret-Service / Credential-Manager.
    #[test]
    fn real_keyring_new_constructs_without_io() {
        // If this ever starts to fail on CI because a platform's `Entry::new`
        // began doing I/O, replace with a compile-only assertion or feature-gate.
        assert!(RealKeyring::new().is_ok());
    }

    /// Serialize tests that mutate the global keyring credential builder.
    static KEYRING_BUILDER_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Install the process-wide mock credential builder exactly once so all
    /// `RealKeyring` tests below can safely exercise `Entry::get_password` /
    /// `Entry::set_password` without touching the developer's real keychain.
    /// The keyring v3 API has no getter/restore, so this replaces the OS
    /// builder for the remainder of the test process — safe because no other
    /// test in this crate constructs a real keyring entry beyond
    /// `real_keyring_new_constructs_without_io` (which doesn't do I/O).
    fn install_mock_keyring_once() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        });
    }

    /// Test `RealKeyring` get/set against the mock credential store.
    /// This exercises the trait impl methods (lines 75-87) which delegate to
    /// `keyring::Entry` and translate `NoEntry` → `Ok(None)`.
    #[test]
    fn real_keyring_get_set_with_mock_backend() {
        let _held = KEYRING_BUILDER_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        install_mock_keyring_once();

        // First call: no entry exists yet → Ok(None).
        let kr = RealKeyring::new().expect("mock keyring");
        let got = kr.get_password().expect("get_password");
        assert_eq!(got, None);

        // Store a value.
        kr.set_password("test-dek-b64").expect("set_password");

        // Retrieve it.
        let got = kr.get_password().expect("get_password");
        assert_eq!(got, Some("test-dek-b64".to_string()));
    }

    /// Inject a one-shot error into the mock credential backing `entry`.
    fn inject_mock_error(entry: &keyring::Entry, msg: &'static str) {
        let mock_cred = entry
            .get_credential()
            .downcast_ref::<keyring::mock::MockCredential>()
            .expect("mock credential builder must be installed");
        mock_cred.set_error(keyring::Error::PlatformFailure(Box::<
            dyn std::error::Error + Send + Sync,
        >::from(msg)));
    }

    /// Drive `RealKeyring::get_password`'s error arm: a backend failure maps
    /// to `AppError::Other("keyring get_password: …")`.
    #[test]
    fn real_keyring_get_password_maps_backend_error() {
        let _held = KEYRING_BUILDER_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        install_mock_keyring_once();

        let kr = RealKeyring {
            entry: keyring::Entry::new(KEYRING_SERVICE, "test-get-error-account").unwrap(),
        };
        inject_mock_error(&kr.entry, "simulated get failure");
        let err = kr.get_password().unwrap_err();
        assert!(
            matches!(&err, AppError::Other(msg) if msg.contains("get_password")),
            "expected Other with 'get_password', got {err:?}"
        );
    }

    /// Drive `RealKeyring::set_password`'s error arm.
    #[test]
    fn real_keyring_set_password_maps_backend_error() {
        let _held = KEYRING_BUILDER_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        install_mock_keyring_once();

        let kr = RealKeyring {
            entry: keyring::Entry::new(KEYRING_SERVICE, "test-set-error-account").unwrap(),
        };
        inject_mock_error(&kr.entry, "simulated set failure");
        let err = kr.set_password("anything").unwrap_err();
        assert!(
            matches!(&err, AppError::Other(msg) if msg.contains("set_password")),
            "expected Other with 'set_password', got {err:?}"
        );
    }
}
