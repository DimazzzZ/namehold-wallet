//! The signature LearnHNS Market asks for to renew a listing's expiry
//! (`docs/listing-renewal.md` v1 in learnhns-market). It is made with the
//! listing's lock key, in a command the user starts with the wallet unlocked:
//! the market client and the background jobs never sign.

use crate::market::learnhns::RenewalRequest;
use crate::noncustodial::network::Network;

/// The exact text a renewal signs: LF-separated, no trailing newline.
pub fn renewal_message(network: Network, r: &RenewalRequest<'_>) -> String {
    format!(
        "LearnHNS Market listing renewal v1\n{}\n{}\n{}\n{}\n{}\n{}",
        network.as_str(),
        r.name.trim_end_matches('/').to_ascii_lowercase(),
        r.lock_txid.to_ascii_lowercase(),
        r.lock_vout,
        r.previous_expires_at,
        r.expires_at,
    )
}

/// The renewal signature: secp256k1 ECDSA over SHA-256 of `message`, compact
/// `r || s`, low-S, as 128 lowercase hex characters. Not the Shakedex sighash:
/// the message's first line keeps the two kinds apart, though one key makes both.
pub fn sign_renewal(secret: &secp256k1::SecretKey, message: &str) -> String {
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(message.as_bytes()).into();
    let sig = secp256k1::SECP256K1.sign_ecdsa(secp256k1::Message::from_digest(digest), secret);
    hex::encode(sig.serialize_compact())
}
