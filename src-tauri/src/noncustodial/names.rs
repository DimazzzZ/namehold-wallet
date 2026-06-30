//! Handshake name hashing + validation.
//!
//! Verified against hsd v6.1.1 `lib/covenants/rules.js`:
//!   - `hashName(name)` = SHA3-256 of the ASCII name bytes (32-byte output).
//!     (hsd uses `bcrypto/lib/sha3` which is FIPS-202 SHA3-256, NOT Keccak.)
//!   - Names are 1..=63 ASCII chars, lowercase, no leading/trailing hyphen.

use crate::error::AppError;
use sha3::{Digest, Sha3_256};

/// Maximum name length (hsd `rules.MAX_NAME_SIZE`).
pub const MAX_NAME_SIZE: usize = 63;

/// Validate a Handshake name (subset of hsd `verifyString`): non-empty, <= 63
/// ASCII chars, characters limited to `[a-z0-9]`, `_`, and `-`, with `-`/`_`
/// not allowed at the first or last position.
pub fn verify_name(name: &str) -> bool {
    let len = name.len();
    if len == 0 || len > MAX_NAME_SIZE {
        return false;
    }
    for (i, ch) in name.bytes().enumerate() {
        let ok = match ch {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'-' | b'_' => i != 0 && i != len - 1,
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// SHA3-256 hash of a validated name (hsd `rules.hashName`).
pub fn hash_name(name: &str) -> Result<[u8; 32], AppError> {
    if !verify_name(name) {
        return Err(AppError::InvalidInput(format!("invalid name '{name}'")));
    }
    let digest = Sha3_256::digest(name.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

/// The raw on-the-wire name bytes (ASCII), as pushed into covenants.
pub fn raw_name(name: &str) -> Result<Vec<u8>, AppError> {
    if !verify_name(name) {
        return Err(AppError::InvalidInput(format!("invalid name '{name}'")));
    }
    Ok(name.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_name_is_fips_sha3_256() {
        // "abc" is a valid name; FIPS-202 SHA3-256("abc") is a published vector.
        // This proves we use SHA3-256 (not Keccak-256, whose digest differs).
        assert_eq!(
            hex::encode(hash_name("abc").unwrap()),
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
        );
    }

    #[test]
    fn hash_name_is_deterministic_and_32_bytes() {
        let a = hash_name("handshake").unwrap();
        let b = hash_name("handshake").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert_ne!(hash_name("handshake").unwrap(), hash_name("namebase").unwrap());
    }

    #[test]
    fn verify_name_rules() {
        assert!(verify_name("abc"));
        assert!(verify_name("a-b-c"));
        assert!(verify_name("name123"));
        assert!(!verify_name("")); // empty
        assert!(!verify_name("-abc")); // leading hyphen
        assert!(!verify_name("abc-")); // trailing hyphen
        assert!(!verify_name("ABC")); // uppercase
        assert!(!verify_name("a.b")); // dot not allowed
        assert!(!verify_name(&"a".repeat(64))); // too long
        assert!(hash_name("ABC").is_err());
    }
}
