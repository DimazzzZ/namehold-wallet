//! A NIP-01 event: its id, its BIP-340 signature, and the check of both.
//!
//! The id is SHA-256 of `[0,pubkey,created_at,kind,tags,content]` written
//! with NIP-01's own escaping (only `\n " \ \r \t \b \f` are escaped; every
//! other character goes in verbatim), which is not what `serde_json` writes
//! for other control characters, so the serialisation is done here.

use secp256k1::{schnorr, Keypair, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The string whose SHA-256 is the event id (NIP-01).
pub fn serialize_for_id(
    pubkey: &str,
    created_at: u64,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> String {
    let mut out = String::with_capacity(content.len() + 128);
    out.push_str("[0,");
    push_str(&mut out, pubkey);
    out.push_str(&format!(",{created_at},{kind},["));
    for (i, tag) in tags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        for (j, v) in tag.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            push_str(&mut out, v);
        }
        out.push(']');
    }
    out.push_str("],");
    push_str(&mut out, content);
    out.push(']');
    out
}

pub fn id_of(
    pubkey: &str,
    created_at: u64,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> [u8; 32] {
    Sha256::digest(serialize_for_id(pubkey, created_at, kind, tags, content).as_bytes()).into()
}

/// The x coordinate (Nostr's public key) of a compressed key in hex, or
/// `None` when `compressed` is not a valid compressed secp256k1 key.
pub fn x_only_of(compressed: &str) -> Option<String> {
    let bytes = hex::decode(compressed).ok()?;
    let key = secp256k1::PublicKey::from_slice(&bytes).ok()?;
    if bytes.len() != 33 {
        return None;
    }
    Some(hex::encode(key.x_only_public_key().0.serialize()))
}

/// The event `key` signs (BIP-340, fresh auxiliary randomness).
pub fn sign(
    key: &SecretKey,
    created_at: u64,
    kind: u32,
    tags: Vec<Vec<String>>,
    content: String,
) -> Event {
    let secp = Secp256k1::new();
    let pair = Keypair::from_secret_key(&secp, key);
    let pubkey = hex::encode(pair.x_only_public_key().0.serialize());
    let id = id_of(&pubkey, created_at, kind, &tags, &content);
    let sig = secp.sign_schnorr(&id, &pair);
    Event {
        id: hex::encode(id),
        pubkey,
        created_at,
        kind,
        tags,
        content,
        sig: hex::encode(sig.to_byte_array()),
    }
}

impl Event {
    /// The id recomputes from the fields and the signature is the author's.
    pub fn verify(&self) -> bool {
        let id = id_of(
            &self.pubkey,
            self.created_at,
            self.kind,
            &self.tags,
            &self.content,
        );
        if hex::encode(id) != self.id {
            return false;
        }
        let (Ok(pk), Ok(sig)) = (hex::decode(&self.pubkey), hex::decode(&self.sig)) else {
            return false;
        };
        let (Ok(pk), Ok(sig)) = (
            <[u8; 32]>::try_from(pk.as_slice()),
            <[u8; 64]>::try_from(sig.as_slice()),
        ) else {
            return false;
        };
        let Ok(pk) = XOnlyPublicKey::from_byte_array(pk) else {
            return false;
        };
        Secp256k1::verification_only()
            .verify_schnorr(&schnorr::Signature::from_byte_array(sig), &id, &pk)
            .is_ok()
    }

    /// The first value of the first tag named `name`.
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|t| t.first().map(String::as_str) == Some(name))
            .and_then(|t| t.get(1))
            .map(String::as_str)
    }
}
