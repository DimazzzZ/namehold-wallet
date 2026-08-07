//! Secret-at-rest encryption for wallet seed material.
//!
//! Design goals:
//!   - The BIP39 mnemonic (the root secret) is NEVER stored in plaintext.
//!   - Encryption key is derived from a user passphrase via Argon2id, a
//!     memory-hard KDF resistant to GPU/ASIC brute force.
//!   - Ciphertext is authenticated with AES-256-GCM so any tampering with the
//!     stored blob is detected on decrypt.
//!   - The serialized blob is self-describing and versioned, so the format can
//!     evolve without ambiguity.
//!
//! On-disk blob layout (all binary, then the whole thing is hex/base64 by the
//! caller if needed):
//!
//! ### NHV1 (legacy, decrypt-only)
//! ```text
//!   magic:    4 bytes  = b"NHV1"        (Namehold Vault v1)
//!   salt_len: 1 byte
//!   salt:     salt_len bytes            (Argon2 salt)
//!   nonce:    12 bytes                  (AES-GCM nonce)
//!   ct:       remainder                 (ciphertext + 16-byte GCM tag)
//! ```
//! Argon2id params are fixed by version: 64 MiB / 3 iters / 1 lane.
//!
//! ### NHV2 (current write format)
//! ```text
//!   magic:    4 bytes  = b"NHV2"        (Namehold Vault v2)
//!   mem_kib:  4 bytes  (u32 LE)         (Argon2id memory cost)
//!   iters:    4 bytes  (u32 LE)         (Argon2id iteration cost)
//!   lanes:    4 bytes  (u32 LE)         (Argon2id parallelism)
//!   salt_len: 1 byte
//!   salt:     salt_len bytes            (Argon2 salt)
//!   nonce:    12 bytes                  (AES-GCM nonce)
//!   ct:       remainder                 (ciphertext + 16-byte GCM tag)
//! ```
//! Argon2 parameters are embedded so future increases to cost are self-
//! describing (a v2 blob written today with 256 MiB will still decrypt on a
//! build that has raised the default further tomorrow).
//!
//! ### Migration
//! Existing NHV1 blobs continue to decrypt with the fixed v1 params. There is
//! no forced rewrite: a wallet re-encrypted after any passphrase change is
//! written as NHV2, otherwise it stays NHV1 until the user changes their
//! passphrase. Both formats are equally authenticated (AES-256-GCM); NHV2
//! merely raises the offline brute-force cost.

use crate::error::AppError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::Zeroize;

const MAGIC_V1: &[u8; 4] = b"NHV1";
const MAGIC_V2: &[u8; 4] = b"NHV2";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

// Argon2id v1 params — fixed for legacy NHV1 blobs (64 MiB / 3 iters / 1 lane).
// These constants must NEVER change; decrypting an NHV1 blob requires the exact
// params it was encrypted with.
const V1_MEM_KIB: u32 = 64 * 1024;
const V1_ITERS: u32 = 3;
const V1_LANES: u32 = 1;

// Argon2id v2 defaults — used when writing new blobs. Tuned for a 2025 desktop
// wallet: 256 MiB memory, 4 iterations, 1 lane. Free to bump over time; older
// NHV2 blobs stay decryptable because their params are embedded in the blob.
//
// Under `cfg(test)` the V2 memory/iters cost is dramatically reduced (8 MiB / 1
// iter, still above Argon2's minima). Test coverage never depends on the
// *magnitude* of the cost — only on round-trip / auth-failure / salt-random
// behaviours — so this cuts the vault-test wall-clock from ~226s to sub-second
// on debug builds without weakening a single assertion. Production builds
// (`cfg(not(test))`) keep the full 256 MiB / 4 iter cost. A separate constant
// self-test below pins the literal production values so a cfg mistake cannot
// silently ship weak KDF params to real wallets. `V2_LANES` is not cost-scaled.
#[cfg(not(test))]
const V2_MEM_KIB: u32 = 256 * 1024;
#[cfg(not(test))]
const V2_ITERS: u32 = 4;
#[cfg(test)]
const V2_MEM_KIB: u32 = 8 * 1024; // 8 MiB — well above Argon2's 8 KiB min
#[cfg(test)]
const V2_ITERS: u32 = 1;
const V2_LANES: u32 = 1;

