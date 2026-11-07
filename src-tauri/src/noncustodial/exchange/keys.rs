//! Swap keys and the Buyer's secret (spec §2, R14).
//!
//! `m/44'/<HNS coin type>'/<account>'/3'` is the swap branch; below it each
//! trade has an index `t` (non-hardened, below 2^31) and three children:
//!
//! - `t/0` — the HNS key: the Seller's HNS refund key or the Buyer's HNS
//!   claim key;
//! - `t/1` — the BTC key: the Seller's BTC claim key or the Buyer's BTC
//!   refund key;
//! - `t/2` — never a public key: its private key is the HMAC key of the
//!   Buyer's secret, `x = HMAC-SHA256(k, "namehold exchange secret v1" ‖ n)`
//!   with `n` a random 32-byte nonce kept with the trade.
//!
//! The nonce makes every secret unique even if a trade index is handed out
//! twice (a restore from the phrase into an empty database starts the
//! indexes again): a repeated secret would let the Seller of the later trade
//! open the BTC lock with the secret the earlier trade made public.
//!
//! The branch's extended public key is stored, so the daemon can give a
//! counterparty `t/0` and `t/1` without the phrase. Because `t/2`'s private
//! key is needed for `x`, no holder of the extended public key can compute a
//! secret; a restore from the phrase recomputes every key of a known trade
//! index, and its secret given the nonce.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::AppError;
use crate::noncustodial::hd::{ExtendedPrivKey, ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;

/// The branch below the account node: after receive (0), change (1) and
/// Shakedex lock keys (2').
pub const SWAP_BRANCH: u32 = 3;

pub const ROLE_HNS: u32 = 0;
pub const ROLE_BTC: u32 = 1;
const ROLE_SECRET: u32 = 2;

const SECRET_TAG: &[u8] = b"namehold exchange secret v1";

/// `[44', coin type', account', 3']`.
pub fn swap_branch_path(network: Network, account: u32) -> Result<[u32; 4], AppError> {
    if account >= HARDENED_OFFSET {
        return Err(AppError::InvalidInput(format!(
            "account {account} is out of range for swap keys"
        )));
    }
    Ok([
        HARDENED_OFFSET + 44,
        HARDENED_OFFSET + network.coin_type(),
        HARDENED_OFFSET + account,
        HARDENED_OFFSET + SWAP_BRANCH,
    ])
}

fn check_index(trade_index: u32) -> Result<(), AppError> {
    if trade_index >= HARDENED_OFFSET {
        return Err(AppError::InvalidInput(format!(
            "trade index {trade_index} is out of range"
        )));
    }
    Ok(())
}

/// The swap branch's extended public key, from the unlocked master key.
pub fn branch_xpub(
    master: &ExtendedPrivKey,
    network: Network,
    account: u32,
) -> Result<ExtendedPubKey, AppError> {
    Ok(ExtendedPubKey::from_priv(
        &master.derive_path(&swap_branch_path(network, account)?)?,
    ))
}

/// A trade's two public keys from the branch's extended public key.
pub fn trade_pubkeys(
    xpub: &ExtendedPubKey,
    trade_index: u32,
) -> Result<([u8; 33], [u8; 33]), AppError> {
    check_index(trade_index)?;
    let node = xpub.derive_child(trade_index)?;
    Ok((
        node.derive_child(ROLE_HNS)?.compressed_pubkey(),
        node.derive_child(ROLE_BTC)?.compressed_pubkey(),
    ))
}

/// A trade's private keys, from the unlocked master key. No `Debug`.
pub struct TradeKeys {
    pub hns: secp256k1::SecretKey,
    pub btc: secp256k1::SecretKey,
    pub hns_pubkey: [u8; 33],
    pub btc_pubkey: [u8; 33],
    secret_key: Zeroizing<[u8; 32]>,
}

impl TradeKeys {
    /// The Buyer's secret of this trade and `nonce`.
    pub fn secret(&self, nonce: &[u8; 32]) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret_key.as_ref())
            .expect("hmac takes any key length");
        mac.update(SECRET_TAG);
        mac.update(nonce);
        mac.finalize().into_bytes().into()
    }

    /// `SHA-256(secret)`.
    pub fn hash(&self, nonce: &[u8; 32]) -> [u8; 32] {
        Sha256::digest(self.secret(nonce)).into()
    }
}

/// A fresh nonce for a Buyer's secret.
pub fn new_secret_nonce() -> [u8; 32] {
    let mut n = [0u8; 32];
    rand::Rng::fill(&mut rand::rng(), &mut n);
    n
}

pub fn trade_keys(
    master: &ExtendedPrivKey,
    network: Network,
    account: u32,
    trade_index: u32,
) -> Result<TradeKeys, AppError> {
    check_index(trade_index)?;
    let mut path = swap_branch_path(network, account)?.to_vec();
    path.push(trade_index);
    let node = master.derive_path(&path)?;
    let hns = node.derive_child(ROLE_HNS)?;
    let btc = node.derive_child(ROLE_BTC)?;
    let sec = node.derive_child(ROLE_SECRET)?;
    Ok(TradeKeys {
        hns_pubkey: hns.compressed_pubkey(),
        btc_pubkey: btc.compressed_pubkey(),
        hns: hns.secret,
        btc: btc.secret,
        secret_key: Zeroizing::new(sec.secret.secret_bytes()),
    })
}
