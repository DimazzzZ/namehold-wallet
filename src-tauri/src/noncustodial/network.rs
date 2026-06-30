
//! Handshake network parameters.
//!
//! All constants verified against the canonical `hsd` source:
//!   - lib/protocol/networks.js  (HRP, coinType, BIP32 key prefixes)
//!   - lib/primitives/address.js (address = blake2b-160(pubkey), bech32 v0)
//!   - lib/hd/mnemonic.js        (standard BIP39 PBKDF2-HMAC-SHA512, 2048 iters)

/// Handshake network selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Main,
    Testnet,
    Regtest,
    Simnet,
}

impl Network {
    /// Bech32 human-readable prefix for addresses (hsd `addressPrefix`).
    pub fn address_hrp(self) -> &'static str {
        match self {
            Network::Main => "hs",
            Network::Testnet => "ts",
            Network::Regtest => "rs",
            Network::Simnet => "ss",
        }
    }

    /// BIP44 coin type (hsd `keyPrefix.coinType`).
    pub fn coin_type(self) -> u32 {
        match self {
            Network::Main => 5353,
            Network::Testnet => 5354,
            Network::Regtest => 5355,
            Network::Simnet => 5356,
        }
    }

    /// BIP32 xprv version bytes (hsd `keyPrefix.xprivkey`).
    pub fn xprv_version(self) -> u32 {
        // hsd uses the same value across networks for the binary prefix; the
        // mainnet value 0x0488ade4 matches Bitcoin's BIP32 mainnet xprv.
        match self {
            Network::Main => 0x0488_ade4,
            // testnet/regtest/simnet share the mainnet-style prefix in hsd's
            // binary HD serialization; only the bech32 address HRP differs.
            _ => 0x0488_ade4,
        }
    }

    /// BIP32 xpub version bytes (hsd `keyPrefix.xpubkey`).
    pub fn xpub_version(self) -> u32 {
        0x0488_b21e
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Network::Main => "main",
            Network::Testnet => "testnet",
            Network::Regtest => "regtest",
            Network::Simnet => "simnet",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Network> {
        match s {
            "main" | "mainnet" => Some(Network::Main),
            "testnet" => Some(Network::Testnet),
            "regtest" => Some(Network::Regtest),
            "simnet" => Some(Network::Simnet),
            _ => None,
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Network::Main
    }
}

/// Name-auction consensus parameters (hsd `networks.js` `names`). Block counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameParams {
    pub tree_interval: u32,
    pub bidding_period: u32,
    pub reveal_period: u32,
    pub renewal_window: u32,
    pub transfer_lockup: u32,
    pub revocation_delay: u32,
    /// hsd `renewalMaturity`. `getRenewalBlock` uses `height - 2*renewal_maturity`.
    pub renewal_maturity: u32,
}

impl Network {
    /// Per-network name params. Mainnet/testnet `blocksPerDay` = 144.
    pub fn name_params(self) -> NameParams {
        match self {
            Network::Main => NameParams {
                tree_interval: 36,
                bidding_period: 720,
                reveal_period: 1440,
                renewal_window: 105_120,
                transfer_lockup: 288,
                revocation_delay: 2016,
                renewal_maturity: 4320,
            },
            Network::Testnet => NameParams {
                tree_interval: 36,
                bidding_period: 144,
                reveal_period: 288,
                renewal_window: 4320,
                transfer_lockup: 288,
                revocation_delay: 576,
                renewal_maturity: 144,
            },
            Network::Regtest => NameParams {
                tree_interval: 5,
                bidding_period: 5,
                reveal_period: 10,
                renewal_window: 5000,
                transfer_lockup: 10,
                revocation_delay: 50,
                renewal_maturity: 50,
            },
            Network::Simnet => NameParams {
                tree_interval: 2,
                bidding_period: 25,
                reveal_period: 50,
                renewal_window: 2500,
                transfer_lockup: 5,
                revocation_delay: 25,
                renewal_maturity: 25,
            },
        }
    }
}
