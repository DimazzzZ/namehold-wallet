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
#[allow(unused_imports)]
pub use hnsfans::ExplorerProvider;

/// The ONE place settings turn into an explorer client (Task 11 / S1).
///
/// Before this, `HnsFansClient::new(...)` was constructed at three separate
/// call sites (`commands/sync.rs` x2, `commands/read.rs`), each re-deriving
/// `explorer_api_url` from settings with its own copy of the
/// trim/filter-empty/default logic — and the default URL was hard-coded at
/// each of those sites too. Every construction site now calls this instead,
/// so there is exactly one settings key read and one fallback default
/// ([`hnsfans::DEFAULT_EXPLORER_URL`]) in the whole app.
pub fn explorer_client_from_settings(
    settings: &crate::models::settings::SettingsMap,
) -> hnsfans::HnsFansClient {
    let url = settings
        .get("explorer_api_url")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(hnsfans::DEFAULT_EXPLORER_URL);
    hnsfans::HnsFansClient::new(url)
}
