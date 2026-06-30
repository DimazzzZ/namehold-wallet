//! Future signing abstraction (placeholder).
//!
//! Today, all writes are performed by a trusted `hsd` backend which holds the
//! keys and signs transactions internally. The `future_signer_mode` setting
//! (`none` | `local_signer_planned`) anticipates a future where Namehold owns
//! key material and signs transactions locally, sending only fully-signed
//! transactions to a (potentially untrusted) node for broadcast.
//!
//! This module intentionally ships only the trait + types so the rest of the
//! codebase can reference a stable shape. No real signing is implemented yet;
//! the placeholder implementation returns `AppError::Other` to make accidental
//! use loud and obvious.

use crate::error::AppError;

/// Which signing strategy is active. Mirrors the `future_signer_mode` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignerMode {
    /// No local signer. Writes go through a trusted hsd which signs internally.
    None,
    /// Reserved: Namehold will hold keys and sign locally. Not yet implemented.
    LocalSignerPlanned,
}

impl SignerMode {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "local_signer_planned" => SignerMode::LocalSignerPlanned,
            _ => SignerMode::None,
        }
    }

    pub fn as_setting(&self) -> &'static str {
        match self {
            SignerMode::None => "none",
            SignerMode::LocalSignerPlanned => "local_signer_planned",
        }
    }
}

/// A request to sign an arbitrary transaction payload. The concrete encoding is
/// intentionally opaque (raw bytes) so this trait does not bind to a specific
/// transaction format ahead of the actual implementation.
#[derive(Debug, Clone)]
pub struct SignRequest {
    /// Unsigned transaction bytes (encoding defined by the future implementation).
    pub unsigned_tx: Vec<u8>,
    /// Optional human-readable summary for confirmation UIs.
    pub summary: Option<String>,
}

/// The result of a successful signing operation.
#[derive(Debug, Clone)]
pub struct SignedTx {
    /// Fully-signed transaction bytes, ready to broadcast.
    pub signed_tx: Vec<u8>,
}

/// Abstraction over "the thing that turns an unsigned transaction into a signed
/// one". Implementations may keep keys in-process, in an OS keychain, or on an
/// external device. Reads never go through here.
pub trait SignerBackend: Send + Sync {
    /// Whether this signer is currently able to sign (unlocked, device present).
    fn is_available(&self) -> bool;

    /// Sign the given request, returning fully-signed transaction bytes.
    fn sign(&self, request: &SignRequest) -> Result<SignedTx, AppError>;
}

/// Placeholder signer used while `local_signer_planned` is selected but no real
/// implementation exists. It is never available and always errors on `sign`,
/// guaranteeing that no half-built signing path can be exercised by mistake.
pub struct PlaceholderSigner;

impl SignerBackend for PlaceholderSigner {
    fn is_available(&self) -> bool {
        false
    }

    fn sign(&self, _request: &SignRequest) -> Result<SignedTx, AppError> {
        Err(AppError::Other(
            "Local signing is not implemented yet. Writes currently require a trusted hsd."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_mode_round_trips() {
        assert_eq!(SignerMode::from_setting("none"), SignerMode::None);
        assert_eq!(
            SignerMode::from_setting("local_signer_planned"),
            SignerMode::LocalSignerPlanned
        );
        // Unknown values default to None.
        assert_eq!(SignerMode::from_setting("bogus"), SignerMode::None);
        assert_eq!(SignerMode::None.as_setting(), "none");
        assert_eq!(
            SignerMode::LocalSignerPlanned.as_setting(),
            "local_signer_planned"
        );
    }

    #[test]
    fn placeholder_signer_is_never_available_and_errors() {
        let signer = PlaceholderSigner;
        assert!(!signer.is_available());
        let req = SignRequest {
            unsigned_tx: vec![1, 2, 3],
            summary: None,
        };
        assert!(signer.sign(&req).is_err());
    }
}