/// Compile-time self-test: the literal production Argon2id V2 write-cost
/// parameters. Any change to the `#[cfg(not(test))]` `V2_MEM_KIB` / `V2_ITERS`
/// above will fail to compile until this constant is updated in lockstep,
/// making it impossible for a routine `cfg(test)` edit to accidentally weaken
/// the production KDF cost. Only relevant in non-test builds — under
/// `cfg(test)` the constants above are reduced on purpose, so this pin is
/// disabled there.
#[cfg(not(test))]
const _PROD_V2_COST_PIN: () = {
    assert!(
        V2_MEM_KIB == 256 * 1024,
        "production V2_MEM_KIB must be 256 MiB"
    );
    assert!(V2_ITERS == 4, "production V2_ITERS must be 4");
    assert!(V2_LANES == 1, "production V2_LANES must be 1");
};

// Upper bounds enforced when reading an NHV2 blob. A tampered blob claiming
// pathological params (e.g. 16 GiB memory, billions of iters) must not be able
// to hang or OOM the process on decrypt. The bounds are generous enough that
// any legitimate future cost bump fits, but bounded enough that untrusted
// input can't DoS us. Argon2 itself has stricter minimums that Params::new
// enforces, so we only need upper caps here.
const MAX_MEM_KIB: u32 = 4 * 1024 * 1024; // 4 GiB
const MAX_ITERS: u32 = 32;
const MAX_LANES: u32 = 8;

/// The tuple returned by [`parse_v1`] / [`parse_v2`]:
/// `(mem_kib, iters, lanes, salt, nonce, ciphertext)`. Factored into a type
/// alias to satisfy `clippy::type_complexity`.
type ParsedBlob<'a> = (u32, u32, u32, &'a [u8], &'a [u8], &'a [u8]);

/// Derive a 32-byte AES key from a passphrase + salt using Argon2id with the
/// provided cost parameters. `mem_kib`/`iters`/`lanes` are read from the blob
/// for NHV2, or are the fixed v1 constants for NHV1.
fn derive_key(
    passphrase: &[u8],
    salt: &[u8],
    mem_kib: u32,
    iters: u32,
    lanes: u32,
) -> Result<[u8; KEY_LEN], AppError> {
    let params = Params::new(mem_kib, iters, lanes, Some(KEY_LEN))
        .map_err(|e| AppError::Crypto(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| AppError::Crypto(format!("argon2 derive: {e}")))?;
    Ok(key)
}

/// Encrypt `plaintext` (e.g. the mnemonic bytes) under `passphrase`, returning
/// the self-describing vault blob. Always writes the current NHV2 format.
pub fn encrypt(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, AppError> {
    if passphrase.is_empty() {
        return Err(AppError::InvalidInput(
            "passphrase must not be empty".into(),
        ));
    }

    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let mut key_bytes = derive_key(passphrase.as_bytes(), &salt, V2_MEM_KIB, V2_ITERS, V2_LANES)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::Crypto(format!("aes-gcm encrypt: {e}")))?;
    key_bytes.zeroize();

    // Assemble NHV2 blob:
    //   magic(4) | mem_kib(4) | iters(4) | lanes(4) | salt_len(1) | salt | nonce(12) | ct
    let mut blob =
        Vec::with_capacity(4 + 4 + 4 + 4 + 1 + salt.len() + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC_V2);
    blob.extend_from_slice(&V2_MEM_KIB.to_le_bytes());
    blob.extend_from_slice(&V2_ITERS.to_le_bytes());
    blob.extend_from_slice(&V2_LANES.to_le_bytes());
    blob.push(salt.len() as u8);
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Decrypt a vault blob produced by [`encrypt`] (any supported version) using
/// `passphrase`.
///
/// Returns `AppError::Crypto` on a wrong passphrase or tampered blob (GCM auth
/// failure is indistinguishable from a wrong key, by design).
pub fn decrypt(blob: &[u8], passphrase: &str) -> Result<Vec<u8>, AppError> {
    if blob.len() < 4 {
        return Err(AppError::Crypto("vault blob too short".into()));
    }
    let magic = &blob[0..4];
    let (mem_kib, iters, lanes, salt, nonce_bytes, ciphertext) = if magic == MAGIC_V1 {
        parse_v1(blob)?
    } else if magic == MAGIC_V2 {
        parse_v2(blob)?
    } else {
        return Err(AppError::Crypto("unrecognized vault format/version".into()));
    };

    let mut key_bytes = derive_key(passphrase.as_bytes(), salt, mem_kib, iters, lanes)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        AppError::Crypto("decryption failed (wrong passphrase or corrupt data)".into())
    });
    key_bytes.zeroize();
    plaintext
}

