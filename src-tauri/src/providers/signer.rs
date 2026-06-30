//! Signing abstraction + non-custodial write-capability gate.
//!
//! Namehold is non-custodial: it owns key material and signs transactions
//! locally, sending only fully-signed transactions to a node for broadcast.
//! Write capability therefore depends on TWO things being true at once:
//!   1. the local signer session is unlocked (keys in memory), and
//!   2. a broadcaster is available (a node RPC source that can `sendrawtransaction`).
//!
//! [`WriteCapability`] captures that gate for the frontend. The actual
//! covenant-aware signing of a Handshake transaction lives in
//! `noncustodial::{send, tx}` — that path needs per-input prevout values and
//! BIP44 derivation paths, which the byte-opaque [`SignerBackend`] trait cannot
//! carry. The trait is retained as a stable seam for future hardware/multisig
//! backends; [`LocalHotSigner`] reports availability through it.

use crate::error::AppError;
use crate::noncustodial::rpc::ChainSource;
use serde::Serialize;

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

/// Placeholder signer for explicitly-unsupported modes (e.g. a future hardware
/// backend that isn't wired yet). Never available; always errors on `sign`.
pub struct PlaceholderSigner;

impl SignerBackend for PlaceholderSigner {
    fn is_available(&self) -> bool {
        false
    }

    fn sign(&self, _request: &SignRequest) -> Result<SignedTx, AppError> {
        Err(AppError::Other(
            "This signer backend is not available.".to_string(),
        ))
    }
}

/// The local hot signer. Availability mirrors whether the in-memory signer
/// session is unlocked. Byte-level `sign` is intentionally unsupported: the
/// covenant-aware signing path is `noncustodial::send`/`tx`, which has the
/// prevout + derivation context this trait lacks.
pub struct LocalHotSigner {
    available: bool,
}

impl LocalHotSigner {
    pub fn new(available: bool) -> Self {
        Self { available }
    }
}

impl SignerBackend for LocalHotSigner {
    fn is_available(&self) -> bool {
        self.available
    }

    fn sign(&self, _request: &SignRequest) -> Result<SignedTx, AppError> {
        Err(AppError::Other(
            "Local signing of Handshake transactions is performed via the draft \
             sign flow (build → sign → broadcast), not raw byte signing."
                .to_string(),
        ))
    }
}

/// Non-custodial write capability: writes require BOTH an unlocked signer AND a
/// broadcaster that can `sendrawtransaction`. Serialized to the frontend so the
/// UI can enable/disable spend actions with an accurate reason.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteCapability {
    pub signer_unlocked: bool,
    pub broadcaster_available: bool,
    pub can_write: bool,
    pub reason: Option<String>,
}

impl WriteCapability {
    /// Evaluate the gate from the unlocked-signer flag and the chain source.
    /// `allow_remote` mirrors the `allow_remote_broadcast` setting and only
    /// matters for a remote node source.
    pub fn evaluate(signer_unlocked: bool, source: ChainSource, allow_remote: bool) -> Self {
        let broadcaster_available = match source {
            ChainSource::LocalNode => true,
            ChainSource::RemoteNode => allow_remote,
            ChainSource::Explorer => false,
        };
        let reason = if !signer_unlocked && !broadcaster_available {
            Some("Unlock your wallet and configure a node that can broadcast.".to_string())
        } else if !signer_unlocked {
            Some("Unlock your wallet to sign transactions.".to_string())
        } else if !broadcaster_available {
            Some(match source {
                ChainSource::Explorer => {
                    "The configured chain source is read-only and cannot broadcast.".to_string()
                }
                ChainSource::RemoteNode => {
                    "Remote broadcast is disabled (enable allow_remote_broadcast).".to_string()
                }
                ChainSource::LocalNode => unreachable!(),
            })
        } else {
            None
        };
        Self {
            signer_unlocked,
            broadcaster_available,
            can_write: signer_unlocked && broadcaster_available,
            reason,
        }
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

    #[test]
    fn local_hot_signer_reflects_availability() {
        assert!(LocalHotSigner::new(true).is_available());
        assert!(!LocalHotSigner::new(false).is_available());
        // Byte-level signing is intentionally unsupported (use the draft flow).
        assert!(LocalHotSigner::new(true)
            .sign(&SignRequest {
                unsigned_tx: vec![],
                summary: None
            })
            .is_err());
    }

    #[test]
    fn write_capability_requires_unlock_and_broadcaster() {
        // Locked + local node: no write, reason mentions unlocking.
        let c = WriteCapability::evaluate(false, ChainSource::LocalNode, false);
        assert!(!c.can_write);
        assert!(c.broadcaster_available);
        assert!(c.reason.unwrap().to_lowercase().contains("unlock"));

        // Unlocked + local node: can write.
        let c = WriteCapability::evaluate(true, ChainSource::LocalNode, false);
        assert!(c.can_write);
        assert!(c.reason.is_none());

        // Unlocked + explorer: read-only source blocks broadcast.
        let c = WriteCapability::evaluate(true, ChainSource::Explorer, false);
        assert!(!c.can_write);
        assert!(!c.broadcaster_available);

        // Unlocked + remote node, opt-in off vs on.
        assert!(!WriteCapability::evaluate(true, ChainSource::RemoteNode, false).can_write);
        assert!(WriteCapability::evaluate(true, ChainSource::RemoteNode, true).can_write);
    }
}
