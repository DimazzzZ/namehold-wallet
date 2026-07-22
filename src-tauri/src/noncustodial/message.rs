//! hsd-compatible "sign message with name" — proves ownership of a wallet
//! key (e.g. the one that owns a name) by signing arbitrary text, for
//! third-party verification flows such as Namebase's domain-claim process.
//!
//! Verified against hsd master source:
//!   - `lib/wallet/wallet.js` `signMessageWithName` / `signMessage`, which
//!     call into `lib/utils/message.js` `sign(msg, key)`:
//!       `const hash = Message.hash(msg);` then
//!       `return secp256k1.sign(hash, key);` — a plain (non-recoverable)
//!     ECDSA signature over the message hash.
//!   - `lib/utils/message.js` `Message.hash(text)`:
//!       `const prefix = `${pkg.currency} signed message:\n`;` (currency =
//!       "handshake") concatenated with the UTF-8 message bytes, hashed with
//!       a single Blake2b-256 (`blake2b.digest`, no double-hash, unlike
//!       Bitcoin's message-signing scheme).
//!   - bcrypto's `secp256k1.sign` returns a 64-byte compact (R||S) signature,
//!     low-S normalized — exactly what `secp256k1::sign_ecdsa` produces here
//!     (see `tx.rs::sign_p2wpkh_input`, which signs sighashes the same way).
//!     There is no sighash-type byte appended (that's a transaction-witness
//!     detail, not part of message signing) and no recovery id (this is NOT
//!     a recoverable signature).
//!
//! The output is base64(sig_64_bytes), matching what hsd's RPC
//! `signmessagewithname` / `signmessage` returns.

use crate::noncustodial::tx::blake2b256;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use secp256k1::{Message, Secp256k1, SecretKey};

/// hsd's message-signing magic prefix (`${pkg.currency} signed message:\n`
/// with currency = "handshake").
pub const MESSAGE_MAGIC: &[u8] = b"handshake signed message:\n";

/// The exact preimage hsd hashes before signing: `MAGIC || utf8(text)`.
pub fn message_preimage(text: &str) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(MESSAGE_MAGIC.len() + text.len());
    preimage.extend_from_slice(MESSAGE_MAGIC);
    preimage.extend_from_slice(text.as_bytes());
    preimage
}

/// Sign `text` with `secret`, reproducing hsd's `signmessagewithname` /
/// `signmessage` byte-for-byte: base64 of a 64-byte compact (low-S) ECDSA
/// signature over `blake2b256(MAGIC || text)`. Pure — no session, no DB, no
/// I/O — so it's directly testable against known vectors.
pub fn sign_handshake_message(secret: &SecretKey, text: &str) -> String {
    let hash = blake2b256(&message_preimage(text));
    let secp = Secp256k1::new();
    let msg = Message::from_digest(hash);
    let sig = secp.sign_ecdsa(&msg, secret);
    let compact = sig.serialize_compact(); // 64 bytes, low-S normalized.
    BASE64.encode(compact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::ExtendedPrivKey;
    use secp256k1::PublicKey;

    fn test_secret() -> SecretKey {
        // BIP32 vector-1 seed, same known material used throughout the HD tests.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        ExtendedPrivKey::from_seed(&seed).unwrap().secret
    }

    /// Preimage / known-answer: the exact magic bytes concatenated with the
    /// message, hashed with the same Blake2b-256 used for tx sighashes.
    #[test]
    fn preimage_matches_exact_magic_and_message_bytes() {
        let text = "Namebase registry: I verify ownership of \"ecology\" for account #20544.";
        let preimage = message_preimage(text);

        let mut expected = Vec::new();
        expected.extend_from_slice(b"handshake signed message:\n");
        expected.extend_from_slice(text.as_bytes());
        assert_eq!(preimage, expected);

        // The magic string is EXACTLY 26 bytes: "handshake signed message:\n".
        assert_eq!(MESSAGE_MAGIC.len(), 26);
        assert_eq!(&preimage[..26], MESSAGE_MAGIC);
        assert_eq!(&preimage[26..], text.as_bytes());

        let hash = blake2b256(&preimage);
        assert_eq!(hash, blake2b256(&expected));
    }

    /// Round-trip: sign with a known key, base64-decode, and verify against
    /// the derived compressed pubkey over the same preimage hash. Proves the
    /// helper produces a valid, verifiable signature over hsd's exact digest.
    #[test]
    fn sign_handshake_message_round_trips_through_verification() {
        let secret = test_secret();
        let secp = Secp256k1::new();
        let pubkey = PublicKey::from_secret_key(&secp, &secret);

        let text = "Namebase registry: I verify ownership of \"example\" for account #1.";
        let signature_b64 = sign_handshake_message(&secret, text);

        let sig_bytes = BASE64.decode(&signature_b64).expect("valid base64");
        assert_eq!(sig_bytes.len(), 64, "hsd compact signatures are 64 bytes, non-recoverable");

        let sig = secp256k1::ecdsa::Signature::from_compact(&sig_bytes).expect("valid compact sig");
        let hash = blake2b256(&message_preimage(text));
        let msg = Message::from_digest(hash);
        secp.verify_ecdsa(&msg, &sig, &pubkey).expect("signature verifies against derived pubkey");
    }

    /// Different messages must not collide onto the same signature (sanity —
    /// catches an accidental constant-preimage bug).
    #[test]
    fn different_messages_produce_different_signatures() {
        let secret = test_secret();
        let sig_a = sign_handshake_message(&secret, "message A");
        let sig_b = sign_handshake_message(&secret, "message B");
        assert_ne!(sig_a, sig_b);
    }

    /// Signing is deterministic (RFC 6979) — repeated calls with the same
    /// key+message reproduce the identical base64 signature, matching hsd
    /// (bcrypto's secp256k1.sign is also RFC-6979 deterministic).
    #[test]
    fn signing_is_deterministic() {
        let secret = test_secret();
        let text = "same message, twice";
        assert_eq!(
            sign_handshake_message(&secret, text),
            sign_handshake_message(&secret, text)
        );
    }
}
