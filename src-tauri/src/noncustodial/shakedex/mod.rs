//! Shakedex name sales: lock script, lock keys, price-step template, listing
//! file, verification and purchase plans. Shakedex's own words (auction, bid,
//! presign, proof, fill) are allowed inside this module only; everything it
//! exports speaks the CONTEXT.md vocabulary.

pub mod funding;
pub mod listing_file;
pub mod lock_key;
pub mod purchase;
pub mod script;
pub mod sell;
pub mod template;
pub mod verify;

/// Why any profile but a recovery-phrase (mnemonic) one is refused (R16): a
/// Ledger, a watch-only profile or an imported extended private key. The UI
/// shows the same sentence on its disabled buttons
/// (`marketText.ts::RECOVERY_PHRASE_ONLY`).
pub const RECOVERY_PHRASE_ONLY: &str = "Shakedex works with a recovery-phrase wallet for now";

/// Why a new mainnet purchase is refused until Settings allows it (R15). The
/// UI shows the same sentence on its disabled Buy
/// (`marketText.ts::MAINNET_EXPERIMENTAL`).
pub const MAINNET_EXPERIMENTAL: &str =
    "Shakedex purchases on mainnet are experimental: enable them in Settings";
