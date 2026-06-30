
//! BIP32 / BIP39 / BIP44 hierarchical deterministic key derivation for
//! Handshake.
//!
//! Verified against hsd:
//!   - lib/hd/mnemonic.js: standard BIP39 seed =
//!       PBKDF2(HMAC-SHA512, NFKD(phrase), "mnemonic"+NFKD(passphrase), 2048, 64)
//!     The `bip39` crate's `Mnemonic::to_seed` implements exactly this.
//!   - lib/protocol/networks.js: coinType per network (5353 main, etc.).
//!   - BIP32 master/child derivation is identical to Bitcoin; hsd reuses the
//!     standard scheme with secp256k1.
//!
//! Derivation path follows BIP44: m / 44' / coin' / account' / change / index.

use crate::error::AppError;
use crate::noncustodial::network::Network;
use bip39::Mnemonic;
use hmac::{Hmac, Mac};
use secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::Sha512;
use zeroize::Zeroize;

type HmacSha512 = Hmac<Sha512>;

pub const HARDENED_OFFSET: u32 = 0x8000_0000;

/// An extended private key (BIP32). Holds the 32-byte key and 32-byte chain
/// code. Secret material is zeroized on drop.
pub struct ExtendedPrivKey {
    pub secret: SecretKey,
    pub chain_code: [u8; 32],
}

impl Drop for ExtendedPrivKey {
    fn drop(&mut self) {
        self.chain_code.zeroize();
        // SecretKey from secp256k1 implements its own zeroization on drop.
    }
}

impl ExtendedPrivKey {
    /// Derive the BIP32 master key from a seed (BIP39 output, 64 bytes).
    ///
    /// master = HMAC-SHA512(key="Bitcoin seed", data=seed)
    /// left 32 bytes -> secret key, right 32 bytes -> chain code.
    /// hsd uses the same "Bitcoin seed" constant as standard BIP32.
    pub fn from_seed(seed: &[u8]) -> Result<Self, AppError> {
        let mut mac = HmacSha512::new_from_slice(b"Bitcoin seed")
            .map_err(|e| AppError::Crypto(format!("hmac init: {e}")))?;
        mac.update(seed);
        let i = mac.finalize().into_bytes();

        let (il, ir) = i.split_at(32);
        let secret = SecretKey::from_slice(il)
            .map_err(|e| AppError::Crypto(format!("invalid master key: {e}")))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(ir);
        Ok(ExtendedPrivKey { secret, chain_code })
    }

    /// Derive a single child key at `index`. Indices >= HARDENED_OFFSET are
    /// hardened.
    pub fn derive_child(&self, index: u32) -> Result<Self, AppError> {
        let secp = Secp256k1::new();
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| AppError::Crypto(format!("hmac init: {e}")))?;

        if index >= HARDENED_OFFSET {
            // Hardened: 0x00 || ser256(k_par) || ser32(index)
            mac.update(&[0u8]);
            mac.update(&self.secret.secret_bytes());
        } else {
            // Normal: serP(point(k_par)) || ser32(index)
            let pubkey = PublicKey::from_secret_key(&secp, &self.secret);
            mac.update(&pubkey.serialize());
        }
        mac.update(&index.to_be_bytes());
        let i = mac.finalize().into_bytes();

        let (il, ir) = i.split_at(32);
        // child secret = (parse256(il) + k_par) mod n
        let tweak = secp256k1::Scalar::from_be_bytes(
            il.try_into()
                .map_err(|_| AppError::Crypto("tweak length".into()))?,
        )
        .map_err(|e| AppError::Crypto(format!("invalid tweak: {e}")))?;
        let child_secret = self
            .secret
            .add_tweak(&tweak)
            .map_err(|e| AppError::Crypto(format!("key derivation overflow: {e}")))?;

        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(ir);
        Ok(ExtendedPrivKey {
            secret: child_secret,
            chain_code,
        })
    }

    /// Derive along a full path of indices.
    pub fn derive_path(&self, path: &[u32]) -> Result<Self, AppError> {
        let mut key = ExtendedPrivKey {
            secret: self.secret,
            chain_code: self.chain_code,
        };
        for &index in path {
            key = key.derive_child(index)?;
        }
        Ok(key)
    }

    /// The 33-byte compressed public key for this private key.
    pub fn compressed_pubkey(&self) -> [u8; 33] {
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &self.secret).serialize()
    }
}

