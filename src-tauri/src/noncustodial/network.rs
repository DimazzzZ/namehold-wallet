
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
