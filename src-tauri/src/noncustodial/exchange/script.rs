//! The hash-time-locked script both locks of a trade use (spec §3.1):
//!
//! ```text
//! OP_IF
//!   OP_SIZE 32 OP_EQUALVERIFY OP_SHA256 <H> OP_EQUALVERIFY <claim key>
//! OP_ELSE
//!   <refund time> OP_CHECKLOCKTIMEVERIFY OP_DROP <refund key>
//! OP_ENDIF
//! OP_CHECKSIG
//! ```
//!
//! The opcodes have the same values in hsd (`lib/script/common.js`) and in
//! Bitcoin, so one builder serves both chains; only the refund time's
//! encoding and the script's hash differ, and those live in `hns_htlc` and
//! `btc_htlc`. A script is recognised as a lock of given parameters only by
//! rebuilding it and comparing every byte ([`HtlcParams::matches`]): no
//! parser decides what a foreign script "means".

use crate::error::AppError;

pub const OP_0: u8 = 0x00;
pub const OP_1: u8 = 0x51;
pub const OP_IF: u8 = 0x63;
pub const OP_ELSE: u8 = 0x67;
pub const OP_ENDIF: u8 = 0x68;
pub const OP_DROP: u8 = 0x75;
pub const OP_SIZE: u8 = 0x82;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_SHA256: u8 = 0xa8;
pub const OP_CHECKSIG: u8 = 0xac;
pub const OP_CHECKLOCKTIMEVERIFY: u8 = 0xb1;

/// The only secret length either script accepts (`OP_SIZE 32`).
pub const SECRET_LEN: usize = 32;

/// What a lock script is made of. `refund_lock_time` is already in the
/// chain's own encoding (an hsd time lock on HNS, a Unix time on Bitcoin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtlcParams {
    pub hash: [u8; 32],
    pub claim_pubkey: [u8; 33],
    pub refund_pubkey: [u8; 33],
    pub refund_lock_time: u32,
}

/// The minimal `CScriptNum` encoding of a non-negative number, as both
/// interpreters require for `OP_CHECKLOCKTIMEVERIFY` under the minimal-data
/// rule: little-endian, no trailing zero byte unless the sign bit of the
/// last byte would otherwise be set.
pub fn script_num(n: u32) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(5);
    let mut v = n;
    while v > 0 {
        out.push((v & 0xff) as u8);
        v >>= 8;
    }
    if out.last().is_some_and(|b| b & 0x80 != 0) {
        out.push(0x00);
    }
    out
}

/// Push `data` with the shortest push opcode (both chains' minimal-push
/// rule). Lock-script pushes are 1..=75 bytes, so a direct push always fits.
fn push(script: &mut Vec<u8>, data: &[u8]) {
    debug_assert!(!data.is_empty() && data.len() <= 75);
    script.push(data.len() as u8);
    script.extend_from_slice(data);
}

impl HtlcParams {
    /// The lock script, byte for byte as in the module comment.
    pub fn script(&self) -> Vec<u8> {
        let mut s = Vec::with_capacity(110);
        s.push(OP_IF);
        s.push(OP_SIZE);
        push(&mut s, &[SECRET_LEN as u8]);
        s.push(OP_EQUALVERIFY);
        s.push(OP_SHA256);
        push(&mut s, &self.hash);
        s.push(OP_EQUALVERIFY);
        push(&mut s, &self.claim_pubkey);
        s.push(OP_ELSE);
        push(&mut s, &script_num(self.refund_lock_time));
        s.push(OP_CHECKLOCKTIMEVERIFY);
        s.push(OP_DROP);
        push(&mut s, &self.refund_pubkey);
        s.push(OP_ENDIF);
        s.push(OP_CHECKSIG);
        s
    }

    /// True only if `script` is exactly this lock's script.
    pub fn matches(&self, script: &[u8]) -> bool {
        script == self.script().as_slice()
    }

    /// Refuses parameters no lock may be made of: a refund time of 0 (CLTV
    /// would be satisfied at once), the same key on both paths, or a key
    /// that is not a compressed secp256k1 point.
    pub fn check(&self) -> Result<(), AppError> {
        if self.refund_lock_time == 0 {
            return Err(AppError::InvalidInput(
                "a lock's refund time cannot be zero".into(),
            ));
        }
        if self.claim_pubkey == self.refund_pubkey {
            return Err(AppError::InvalidInput(
                "a lock's claim key and refund key must differ".into(),
            ));
        }
        for k in [&self.claim_pubkey, &self.refund_pubkey] {
            secp256k1::PublicKey::from_slice(k).map_err(|_| {
                AppError::InvalidInput("a lock key is not a valid public key".into())
            })?;
        }
        Ok(())
    }
}

/// The witness that claims a lock: `[signature, secret, 0x01, script]`.
pub fn claim_witness(signature: &[u8], secret: &[u8; 32], script: &[u8]) -> Vec<Vec<u8>> {
    vec![
        signature.to_vec(),
        secret.to_vec(),
        vec![0x01],
        script.to_vec(),
    ]
}

/// The witness that refunds a lock: `[signature, <empty>, script]`. The empty
/// item selects OP_ELSE (minimal-IF requires exactly empty for false).
pub fn refund_witness(signature: &[u8], script: &[u8]) -> Vec<Vec<u8>> {
    vec![signature.to_vec(), Vec::new(), script.to_vec()]
}

/// The secret in a witness that spent `params`'s lock through its claim
/// path, if and only if the witness is a claim of exactly that script and
/// its secret hashes to `params.hash`. `None` for a refund, a different
/// script, or a secret that does not hash to `H`.
pub fn secret_from_claim_witness(params: &HtlcParams, witness: &[Vec<u8>]) -> Option<[u8; 32]> {
    use sha2::{Digest, Sha256};
    if witness.len() != 4 || witness[2] != [0x01] || !params.matches(&witness[3]) {
        return None;
    }
    let secret: [u8; 32] = witness[1].as_slice().try_into().ok()?;
    let h: [u8; 32] = Sha256::digest(secret).into();
    (h == params.hash).then_some(secret)
}