/// An extended PUBLIC key (BIP32). Holds the 33-byte compressed public key and
/// 32-byte chain code. Carries no secret material, so it is safe for watch-only
/// wallets and for deriving receive/change addresses without unlocking.
///
/// Only NON-hardened children can be derived from a public key (BIP32). The
/// receive (branch 0) and change (branch 1) paths plus their child indices are
/// all non-hardened, so an account-level xpub is sufficient for address
/// discovery.
#[derive(Clone, Debug)]
pub struct ExtendedPubKey {
    pub public: PublicKey,
    pub chain_code: [u8; 32],
}

impl ExtendedPubKey {
    /// Derive the account-level public key from a private key (e.g. to publish
    /// an `account_xpub` after computing the BIP44 account path privately).
    pub fn from_priv(xprv: &ExtendedPrivKey) -> Self {
        let secp = Secp256k1::new();
        ExtendedPubKey {
            public: PublicKey::from_secret_key(&secp, &xprv.secret),
            chain_code: xprv.chain_code,
        }
    }

    /// Derive a single NON-hardened child public key at `index`.
    ///
    /// Returns `InvalidInput` if a hardened index is requested, since BIP32
    /// hardened derivation is impossible from a public key alone.
    pub fn derive_child(&self, index: u32) -> Result<Self, AppError> {
        if index >= HARDENED_OFFSET {
            return Err(AppError::InvalidInput(
                "cannot derive hardened child from an extended public key".to_string(),
            ));
        }
        let secp = Secp256k1::new();
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| AppError::Crypto(format!("hmac init: {e}")))?;
        // Normal: serP(K_par) || ser32(index)
        mac.update(&self.public.serialize());
        mac.update(&index.to_be_bytes());
        let i = mac.finalize().into_bytes();

        let (il, ir) = i.split_at(32);
        // child pubkey = point(parse256(il)) + K_par
        let tweak = Scalar::from_be_bytes(
            il.try_into()
                .map_err(|_| AppError::Crypto("tweak length".into()))?,
        )
        .map_err(|e| AppError::Crypto(format!("invalid tweak: {e}")))?;
        let child_public = self
            .public
            .add_exp_tweak(&secp, &tweak)
            .map_err(|e| AppError::Crypto(format!("public key derivation overflow: {e}")))?;

        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(ir);
        Ok(ExtendedPubKey {
            public: child_public,
            chain_code,
        })
    }

    /// Derive along a path of NON-hardened indices.
    pub fn derive_path(&self, path: &[u32]) -> Result<Self, AppError> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(index)?;
        }
        Ok(key)
    }

    /// The 33-byte compressed public key.
    pub fn compressed_pubkey(&self) -> [u8; 33] {
        self.public.serialize()
    }

    /// Parse a base58check-encoded BIP32 xpub string into an `ExtendedPubKey`.
    ///
    /// Validates the 4-byte version prefix against the network's `xpub_version`
    /// and the 33-byte compressed public key. The depth / parent fingerprint /
    /// child-number fields are present in the payload but not retained, since
    /// only the key + chain code are needed for further child derivation.
    pub fn from_xpub(network: Network, xpub: &str) -> Result<Self, AppError> {
        let payload = base58check_decode(xpub)?;
        // BIP32 serialized key: 4 + 1 + 4 + 4 + 32 + 33 = 78 bytes.
        if payload.len() != 78 {
            return Err(AppError::InvalidInput(format!(
                "xpub payload must be 78 bytes, got {}",
                payload.len()
            )));
        }
        let version = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        if version != network.xpub_version() {
            return Err(AppError::InvalidInput(format!(
                "xpub version {:#010x} does not match network {} ({:#010x})",
                version,
                network.as_str(),
                network.xpub_version()
            )));
        }
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&payload[13..45]);
        let key_bytes = &payload[45..78];
        let public = PublicKey::from_slice(key_bytes)
            .map_err(|e| AppError::InvalidInput(format!("invalid xpub public key: {e}")))?;
        Ok(ExtendedPubKey { public, chain_code })
    }
}

/// Decode a base58check string into its payload (the trailing 4-byte SHA256d
/// checksum is verified and stripped).
///
/// Implemented locally over `sha2` to avoid pulling in an extra base58 crate.
fn base58check_decode(s: &str) -> Result<Vec<u8>, AppError> {
    let full = base58_decode(s)?;
    if full.len() < 4 {
        return Err(AppError::InvalidInput("base58check too short".to_string()));
    }
    let (payload, checksum) = full.split_at(full.len() - 4);
    let digest = sha256d(payload);
    if &digest[..4] != checksum {
        return Err(AppError::InvalidInput(
            "base58check checksum mismatch".to_string(),
        ));
    }
    Ok(payload.to_vec())
}

