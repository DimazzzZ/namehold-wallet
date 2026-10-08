//! Lock keys (R17, ADR 0004): one per name and account, at
//! `m/44'/<coin type>'/<account>'/2'/<i>'` with every level hardened, so no
//! account xpub can compute one. `i` is the first four bytes of the name hash,
//! big-endian, masked to 31 bits. A name has the same lock key in an account
//! on every device that holds the phrase. Two names whose hashes agree in bits
//! 1-31 of those bytes share a key, which is why listing state is keyed by
//! name and lock outpoint, never by lock address.

use secp256k1::SecretKey;

use crate::error::AppError;
use crate::noncustodial::hd::{ExtendedPrivKey, HARDENED_OFFSET};
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::script;

/// The lock branch below the account node, after receive (0) and change (1).
pub const LOCK_BRANCH: u32 = 2;

/// Keeps the index below 2^31 so that it can be hardened.
const INDEX_MASK: u32 = 0x7fff_ffff;

/// `u32_be(sha3_256(name)[0..4]) & 0x7fffffff`. Refuses a name that is not a
/// valid Handshake name: it can never be on chain, so its key locks nothing.
pub fn lock_key_index(name: &str) -> Result<u32, AppError> {
    let h = names::hash_name(name)?;
    Ok(u32::from_be_bytes([h[0], h[1], h[2], h[3]]) & INDEX_MASK)
}

/// `[44', coin_type', account', 2', index']`. An account at or above 2^31 is
/// refused rather than clamped: a lock key derived on another path than the
/// one the lock was made with could never move the name.
pub fn lock_key_path(coin_type: u32, account: u32, name: &str) -> Result<[u32; 5], AppError> {
    if account >= HARDENED_OFFSET {
        return Err(AppError::InvalidInput(format!(
            "account {account} is out of range for a lock key"
        )));
    }
    let index = lock_key_index(name)?;
    Ok([
        HARDENED_OFFSET + 44,
        HARDENED_OFFSET + coin_type,
        HARDENED_OFFSET + account,
        HARDENED_OFFSET + LOCK_BRANCH,
        HARDENED_OFFSET + index,
    ])
}

/// A name's lock key with the lock it defines. No `Debug`: it holds the
/// secret.
pub struct LockKey {
    pub secret: SecretKey,
    pub pubkey: [u8; 33],
    /// `lock_script(pubkey)`, the P2WSH witness script.
    pub script: Vec<u8>,
    /// SHA3-256 of `script`, the lock address's program.
    pub program: [u8; 32],
    pub address: String,
}

pub fn derive_lock_key(
    master: &ExtendedPrivKey,
    network: Network,
    account: u32,
    name: &str,
) -> Result<LockKey, AppError> {
    let path = lock_key_path(network.coin_type(), account, name)?;
    let child = master.derive_path(&path)?;
    let pubkey = child.compressed_pubkey();
    Ok(LockKey {
        secret: child.secret,
        pubkey,
        script: script::lock_script(&pubkey),
        program: script::lock_program(&pubkey),
        address: script::lock_address(network, &pubkey)?,
    })
}
