//! Offers and trade messages carried by the relay (spec §2, §3.3, R18).
//!
//! Each is signed by the sender's relay key: an ECDSA secp256k1 signature
//! (compact, low S) over SHA-256 of a domain tag and the canonical JSON of
//! the unsigned part. A relay key controls no coin; a forged or replayed
//! message can waste time but cannot move money, because every chain fact a
//! message claims is checked on chain and every amount and time against the
//! terms and the policy. What the signatures buy is that the relay cannot
//! speak for a party.

use secp256k1::{ecdsa::Signature, Message as Digest32, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::noncustodial::exchange::terms::Terms;

pub const MSG_VERSION: u8 = 1;

/// The largest message a Namehold reads (bytes of JSON).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;

const OFFER_TAG: &[u8] = b"namehold exchange offer v1\0";
const MESSAGE_TAG: &[u8] = b"namehold exchange message v1\0";

/// Which side of HNS the maker is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The maker sells HNS for BTC (the maker is the Seller).
    Sell,
    /// The maker buys HNS with BTC (the maker is the Buyer).
    Buy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Offer {
    pub version: u8,
    pub network: String,
    pub side: Side,
    pub price_msat_per_hns: u64,
    pub min_hns: u64,
    pub max_hns: u64,
    pub maker_relay_pubkey: String,
    pub created_at: u64,
    pub expires_at: u64,
    /// 32 hex, so two offers alike in every field have different ids.
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedOffer {
    pub offer: Offer,
    pub signature: String,
}

/// A taker's request: its amount and its half of the terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TakeRequest {
    pub hns_amount: u64,
    /// The taker's HNS and BTC keys (the Buyer's claim and refund keys, or
    /// the Seller's refund and claim keys).
    pub hns_pubkey: String,
    pub btc_pubkey: String,
    /// `H`, when the taker is the Buyer.
    pub hash: Option<String>,
    /// The start the taker proposes; the maker may answer with its own.
    pub start: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockAnnouncement {
    pub terms_id: String,
    pub txid: String,
    pub vout: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "camelCase")]
pub enum Body {
    Take(TakeRequest),
    Accept { terms: Box<Terms> },
    BtcLocked(LockAnnouncement),
    HnsLocked(LockAnnouncement),
    Decline { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Message {
    pub version: u8,
    pub network: String,
    pub offer_id: String,
    pub trade_id: String,
    pub from: String,
    pub to: String,
    pub sent_at: u64,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Envelope {
    pub message: Message,
    pub signature: String,
}

fn digest(tag: &[u8], body: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(tag);
    h.update(body);
    h.finalize().into()
}

fn sign_digest(key: &SecretKey, d: [u8; 32]) -> String {
    let sig = Secp256k1::signing_only().sign_ecdsa(Digest32::from_digest(d), key);
    hex::encode(sig.serialize_compact())
}

fn verify_digest(pubkey_hex: &str, d: [u8; 32], sig_hex: &str) -> bool {
    let (Ok(pk), Ok(sig)) = (hex::decode(pubkey_hex), hex::decode(sig_hex)) else {
        return false;
    };
    let (Ok(pk), Ok(sig)) = (PublicKey::from_slice(&pk), Signature::from_compact(&sig)) else {
        return false;
    };
    let mut low = sig;
    low.normalize_s();
    if low != sig {
        return false;
    }
    Secp256k1::verification_only()
        .verify_ecdsa(Digest32::from_digest(d), &sig, &pk)
        .is_ok()
}

pub fn relay_pubkey_hex(key: &SecretKey) -> String {
    hex::encode(PublicKey::from_secret_key(&Secp256k1::new(), key).serialize())
}

impl Offer {
    pub fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("offers always serialize")
    }

    /// SHA-256 of the canonical encoding, hex.
    pub fn id(&self) -> String {
        hex::encode(Sha256::digest(self.canonical()))
    }

    pub fn sign(self, key: &SecretKey) -> Result<SignedOffer, AppError> {
        if relay_pubkey_hex(key) != self.maker_relay_pubkey {
            return Err(AppError::InvalidInput("not the offer's relay key".into()));
        }
        let signature = sign_digest(key, digest(OFFER_TAG, &self.canonical()));
        Ok(SignedOffer {
            offer: self,
            signature,
        })
    }

    /// The offer's own rules, before anyone acts on it: version, amounts,
    /// times, the nonce.
    pub fn check_shape(&self, now: u64) -> Result<(), AppError> {
        let bad = |m: &str| Err(AppError::InvalidInput(format!("the offer is refused: {m}")));
        if self.version != MSG_VERSION {
            return bad("unknown version");
        }
        if crate::noncustodial::network::Network::from_str_opt(&self.network).is_none() {
            return bad("unknown network");
        }
        if self.price_msat_per_hns == 0 || self.min_hns == 0 || self.max_hns < self.min_hns {
            return bad("its price or amounts are empty");
        }
        if self.expires_at <= self.created_at || self.expires_at <= now {
            return bad("it has expired");
        }
        if self.nonce.len() != 32 || hex::decode(&self.nonce).is_err() {
            return bad("its nonce is malformed");
        }
        Ok(())
    }
}

impl SignedOffer {
    pub fn verify(&self) -> bool {
        verify_digest(
            &self.offer.maker_relay_pubkey,
            digest(OFFER_TAG, &self.offer.canonical()),
            &self.signature,
        )
    }

    pub fn parse(json: &str) -> Result<SignedOffer, AppError> {
        if json.len() > MAX_MESSAGE_BYTES {
            return Err(AppError::InvalidInput("the offer is too large".into()));
        }
        let o: SignedOffer = serde_json::from_str(json)
            .map_err(|e| AppError::InvalidInput(format!("bad offer: {e}")))?;
        if !o.verify() {
            return Err(AppError::InvalidInput(
                "the offer's signature is invalid".into(),
            ));
        }
        Ok(o)
    }
}

impl Message {
    pub fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("messages always serialize")
    }

    pub fn sign(self, key: &SecretKey) -> Result<Envelope, AppError> {
        if relay_pubkey_hex(key) != self.from {
            return Err(AppError::InvalidInput("not the sender's relay key".into()));
        }
        let signature = sign_digest(key, digest(MESSAGE_TAG, &self.canonical()));
        Ok(Envelope {
            message: self,
            signature,
        })
    }
}

impl Envelope {
    pub fn verify(&self) -> bool {
        verify_digest(
            &self.message.from,
            digest(MESSAGE_TAG, &self.message.canonical()),
            &self.signature,
        )
    }

    /// Parse and check a message addressed to `to` on `network`: size,
    /// version, addressee, signature. A message that fails is dropped.
    pub fn parse_for(json: &str, to: &str, network: &str) -> Result<Envelope, AppError> {
        if json.len() > MAX_MESSAGE_BYTES {
            return Err(AppError::InvalidInput("the message is too large".into()));
        }
        let e: Envelope = serde_json::from_str(json)
            .map_err(|e| AppError::InvalidInput(format!("bad message: {e}")))?;
        if e.message.version != MSG_VERSION {
            return Err(AppError::InvalidInput("unknown message version".into()));
        }
        if e.message.to != to {
            return Err(AppError::InvalidInput(
                "the message is for someone else".into(),
            ));
        }
        if e.message.network != network {
            return Err(AppError::InvalidInput(
                "the message is for another network".into(),
            ));
        }
        if !e.verify() {
            return Err(AppError::InvalidInput(
                "the message's signature is invalid".into(),
            ));
        }
        Ok(e)
    }
}

/// A fresh random relay key.
pub fn new_relay_key() -> SecretKey {
    let mut bytes = [0u8; 32];
    loop {
        rand::Rng::fill(&mut rand::rng(), &mut bytes);
        if let Ok(k) = SecretKey::from_byte_array(bytes) {
            return k;
        }
    }
}

/// 16 random bytes as hex (trade ids, nonces).
pub fn random_hex16() -> String {
    let mut bytes = [0u8; 16];
    rand::Rng::fill(&mut rand::rng(), &mut bytes);
    hex::encode(bytes)
}