/// Parse an NHV1 blob into `(mem_kib, iters, lanes, salt, nonce, ct)`. Params
/// are the fixed v1 constants.
fn parse_v1(blob: &[u8]) -> Result<ParsedBlob<'_>, AppError> {
    if blob.len() < 4 + 1 {
        return Err(AppError::Crypto("vault blob too short".into()));
    }
    let salt_len = blob[4] as usize;
    let mut offset = 5;
    if blob.len() < offset + salt_len + NONCE_LEN {
        return Err(AppError::Crypto("vault blob truncated".into()));
    }
    let salt = &blob[offset..offset + salt_len];
    offset += salt_len;
    let nonce_bytes = &blob[offset..offset + NONCE_LEN];
    offset += NONCE_LEN;
    let ciphertext = &blob[offset..];
    Ok((
        V1_MEM_KIB,
        V1_ITERS,
        V1_LANES,
        salt,
        nonce_bytes,
        ciphertext,
    ))
}

/// Parse an NHV2 blob into `(mem_kib, iters, lanes, salt, nonce, ct)`. Params
/// are read from the blob and bounds-checked to prevent tampered blobs from
/// requesting pathological KDF costs on decrypt.
fn parse_v2(blob: &[u8]) -> Result<ParsedBlob<'_>, AppError> {
    // magic(4) | mem_kib(4) | iters(4) | lanes(4) | salt_len(1) | ...
    if blob.len() < 4 + 4 + 4 + 4 + 1 {
        return Err(AppError::Crypto("vault blob too short".into()));
    }
    let mem_kib = u32::from_le_bytes(blob[4..8].try_into().unwrap());
    let iters = u32::from_le_bytes(blob[8..12].try_into().unwrap());
    let lanes = u32::from_le_bytes(blob[12..16].try_into().unwrap());
    if mem_kib > MAX_MEM_KIB || iters > MAX_ITERS || lanes == 0 || lanes > MAX_LANES {
        return Err(AppError::Crypto("vault blob params out of range".into()));
    }
    let salt_len = blob[16] as usize;
    let mut offset = 17;
    if blob.len() < offset + salt_len + NONCE_LEN {
        return Err(AppError::Crypto("vault blob truncated".into()));
    }
    let salt = &blob[offset..offset + salt_len];
    offset += salt_len;
    let nonce_bytes = &blob[offset..offset + NONCE_LEN];
    offset += NONCE_LEN;
    let ciphertext = &blob[offset..];
    Ok((mem_kib, iters, lanes, salt, nonce_bytes, ciphertext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_recovers_plaintext() {
        let secret = b"abandon abandon abandon abandon about";
        let blob = encrypt(secret, "correct horse battery staple").expect("encrypt");
        // Blob must be self-describing and not contain the plaintext.
        assert_eq!(&blob[0..4], MAGIC_V2);
        assert!(!blob.windows(secret.len()).any(|w| w == secret));
        let out = decrypt(&blob, "correct horse battery staple").expect("decrypt");
        assert_eq!(out, secret);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let blob = encrypt(b"top secret seed", "right-pass").expect("encrypt");
        assert!(decrypt(&blob, "wrong-pass").is_err());
    }

    #[test]
    fn tampered_blob_fails() {
        let mut blob = encrypt(b"top secret seed", "pass").expect("encrypt");
        // Flip a bit in the ciphertext (last byte is part of the GCM tag).
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(decrypt(&blob, "pass").is_err());
    }

    #[test]
    fn empty_passphrase_rejected() {
        assert!(encrypt(b"seed", "").is_err());
    }

    #[test]
    fn distinct_salts_produce_distinct_blobs() {
        // Same input + passphrase should still produce different blobs due to
        // random salt + nonce.
        let a = encrypt(b"seed", "pass").expect("a");
        let b = encrypt(b"seed", "pass").expect("b");
        assert_ne!(a, b);
    }

    /// Backward-compatibility guarantee: a real NHV1 blob (as would exist on
    /// an upgraded user's disk) must still decrypt with the fixed v1 params.
    /// We build one by hand rather than depending on the removed v1 encode
    /// path, so this test locks the on-disk format for existing wallets.
    #[test]
    fn legacy_nhv1_blob_decrypts() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};

        let passphrase = "legacy-pass";
        let plaintext = b"legacy seed material";
        let salt = [0x11u8; 16];
        let nonce_bytes = [0x22u8; NONCE_LEN];

        // Derive the v1 key manually with the fixed v1 params.
        let key_bytes = derive_key(passphrase.as_bytes(), &salt, V1_MEM_KIB, V1_ITERS, V1_LANES)
            .expect("derive v1");
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .expect("encrypt");

        // Hand-assemble an NHV1 blob exactly as v1 wrote them:
        //   magic(4) | salt_len(1) | salt | nonce(12) | ct
        let mut blob = Vec::new();
        blob.extend_from_slice(MAGIC_V1);
        blob.push(salt.len() as u8);
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ct);

        let out = decrypt(&blob, passphrase).expect("decrypt v1");
        assert_eq!(out, plaintext);
    }

    #[test]
    fn v2_blob_has_v2_magic_and_embedded_params() {
        let blob = encrypt(b"seed", "p").expect("encrypt");
        assert_eq!(&blob[0..4], MAGIC_V2);
        let mem = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        let iters = u32::from_le_bytes(blob[8..12].try_into().unwrap());
        let lanes = u32::from_le_bytes(blob[12..16].try_into().unwrap());
        assert_eq!(mem, V2_MEM_KIB);
        assert_eq!(iters, V2_ITERS);
        assert_eq!(lanes, V2_LANES);
    }

    #[test]
    fn v2_out_of_range_params_rejected() {
        // Build a malformed v2 blob that claims 8 GiB memory (above MAX_MEM_KIB).
        // Decrypt must reject it before invoking Argon2, so no OOM/hang risk
        // from untrusted stored blobs.
        let mut blob = Vec::new();
        blob.extend_from_slice(MAGIC_V2);
        blob.extend_from_slice(&(8u32 * 1024 * 1024).to_le_bytes()); // 8 GiB
        blob.extend_from_slice(&V2_ITERS.to_le_bytes());
        blob.extend_from_slice(&V2_LANES.to_le_bytes());
        blob.push(16);
        blob.extend_from_slice(&[0u8; 16]);
        blob.extend_from_slice(&[0u8; NONCE_LEN]);
        blob.extend_from_slice(&[0u8; 32]);
        let err = decrypt(&blob, "p").unwrap_err();
        assert!(matches!(err, AppError::Crypto(_)), "got {err:?}");
    }

    #[test]
    fn v2_truncated_header_rejected() {
        // NHV2 magic but only 6 bytes total — must not panic.
        let blob = [b'N', b'H', b'V', b'2', 0, 0];
        assert!(decrypt(&blob, "p").is_err());
    }

    #[test]
    fn unknown_magic_rejected() {
        let blob = b"NHV9\x00\x00\x00\x00";
        assert!(decrypt(blob, "p").is_err());
    }
}
