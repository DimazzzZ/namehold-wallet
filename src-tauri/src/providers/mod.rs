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
pub mod ledger;
pub mod signer;

#[allow(unused_imports)]
pub use hnsfans::ExplorerProvider;
#[allow(unused_imports)]
pub use signer::{
    LocalHotSigner, PlaceholderSigner, SignRequest, SignedTx, SignerBackend, SignerMode,
    WriteCapability,
};

/// What to tell the user when [`explorer_client_from_settings`] returns `None`
/// and the caller needed it.
///
/// One sentence, in one place, because it was written four different ways
/// across the read and sync paths — and it has to cover both reasons the
/// factory refuses: the profile's network has no explorer configured, or the
/// network could not be read at all. Either way the actionable part is the
/// same, and naming the setting beats degrading to empty or to mainnet data.
pub const EXPLORER_UNAVAILABLE: &str =
    "No explorer is available for this wallet's network, and the local node is \
     not synced. Set 'explorer_api_url' in Settings, or wait for the node to \
     finish syncing.";

/// The ONE place settings turn into an explorer client (Task 11 / S1, G2).
///
/// Before this, `HnsFansClient::new(...)` was constructed at three separate
/// call sites (`commands/sync.rs` x2, `commands/read.rs`), each re-deriving
/// `explorer_api_url` from settings with its own copy of the
/// trim/filter-empty/default logic — and the default URL was hard-coded at
/// each of those sites too. Every construction site now calls this instead,
/// so there is exactly one settings key read in the whole app.
///
/// Network-aware (G2): the HNSFans default explorer only serves *mainnet*
/// data. On testnet/regtest/simnet there is no known public explorer, so
/// pointing a non-mainnet wallet at `e.hnsfans.com` produced false "no data"
/// results. Resolution order:
///   1. explicit `explorer_api_url` from settings, unless it is the known
///      mainnet explorer and the profile is not on mainnet, else
///   2. [`Network::default_explorer_base_url`] (mainnet only), else
///   3. `None` — the explorer fallback is *disabled* and callers must degrade
///      to cache or a candid "explorer unavailable" error instead of mainnet.
///
/// Step 1 refuses the mainnet explorer off mainnet because the stored setting
/// is not always something the user chose: migration 009 seeded it with the
/// mainnet URL, and every installation that ran that version carries the value
/// still (032 clears exactly that seeded value, but a database can reach this
/// code before migrations of a newer build have run). An explicit URL outranks
/// the network default, so without this check the seeded mainnet URL silently
/// won on a testnet or regtest profile — the cross-network read the guard is
/// for. Any other URL is the user's own and is honoured on every network: the
/// wallet cannot know which chain a private explorer serves.
///
/// If `explorer_fallback_url` is set in settings, the client will
/// automatically fail over to it when the primary explorer is unreachable.
pub fn explorer_client_from_settings(
    settings: &crate::models::settings::SettingsMap,
    network: crate::noncustodial::network::Network,
) -> Option<hnsfans::HnsFansClient> {
    let explicit = settings
        .get("explorer_api_url")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter(|u| {
            network == crate::noncustodial::network::Network::Main
                || u.trim_end_matches('/') != hnsfans::DEFAULT_EXPLORER_URL
        });
    let url = match explicit {
        Some(u) => u,
        None => network.default_explorer_base_url()?,
    };
    let fallback = settings
        .get("explorer_fallback_url")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    Some(match fallback {
        Some(fb) => hnsfans::HnsFansClient::with_fallback(url, fb),
        None => hnsfans::HnsFansClient::new(url),
    })
}