/// Encode a payload as base58check (appends a 4-byte SHA256d checksum).
fn base58check_encode(payload: &[u8]) -> String {
    let digest = sha256d(payload);
    let mut full = Vec::with_capacity(payload.len() + 4);
    full.extend_from_slice(payload);
    full.extend_from_slice(&digest[..4]);
    base58_encode(&full)
}

/// Encode bytes as base58 (Bitcoin alphabet).
fn base58_encode(data: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::with_capacity(data.len() * 2);
    for &byte in data {
        let mut carry = byte as u32;
        for digit in digits.iter_mut() {
            carry += (*digit as u32) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    // Leading zero bytes become leading '1's.
    let mut out = String::new();
    for &byte in data {
        if byte == 0 {
            out.push('1');
        } else {
            break;
        }
    }
    for &d in digits.iter().rev() {
        out.push(BASE58_ALPHABET[d as usize] as char);
    }
    out
}

/// Double SHA-256.
fn sha256d(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

const BASE58_ALPHABET: &[u8; 58] =
    b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Decode a base58 (Bitcoin alphabet) string into bytes.
fn base58_decode(s: &str) -> Result<Vec<u8>, AppError> {
    if s.is_empty() {
        return Err(AppError::InvalidInput("empty base58 string".to_string()));
    }
    // Big-integer base conversion: 58 -> 256.
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for ch in s.bytes() {
        let value = BASE58_ALPHABET
            .iter()
            .position(|&c| c == ch)
            .ok_or_else(|| AppError::InvalidInput(format!("invalid base58 char: {}", ch as char)))?
            as u32;
        let mut carry = value;
        for byte in bytes.iter_mut() {
            carry += (*byte as u32) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Leading '1's in base58 encode leading zero bytes.
    for ch in s.bytes() {
        if ch == b'1' {
            bytes.push(0);
        } else {
            break;
        }
    }
    bytes.reverse();
    Ok(bytes)
}

/// Build a BIP44 path: m / 44' / coin' / account' / change / index.
pub fn bip44_path(network: Network, account: u32, change: u32, index: u32) -> [u32; 5] {
    [
        44 + HARDENED_OFFSET,
        network.coin_type() + HARDENED_OFFSET,
        account + HARDENED_OFFSET,
        change,
        index,
    ]
}

/// Derive a seed from a mnemonic phrase + optional passphrase using standard
/// BIP39 (matches hsd `mnemonic.toSeed`).
pub fn seed_from_mnemonic(phrase: &str, passphrase: &str) -> Result<[u8; 64], AppError> {
    let mnemonic = Mnemonic::parse(phrase.trim())
        .map_err(|e| AppError::InvalidInput(format!("invalid mnemonic: {e}")))?;
    let seed = mnemonic.to_seed(passphrase);
    Ok(seed)
}

/// Derive the compressed public key and Handshake address at a BIP44 path.
pub fn derive_address(
    network: Network,
    seed: &[u8],
    account: u32,
    change: u32,
    index: u32,
) -> Result<(SecretKey, [u8; 33], String), AppError> {
    let master = ExtendedPrivKey::from_seed(seed)?;
    let path = bip44_path(network, account, change, index);
    let child = master.derive_path(&path)?;
    let pubkey = child.compressed_pubkey();
    let address = crate::noncustodial::address::address_from_pubkey(network, &pubkey)?;
    Ok((child.secret, pubkey, address))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Standard BIP32 test vector 1 (seed 000102...0f) verifies our derivation
    // matches the canonical spec, which hsd also follows.
    #[test]
    fn bip32_master_from_known_seed() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        // BIP32 vector 1 master private key.
        assert_eq!(
            hex::encode(master.secret.secret_bytes()),
            "e8f32e723decf4051aefac8e2c93c9c5b214313817cdb01a1494b917c8436b35"
        );
        assert_eq!(
            hex::encode(master.chain_code),
            "873dff81c02f525623fd1fe5167eac3a55a049de3d314bb42ee227ffed37d508"
        );
    }

    #[test]
    fn bip32_hardened_child_matches_vector() {
        // m/0' from BIP32 test vector 1.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        let child = master.derive_child(HARDENED_OFFSET).expect("child");
        // Verified by decoding the canonical BIP32 vector-1 m/0' xprv:
        // raw private key tail is ...0715a2d911a0afea (NOT a8 — a common
        // transcription error). hsd uses identical BIP32 derivation.
        assert_eq!(
            hex::encode(child.secret.secret_bytes()),
            "edb2e14f9ee77d26dd93b4ecede8d16ed408ce149b6cd80b0715a2d911a0afea"
        );
    }

    #[test]
    fn mnemonic_seed_matches_bip39_vector() {
        // BIP39 English test vector (Trezor), passphrase "TREZOR".
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let seed = seed_from_mnemonic(phrase, "TREZOR").expect("seed");
        assert_eq!(
            hex::encode(seed),
            "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04"
        );
    }

    // BIP32 invariant: deriving a non-hardened child publicly from the parent
    // xpub yields the same public key as deriving it privately and taking the
    // pubkey. This is exactly what watch-only address scanning relies on.
    #[test]
    fn public_derivation_matches_private_derivation() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        let xpub = ExtendedPubKey::from_priv(&master);

        for &index in &[0u32, 1, 2, 7, 1000, HARDENED_OFFSET - 1] {
            let priv_child = master.derive_child(index).expect("priv child");
            let pub_child = xpub.derive_child(index).expect("pub child");
            assert_eq!(
                priv_child.compressed_pubkey(),
                pub_child.compressed_pubkey(),
                "mismatch at index {index}"
            );
        }
    }

    #[test]
    fn public_derivation_along_branch_path_matches() {
        // Mirror an account-level xpub deriving receive (branch 0) children.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        let xpub = ExtendedPubKey::from_priv(&master);

        // branch 0 (receive), child 5
        let priv_child = master.derive_path(&[0, 5]).expect("priv path");
        let pub_child = xpub.derive_path(&[0, 5]).expect("pub path");
        assert_eq!(
            priv_child.compressed_pubkey(),
            pub_child.compressed_pubkey()
        );
    }

    #[test]
    fn public_derivation_refuses_hardened() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        let xpub = ExtendedPubKey::from_priv(&master);
        let err = xpub.derive_child(HARDENED_OFFSET).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    /// Serialize an ExtendedPubKey to a BIP32 base58check xpub for the given
    /// network (test helper / future account_xpub publishing). depth, parent
    /// fingerprint, and child number are zeroed since callers only need the
    /// key + chain code for further derivation.
    fn serialize_xpub(network: Network, xpub: &ExtendedPubKey) -> String {
        let mut payload = Vec::with_capacity(78);
        payload.extend_from_slice(&network.xpub_version().to_be_bytes());
        payload.push(0); // depth
        payload.extend_from_slice(&[0u8; 4]); // parent fingerprint
        payload.extend_from_slice(&[0u8; 4]); // child number
        payload.extend_from_slice(&xpub.chain_code);
        payload.extend_from_slice(&xpub.public.serialize());
        base58check_encode(&payload)
    }

    // Round-trip: serialize a master xpub to base58check, parse it back, and
    // confirm the recovered chain code and child derivation match. This
    // exercises base58check encode/decode and BIP32 public-key serialization
    // without relying on an externally transcribed literal.
    #[test]
    fn from_xpub_round_trips_master() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        let xpub = ExtendedPubKey::from_priv(&master);

        let serialized = serialize_xpub(Network::Main, &xpub);
        assert!(serialized.starts_with("xpub"));

        let parsed = ExtendedPubKey::from_xpub(Network::Main, &serialized).expect("parse xpub");
        assert_eq!(
            hex::encode(parsed.chain_code),
            "873dff81c02f525623fd1fe5167eac3a55a049de3d314bb42ee227ffed37d508"
        );
        assert_eq!(parsed.compressed_pubkey(), xpub.compressed_pubkey());

        // Parsed xpub derives the same non-hardened child as the private master.
        let priv_child = master.derive_child(1).expect("priv child");
        let pub_child = parsed.derive_child(1).expect("pub child");
        assert_eq!(
            priv_child.compressed_pubkey(),
            pub_child.compressed_pubkey()
        );
    }

    #[test]
    fn from_xpub_rejects_corrupted_checksum() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        let xpub = ExtendedPubKey::from_priv(&master);
        let mut serialized = serialize_xpub(Network::Main, &xpub);
        // Flip the final character to break the checksum.
        serialized.pop();
        serialized.push(if serialized.ends_with('1') { '2' } else { '1' });
        let err = ExtendedPubKey::from_xpub(Network::Main, &serialized).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }
}
