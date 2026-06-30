//! Handshake address encoding.
//!
//! Verified against hsd `lib/primitives/address.js`:
//!   - P2WPKH: `fromHash(blake2b.digest(pubkey, 20), 0)` — version 0,
//!     20-byte Blake2b hash of the **compressed** public key.
//!   - Encoding: `bech32.encode(hrp, version, hash)` — BIP-173 Bech32
//!     (NOT Bech32m) for witness version 0.

use crate::error::AppError;
use crate::noncustodial::network::Network;
use bech32::segwit;
use bech32::Hrp;
use blake2::digest::consts::U20;
use blake2::Blake2b;
use blake2::Digest;

/// Blake2b with a 20-byte (160-bit) output, matching hsd's
/// `blake2b.digest(key, 20)`.
type Blake2b160 = Blake2b<U20>;

/// Compute the 20-byte Blake2b-160 hash of a compressed public key.
///
/// `pubkey` MUST be the 33-byte SEC1 compressed encoding, matching hsd which
/// always hashes compressed keys for P2WPKH.
pub fn pubkey_to_hash160(pubkey: &[u8]) -> [u8; 20] {
    let mut hasher = Blake2b160::new();
    hasher.update(pubkey);
    let out = hasher.finalize();
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&out);
    hash
}

/// Encode a P2WPKH address (witness v0, 20-byte program) for the given network.
pub fn encode_p2wpkh(network: Network, hash160: &[u8; 20]) -> Result<String, AppError> {
    let hrp = Hrp::parse(network.address_hrp())
        .map_err(|e| AppError::Crypto(format!("invalid hrp: {e}")))?;
    segwit::encode_v0(hrp, hash160)
        .map_err(|e| AppError::Crypto(format!("bech32 encode failed: {e}")))
}

/// Derive a Handshake P2WPKH address directly from a compressed public key.
pub fn address_from_pubkey(network: Network, compressed_pubkey: &[u8]) -> Result<String, AppError> {
    if compressed_pubkey.len() != 33 {
        return Err(AppError::Crypto(format!(
            "expected 33-byte compressed pubkey, got {}",
            compressed_pubkey.len()
        )));
    }
    let hash = pubkey_to_hash160(compressed_pubkey);
    encode_p2wpkh(network, &hash)
}

/// Build the P2WPKH scriptPubKey bytes for a 20-byte program.
///
/// On Handshake (as on Bitcoin segwit v0), a witness program is serialized in
/// a script as: `OP_<version> <push len> <program>`. For witness version 0 the
/// opcode is `0x00`, so a P2WPKH output script is `00 14 <hash160>` (22 bytes).
/// Verified against hsd `lib/script/script.js` `fromProgram(0, hash)`.
pub fn p2wpkh_script_pubkey(hash160: &[u8; 20]) -> Vec<u8> {
    let mut script = Vec::with_capacity(22);
    script.push(0x00); // OP_0 (witness version 0)
    script.push(0x14); // push 20 bytes
    script.extend_from_slice(hash160);
    script
}

/// Build the P2WPKH scriptPubKey directly from a compressed public key.
pub fn script_pubkey_from_pubkey(compressed_pubkey: &[u8]) -> Result<Vec<u8>, AppError> {
    if compressed_pubkey.len() != 33 {
        return Err(AppError::Crypto(format!(
            "expected 33-byte compressed pubkey, got {}",
            compressed_pubkey.len()
        )));
    }
    let hash = pubkey_to_hash160(compressed_pubkey);
    Ok(p2wpkh_script_pubkey(&hash))
}

/// Decode a Handshake address into (witness_version, program), validating the
/// HRP matches the expected network.
pub fn decode(network: Network, addr: &str) -> Result<(u8, Vec<u8>), AppError> {
    let (hrp, version, program) =
        segwit::decode(addr).map_err(|e| AppError::Crypto(format!("bech32 decode failed: {e}")))?;
    if hrp.as_str() != network.address_hrp() {
        return Err(AppError::InvalidInput(format!(
            "address HRP '{}' does not match network '{}'",
            hrp.as_str(),
            network.address_hrp()
        )));
    }
    Ok((version.to_u8(), program))
}

