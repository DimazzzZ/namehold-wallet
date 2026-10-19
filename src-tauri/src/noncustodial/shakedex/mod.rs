//! Shakedex name sales: lock script, lock keys, price-step template, listing
//! file, verification and purchase plans. Shakedex's own words (auction, bid,
//! presign, proof, fill) are allowed inside this module only; everything it
//! exports speaks the CONTEXT.md vocabulary.

pub mod cancel;
pub mod funding;
pub mod listing_file;
pub mod lock_key;
pub mod purchase;
pub mod script;
pub mod sell;
pub mod template;
pub mod verify;

/// Every Shakedex draft action starts with this (`purchase::PURCHASE_ACTION`,
/// `sell::LOCK_FINALIZE_ACTION`, `cancel::CANCEL_ACTION`, ...): the Ledger
/// guard refuses the whole class by it, so an action constant without the
/// prefix would reach a device. `every_action_constant_has_the_prefix` scans
/// the module sources for any `*_ACTION` constant that lacks it.
pub const ACTION_PREFIX: &str = "shakedex_";

/// Whether a draft's action is a Shakedex one, which the Ledger never signs
/// (R16, R29).
pub fn is_shakedex_action(action: &str) -> bool {
    action.starts_with(ACTION_PREFIX)
}

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

#[cfg(test)]
mod tests {
    use super::ACTION_PREFIX;

    /// Every `pub const <NAME>_ACTION: &str = "..."` in this module's files
    /// starts with [`ACTION_PREFIX`], so a sixth action added without it
    /// fails here rather than slipping past the Ledger's class refusal.
    #[test]
    fn every_action_constant_has_the_prefix() {
        let mut seen = 0;
        for (file, src) in [
            ("cancel.rs", include_str!("cancel.rs")),
            ("funding.rs", include_str!("funding.rs")),
            ("listing_file.rs", include_str!("listing_file.rs")),
            ("lock_key.rs", include_str!("lock_key.rs")),
            ("purchase.rs", include_str!("purchase.rs")),
            ("script.rs", include_str!("script.rs")),
            ("sell.rs", include_str!("sell.rs")),
            ("template.rs", include_str!("template.rs")),
            ("verify.rs", include_str!("verify.rs")),
        ] {
            for line in src.lines() {
                let Some(rest) = line.trim_start().strip_prefix("pub const ") else {
                    continue;
                };
                let Some((name, value)) = rest.split_once(": &str = ") else {
                    continue;
                };
                if !name.ends_with("_ACTION") {
                    continue;
                }
                seen += 1;
                assert!(
                    value.trim_start_matches('"').starts_with(ACTION_PREFIX),
                    "{file}: {name} = {value}"
                );
            }
        }
        assert_eq!(seen, 5, "the five Shakedex action constants");
    }
}
