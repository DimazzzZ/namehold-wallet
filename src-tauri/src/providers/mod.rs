//! Multi-provider read architecture.
//!
//! Namehold can read portfolio/wallet data from several backends:
//!   * a local managed `hsd` (full read+write, lifecycle-managed by the app)
//!   * a user-controlled remote `hsd` (read+write only after explicit trust)
//!   * an external read-only explorer (initially HNSFans) used as a fallback
//!     or as the sole source in `external_read_only` mode.
//!
//! Writes are *only* ever permitted against a trusted `hsd` backend. External
//! providers are strictly read-only and never expose any write capability.

pub mod hnsfans;
pub mod resolver;
pub mod signer;
pub mod types;

pub use resolver::resolve_read_context;
// Placeholder signer abstraction; reserved for the upcoming write path. Not yet
// consumed by any command, so the re-exports are intentionally allowed unused.
#[allow(unused_imports)]
pub use signer::{PlaceholderSigner, SignRequest, SignedTx, SignerBackend, SignerMode};
pub use types::*;
