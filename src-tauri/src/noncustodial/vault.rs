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
//!   magic:    4 bytes  = b"NHV1"        (Namehold Vault v1)
//!   salt_len: 1 byte
//!   salt:     salt_len bytes            (Argon2 salt)
//!   nonce:    12 bytes                  (AES-GCM nonce)
//!   ct:       remainder                 (ciphertext + 16-byte GCM tag)
//!
//! The Argon2 parameters are fixed for v1 and recorded by the magic version so
//! decryption uses the same cost parameters that were used to encrypt.

use crate::error::AppError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::Zeroize;

const MAGIC: &[u8; 4] = b"NHV1";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

// Argon2id v1 cost parameters. These are deliberately conservative for a
// desktop wallet: 64 MiB memory, 3 iterations, 1 lane.
const ARGON_MEM_KIB: u32 = 64 * 1024;
const ARGON_ITERS: u32 = 3;
const ARGON_LANES: u32 = 1;

/// Derive a 32-byte AES key from a passphrase + salt using Argon2id with the
/// fixed v1 parameters.
fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; KEY_LEN], AppError> {
    let params = Params::new(ARGON_MEM_KIB, ARGON_ITERS, ARGON_LANES, Some(KEY_LEN))
        .map_err(|e| AppError::Crypto(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| AppError::Crypto(format!("argon2 derive: {e}")))?;
    Ok(key)
}

/// Encrypt `plaintext` (e.g. the mnemonic bytes) under `passphrase`, returning
/// the self-describing vault blob.
pub fn encrypt(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, AppError> {
    if passphrase.is_empty() {
        return Err(AppError::InvalidInput("passphrase must not be empty".into()));
    }

    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let mut key_bytes = derive_key(passphrase.as_bytes(), &salt)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::Crypto(format!("aes-gcm encrypt: {e}")))?;
    key_bytes.zeroize();

    // Assemble blob: magic | salt_len | salt | nonce | ciphertext
    let mut blob = Vec::with_capacity(4 + 1 + salt.len() + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.push(salt.len() as u8);
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Decrypt a vault blob produced by [`encrypt`] using `passphrase`.
///
/// Returns `AppError::Crypto` on a wrong passphrase or tampered blob (GCM auth
/// failure is indistinguishable from a wrong key, by design).
pub fn decrypt(blob: &[u8], passphrase: &str) -> Result<Vec<u8>, AppError> {
    if blob.len() < 4 + 1 {
        return Err(AppError::Crypto("vault blob too short".into()));
    }
    if &blob[0..4] != MAGIC {
        return Err(AppError::Crypto("unrecognized vault format/version".into()));
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

    let mut key_bytes = derive_key(passphrase.as_bytes(), salt)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AppError::Crypto("decryption failed (wrong passphrase or corrupt data)".into()));
    key_bytes.zeroize();
    plaintext
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_recovers_plaintext() {
        let secret = b"abandon abandon abandon abandon about";
        let blob = encrypt(secret, "correct horse battery staple").expect("encrypt");
        // Blob must be self-describing and not contain the plaintext.
        assert_eq!(&blob[0..4], MAGIC);
        assert!(!blob
            .windows(secret.len())
            .any(|w| w == secret));
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
}
