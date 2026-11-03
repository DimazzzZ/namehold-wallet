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

// Module doc quotes hsd source with deep prose indentation; clippy misreads the
// continuation lines as over-indented markdown list items. Keep the citations
// readable and silence the lint.
#![allow(clippy::doc_overindented_list_items)]

use crate::error::AppError;
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
    let sig = secp.sign_ecdsa(msg, secret);
    let compact = sig.serialize_compact(); // 64 bytes, low-S normalized.
    BASE64.encode(compact)
}

/// The longest message the wallet signs. A domain claim is one sentence.
pub const MAX_SIGNED_MESSAGE_BYTES: usize = 4096;

/// The longest run of whitespace in a signed message, and the most line
/// breaks in such a run (one blank line).
const MAX_WHITESPACE_RUN: usize = 16;
const MAX_LINE_BREAKS_IN_A_RUN: usize = 2;

/// Characters that show nothing, or change how the text around them shows:
/// control characters other than a line break or tab, format characters
/// (Unicode Cf: bidi overrides, zero-width, tags) and the other default-ignorable
/// or blank-looking code points. Spelled out because std has no Cf table.
fn is_hidden_char(c: char) -> bool {
    if c == '\n' || c == '\t' {
        return false;
    }
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{2800}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

/// Refuse a message the secure window could not show as it will be signed:
/// too long to read, with an invisible character, or padded with enough blank
/// space to push the rest of the text out of view. The renderer picks the
/// text, so this is checked before the user is asked anything. Look-alike
/// letters are not caught: the window shows them as they are.
pub fn check_message_can_be_shown(text: &str) -> Result<(), AppError> {
    if text.len() > MAX_SIGNED_MESSAGE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "this message is {} bytes long; the wallet signs at most {MAX_SIGNED_MESSAGE_BYTES}",
            text.len()
        )));
    }
    if let Some(c) = text.chars().find(|c| is_hidden_char(*c)) {
        return Err(AppError::InvalidInput(format!(
            "this message contains an invisible character (U+{:04X}), so the secure window could \
             not show exactly what is signed",
            c as u32
        )));
    }
    let (mut run, mut breaks) = (0, 0);
    for c in text.chars() {
        (run, breaks) = if c.is_whitespace() {
            (run + 1, breaks + usize::from(c == '\n'))
        } else {
            (0, 0)
        };
        if run > MAX_WHITESPACE_RUN || breaks > MAX_LINE_BREAKS_IN_A_RUN {
            return Err(AppError::InvalidInput(
                "this message has a long run of blank space, which could hide text from the \
                 secure window"
                    .to_string(),
            ));
        }
    }
    Ok(())
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

    /// What a claim message looks like passes: plain text, other scripts, a
    /// single blank line, tabs, and the longest length allowed.
    #[test]
    fn a_readable_message_can_be_shown() {
        for ok in [
            "Namebase registry: I verify ownership of \"ecology\" for account #20544.",
            "Я владелец этого имени",
            "first line\nsecond line\n\nthird\tline",
            "padded with sixteen   spaces: x                x",
        ] {
            check_message_can_be_shown(ok).unwrap_or_else(|e| panic!("{ok:?}: {e:?}"));
        }
        check_message_can_be_shown(&"x".repeat(MAX_SIGNED_MESSAGE_BYTES)).unwrap();
    }

    #[test]
    fn a_message_over_the_limit_is_refused() {
        let err = check_message_can_be_shown(&"x".repeat(MAX_SIGNED_MESSAGE_BYTES + 1));
        assert!(matches!(err, Err(AppError::InvalidInput(_))), "{err:?}");
    }

    /// Each of these shows nothing or changes how its neighbours show.
    #[test]
    fn a_message_with_an_invisible_character_is_refused() {
        for c in [
            '\u{0}',
            '\r',
            '\u{1B}',
            '\u{7F}',
            '\u{85}',
            '\u{AD}',
            '\u{61C}',
            '\u{200B}',
            '\u{200D}',
            '\u{200E}',
            '\u{2028}',
            '\u{2029}',
            '\u{202E}',
            '\u{2066}',
            '\u{2069}',
            '\u{2060}',
            '\u{2800}',
            '\u{3164}',
            '\u{FE0F}',
            '\u{FEFF}',
            '\u{E0001}',
            '\u{E0041}',
        ] {
            let err = check_message_can_be_shown(&format!("a{c}b"));
            assert!(
                matches!(err, Err(AppError::InvalidInput(_))),
                "U+{:04X}: {err:?}",
                c as u32
            );
        }
    }

    /// A run of blank space, in any form that wraps or breaks, is refused at
    /// the first character past the limit.
    #[test]
    fn a_message_padded_with_blank_space_is_refused() {
        let refused = |text: String| {
            let err = check_message_can_be_shown(&text);
            assert!(matches!(err, Err(AppError::InvalidInput(_))), "{err:?}");
        };
        refused(format!("a{}b", "\n".repeat(3)));
        refused(format!("a{}b", "\n \t".repeat(3)));
        refused(format!("a{}b", " ".repeat(17)));
        refused(format!("a{}b", "\u{A0}".repeat(17)));
        refused(format!("a{}b", "\u{3000}".repeat(17)));
        refused(format!("{}b", "\n".repeat(3)));
        // Short runs do not add up across the text between them.
        check_message_can_be_shown(&format!("{}x", "a\n\nb".repeat(50))).unwrap();
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
        assert_eq!(
            sig_bytes.len(),
            64,
            "hsd compact signatures are 64 bytes, non-recoverable"
        );

        let sig = secp256k1::ecdsa::Signature::from_compact(&sig_bytes).expect("valid compact sig");
        let hash = blake2b256(&message_preimage(text));
        let msg = Message::from_digest(hash);
        secp.verify_ecdsa(msg, &sig, &pubkey)
            .expect("signature verifies against derived pubkey");
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
