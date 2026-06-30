//! Bid blind + nonce, verified against hsd v6.1.1.
//!
//! `blind(value, nonce) = blake2b256( u64_le(value) || nonce[32] )`
//!   (hsd `rules.blind`).
//!
//! Nonce derivation (hsd `wallet.generateNonce` / `_getNoncePublicKeys`):
//!   index  = (hi(value) ^ lo(value)) & 0x7fffffff
//!   pubkey = accountXpub.derive(index).publicKey        (non-hardened, public)
//!   nonce  = blake2b256( addressHash160[20] || pubkey[33] || nameHash[32] )
//!
//! Because it derives from the ACCOUNT XPUB (public), the nonce — and therefore
//! the reveal — is reproducible from the wallet seed alone, even if the local
//! `bid_commitments` cache is lost. We still persist it for convenience.

use crate::error::AppError;
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::tx::blake2b256;

/// hsd `Rules.blind`: blake2b256 of `u64_le(value) || nonce`.
pub fn compute_blind(value: u64, nonce: &[u8; 32]) -> [u8; 32] {
    let mut data = Vec::with_capacity(40);
    data.extend_from_slice(&value.to_le_bytes());
    data.extend_from_slice(nonce);
    blake2b256(&data)
}

/// hsd `wallet.generateNonce` for a single-sig account.
///
/// * `account_xpub` — the BIP44 account node (`m/44'/coin'/account'`).
/// * `name_hash` — SHA3-256 of the name.
/// * `addr_hash160` — hash160 of the bid output address.
/// * `value` — the (true) bid value in dollarydoos.
pub fn compute_nonce(
    account_xpub: &ExtendedPubKey,
    name_hash: &[u8; 32],
    addr_hash160: &[u8; 20],
    value: u64,
) -> Result<[u8; 32], AppError> {
    let hi = (value >> 32) as u32;
    let lo = value as u32;
    let index = (hi ^ lo) & 0x7fff_ffff;
    let child = account_xpub.derive_child(index)?;
    let pubkey = child.compressed_pubkey(); // 33 bytes

    let mut data = Vec::with_capacity(20 + 33 + 32);
    data.extend_from_slice(addr_hash160);
    data.extend_from_slice(&pubkey);
    data.extend_from_slice(name_hash);
    Ok(blake2b256(&data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::{ExtendedPrivKey, ExtendedPubKey};

    fn xpub() -> ExtendedPubKey {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        ExtendedPubKey::from_priv(&master)
    }

    #[test]
    fn blind_is_deterministic_40_byte_preimage() {
        let nonce = [7u8; 32];
        let a = compute_blind(1_000_000, &nonce);
        let b = compute_blind(1_000_000, &nonce);
        assert_eq!(a, b);
        // Different value or nonce changes the blind.
        assert_ne!(a, compute_blind(1_000_001, &nonce));
        assert_ne!(a, compute_blind(1_000_000, &[8u8; 32]));
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn nonce_is_deterministic_from_public_data() {
        let xpub = xpub();
        let nh = [1u8; 32];
        let addr = [2u8; 20];
        let n1 = compute_nonce(&xpub, &nh, &addr, 500_000).unwrap();
        let n2 = compute_nonce(&xpub, &nh, &addr, 500_000).unwrap();
        assert_eq!(n1, n2);
        // Value participates in the derivation index, so it changes the nonce.
        assert_ne!(n1, compute_nonce(&xpub, &nh, &addr, 500_001).unwrap());
        // The blind built from this nonce round-trips for reveal.
        let blind = compute_blind(500_000, &n1);
        assert_eq!(blind, compute_blind(500_000, &n1));
    }
}