/// Validate that an address is well-formed and belongs to the given network.
pub fn is_valid(network: Network, addr: &str) -> bool {
    decode(network, addr).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // CANONICAL hsd vector from hsd/test/address-test.js:
    //   raw hash160 = 6d5571fdbca1019cd0f0cd792d1b0bdfa7651c7e
    //   address     = hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx
    // This proves our bech32 v0 encoding byte-for-byte matches hsd.
    #[test]
    fn matches_canonical_hsd_mainnet_address() {
        let raw = hex::decode("6d5571fdbca1019cd0f0cd792d1b0bdfa7651c7e").unwrap();
        let mut hash160 = [0u8; 20];
        hash160.copy_from_slice(&raw);
        let addr = encode_p2wpkh(Network::Main, &hash160).expect("encode");
        assert_eq!(addr, "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx");

        // And decoding it back yields the same program.
        let (ver, prog) = decode(Network::Main, &addr).expect("decode");
        assert_eq!(ver, 0);
        assert_eq!(prog, raw);
    }

    // Validate that the `blake2` crate produces the same Blake2b-160 digest
    // hsd uses (bcrypto blake2b with output length 20, no key, no personal).
    // Canonical Blake2b vector: blake2b-160 of the empty input.
    // (Reference: BLAKE2 official test vectors / RFC 7693 derived.)
    #[test]
    fn blake2b_160_empty_matches_reference() {
        let h = pubkey_to_hash160(&[]);
        // blake2b with 20-byte output of empty message.
        assert_eq!(hex::encode(h), "3345524abf6bbe1809449224b5972c41790b6cf2");
    }

    #[test]
    fn p2wpkh_roundtrip_is_consistent() {
        // 33-byte compressed pubkey (all-0x02 + 32 bytes) — structurally valid
        // for encoding purposes (not a real key).
        let mut pk = [0u8; 33];
        pk[0] = 0x02;
        for (i, b) in pk.iter_mut().enumerate().skip(1) {
            *b = i as u8;
        }
        let addr = address_from_pubkey(Network::Main, &pk).expect("encode");
        assert!(addr.starts_with("hs1"));
        let (ver, prog) = decode(Network::Main, &addr).expect("decode");
        assert_eq!(ver, 0);
        assert_eq!(prog.len(), 20);
        assert_eq!(prog, pubkey_to_hash160(&pk).to_vec());
    }

    #[test]
    fn p2wpkh_script_pubkey_is_standard_segwit_v0() {
        let raw = hex::decode("6d5571fdbca1019cd0f0cd792d1b0bdfa7651c7e").unwrap();
        let mut hash160 = [0u8; 20];
        hash160.copy_from_slice(&raw);
        let script = p2wpkh_script_pubkey(&hash160);
        // 00 14 <20-byte hash>
        assert_eq!(script.len(), 22);
        assert_eq!(script[0], 0x00);
        assert_eq!(script[1], 0x14);
        assert_eq!(&script[2..], &raw[..]);
        assert_eq!(
            hex::encode(&script),
            "00146d5571fdbca1019cd0f0cd792d1b0bdfa7651c7e"
        );
    }

    #[test]
    fn script_pubkey_from_pubkey_matches_hash() {
        let mut pk = [0u8; 33];
        pk[0] = 0x02;
        for (i, b) in pk.iter_mut().enumerate().skip(1) {
            *b = i as u8;
        }
        let script = script_pubkey_from_pubkey(&pk).expect("script");
        let hash = pubkey_to_hash160(&pk);
        assert_eq!(script, p2wpkh_script_pubkey(&hash));
    }

    #[test]
    fn wrong_network_hrp_rejected() {
        let mut pk = [0u8; 33];
        pk[0] = 0x03;
        let addr = address_from_pubkey(Network::Main, &pk).expect("encode");
        assert!(!is_valid(Network::Testnet, &addr));
        assert!(is_valid(Network::Main, &addr));
    }
}
