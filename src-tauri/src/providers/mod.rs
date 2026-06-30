//! Multi-provider read architecture.
//!
//! Namehold can read portfolio/wallet data from several backends:
//!   * a local managed `hsd` (full read+write, lifecycle-managed by the app)
//!   * a user-controlled remote `hsd` (read+write only after explicit trust)
//!   * an external read-only explorer (initially HNSFans) used as a fallback
//!     or as the sole source in `external_read_only` mode.
//!
//! Reads may come from a local/remote `hsd` or an external read-only explorer.
//! Writes are non-custodial: they require the local signer to be unlocked AND a
//! broadcaster-capable node source (see `signer::WriteCapability`). External
//! explorers never expose write capability.

pub mod hnsfans;
pub mod signer;

#[allow(unused_imports)]
pub use signer::{
    LocalHotSigner, PlaceholderSigner, SignRequest, SignedTx, SignerBackend, SignerMode,
    WriteCapability,
};
