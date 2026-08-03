//! Handshake name hashing + validation through `hns-covenants`.

use crate::error::AppError;

pub use hns_covenants::MAX_NAME_SIZE;

/// Validate a Handshake name (subset of hsrd `verifyString`): non-empty, <= 63
/// ASCII chars, characters limited to `[a-z0-9]`, `_`, and `-`, with `-`/`_`
/// not allowed at the first or last position.
pub fn verify_name(name: &str) -> bool {
    hns_covenants::validate_name(name.as_bytes())
}

/// Canonical SHA3-256 name hash.
pub fn hash_name(name: &str) -> Result<[u8; 32], AppError> {
    hns_covenants::hash_name(name.as_bytes())
        .map(hns_primitives::NameHash::into_bytes)
        .map_err(|_| AppError::InvalidInput(format!("invalid name '{name}'")))
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
        assert_ne!(
            hash_name("handshake").unwrap(),
            hash_name("namebase").unwrap()
        );
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
