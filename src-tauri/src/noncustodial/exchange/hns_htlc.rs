//! The HNS lock of a trade (spec §3.1, §3.2): the shared script paid to as
//! P2WSH (program = SHA3-256 of the script, as hsd `Address.fromScript`), its
//! refund time as an hsd time lock, and the two spends.
//!
//! - **Claim** (the Buyer's): signed with `SIGHASH_ALL | SIGHASH_NOINPUT |
//!   SIGHASH_ANYONECANPAY` (`0xc1`). hsd `tx.signatureHash` then commits to
//!   the version, the witness script, the spent value, the input's default
//!   sequence, the outputs and the lock time, and not to the outpoint, so
//!   the claim is signed before the lock exists and fits any coin holding
//!   exactly this script and value. `common.isSignatureEncoding` strips
//!   NOINPUT and ANYONECANPAY before checking the base type, so `0xc1` is a
//!   valid encoding on consensus and policy.
//! - **Refund** (the Seller's): an ordinary `SIGHASH_ALL` spend of the known
//!   lock outpoint, with the transaction lock time set to the refund time and
//!   the input's sequence below final, as `OP_CHECKLOCKTIMEVERIFY` requires
//!   (hsd `tx.verifyLocktime`).
//!
//! hsd 8.0.0 has no replace-by-fee, so every HNS spend is signed once, at a
//! rate that will be mined (`fees::hns_presign_rate`).

use secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1, SecretKey};
use sha3::{Digest, Sha3_256};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::exchange::script::{self, HtlcParams};
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::{
    output_address_from_string, sighash, Covenant, Input, Outpoint, Output, OutputAddress,
    Transaction,
};

/// hsd `consensus.LOCKTIME_FLAG`: bit 31 marks a time lock.
pub const LOCKTIME_FLAG: u32 = 1 << 31;
/// hsd `consensus.LOCKTIME_MASK`.
pub const LOCKTIME_MASK: u32 = LOCKTIME_FLAG - 1;
/// hsd `consensus.LOCKTIME_GRANULARITY`: a time lock counts units of 2^9 s.
pub const LOCKTIME_GRANULARITY: u32 = 9;
/// hsd `consensus.LOCKTIME_MULT`.
pub const LOCKTIME_MULT: u64 = 1 << LOCKTIME_GRANULARITY;

/// The sighash type of the Buyer's presigned claim.
pub const CLAIM_SIGHASH: u32 = sighash::ALL | sighash::NOINPUT | sighash::ANYONECANPAY;
/// The sighash type of the Seller's refund.
pub const REFUND_SIGHASH: u32 = sighash::ALL;

/// An input sequence below final, so the transaction's lock time is enforced
/// (hsd `tx.verifyLocktime` refuses `0xffffffff`).
pub const REFUND_SEQUENCE: u32 = 0xffff_fffe;

/// The hsd time lock for "not before `unix_secs`": the flag plus
/// `ceil(unix_secs / 512)`. Rounded **up**, so a refund never opens before
/// the time the terms name. Refuses a time that does not fit 31 bits.
pub fn encode_time_lock(unix_secs: u64) -> Result<u32, AppError> {
    let units = unix_secs.div_ceil(LOCKTIME_MULT);
    if units == 0 || units > u64::from(LOCKTIME_MASK) {
        return Err(AppError::InvalidInput(format!(
            "{unix_secs} cannot be an HNS time lock"
        )));
    }
    Ok(LOCKTIME_FLAG | units as u32)
}

/// The earliest Unix time an hsd time lock names (`512 × units`), or `None`
/// for a height lock.
pub fn time_lock_secs(lock: u32) -> Option<u64> {
    (lock & LOCKTIME_FLAG != 0).then(|| u64::from(lock & LOCKTIME_MASK) * LOCKTIME_MULT)
}

/// True once a transaction with time lock `lock` can enter a block on top of
/// a tip whose median time past is `mtp`: hsd `tx.isFinal` asks
/// `locktime × MULT < time` with `time` the tip's median time
/// (`chain.verifyFinal`), strictly.
pub fn time_lock_open(lock: u32, mtp: u64) -> bool {
    time_lock_secs(lock).is_some_and(|t| t < mtp)
}

/// The P2WSH program of a script: SHA3-256 (hsd `Address.fromScript`).
pub fn program(script: &[u8]) -> [u8; 32] {
    Sha3_256::digest(script).into()
}

/// The lock's address on `network`.
pub fn lock_address(network: Network, params: &HtlcParams) -> Result<String, AppError> {
    address::encode_p2wsh(network, &program(&params.script()))
}

/// The lock's output: `value` to the P2WSH program, plain (NONE covenant).
pub fn lock_output(params: &HtlcParams, value: u64) -> Output {
    Output {
        value,
        address: OutputAddress {
            version: 0,
            hash: program(&params.script()).to_vec(),
        },
        covenant: Covenant::default(),
    }
}

/// True if `out` is exactly the lock of `params` worth exactly `value`, with
/// no covenant. A lock with another value never matches: the Buyer's claim is
/// signed for one value.
pub fn is_lock_output(out: &Output, params: &HtlcParams, value: u64) -> bool {
    out.value == value
        && out.address.version == 0
        && out.address.hash == program(&params.script())
        && out.covenant.covenant_type == 0
        && out.covenant.items.is_empty()
}

/// The witness sizes of a claim, for fee sizing before the signature exists.
fn dummy_claim_witness(script: &[u8]) -> Vec<Vec<u8>> {
    script::claim_witness(&[0u8; 65], &[0u8; 32], script)
}

fn dummy_refund_witness(script: &[u8]) -> Vec<Vec<u8>> {
    script::refund_witness(&[0u8; 65], script)
}

/// The vsize of a one-input, one-output spend of the lock to `to`.
fn spend_vsize(to: &OutputAddress, witness: Vec<Vec<u8>>, locktime: u32) -> u64 {
    let mut tx = Transaction::new();
    tx.locktime = locktime;
    let mut input = Input::new(Outpoint {
        hash: [0u8; 32],
        index: 0,
    });
    input.witness = witness;
    tx.inputs.push(input);
    tx.outputs.push(Output {
        value: 0,
        address: to.clone(),
        covenant: Covenant::default(),
    });
    tx.vsize()
}

/// The fee a claim of the lock pays at `rate` doos per vbyte.
pub fn claim_fee(
    network: Network,
    params: &HtlcParams,
    to: &str,
    rate: u64,
) -> Result<u64, AppError> {
    let to = output_address_from_string(network, to)?;
    spend_vsize(&to, dummy_claim_witness(&params.script()), 0)
        .checked_mul(rate)
        .ok_or_else(|| AppError::InvalidInput("fee rate overflows".into()))
}

/// The fee a refund of the lock pays at `rate` doos per vbyte.
pub fn refund_fee(
    network: Network,
    params: &HtlcParams,
    to: &str,
    rate: u64,
) -> Result<u64, AppError> {
    let to = output_address_from_string(network, to)?;
    spend_vsize(
        &to,
        dummy_refund_witness(&params.script()),
        params.refund_lock_time,
    )
    .checked_mul(rate)
    .ok_or_else(|| AppError::InvalidInput("fee rate overflows".into()))
}

/// The Buyer's presigned claim: everything needed to build the claim once
/// the lock's outpoint and the secret are known, and nothing secret.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignedClaim {
    /// The lock script (its parameters are in the terms).
    pub script_hex: String,
    /// The value of the lock the signature is for.
    pub lock_value: u64,
    /// Where the claim pays, a P2WPKH address of the Buyer's wallet.
    pub to_address: String,
    /// What it pays: `lock_value - fee`.
    pub to_value: u64,
    /// The 65-byte signature, sighash `0xc1`.
    pub signature_hex: String,
}

/// The unsigned claim transaction both the signer and the finisher build:
/// version 0, the lock (any outpoint) as the one input, one output, lock
/// time 0. Under NOINPUT the outpoint and sequence are not signed.
fn claim_tx(
    network: Network,
    c: &PresignedClaimParts<'_>,
    outpoint: Outpoint,
) -> Result<Transaction, AppError> {
    let mut tx = Transaction::new();
    tx.inputs.push(Input::new(outpoint));
    tx.outputs.push(Output {
        value: c.to_value,
        address: output_address_from_string(network, c.to_address)?,
        covenant: Covenant::default(),
    });
    Ok(tx)
}

struct PresignedClaimParts<'a> {
    to_address: &'a str,
    to_value: u64,
}

fn sign(key: &SecretKey, sighash: [u8; 32], hash_type: u32) -> [u8; 65] {
    let secp = Secp256k1::signing_only();
    let sig = secp.sign_ecdsa(Message::from_digest(sighash), key);
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig.serialize_compact());
    out[64] = (hash_type & 0xff) as u8;
    out
}

/// Sign the Buyer's claim of `params`'s lock worth `lock_value`, paying
/// `lock_value - fee` to `to_address`. `key` must be the lock's claim key.
pub fn sign_claim(
    network: Network,
    params: &HtlcParams,
    lock_value: u64,
    key: &SecretKey,
    to_address: &str,
    fee: u64,
) -> Result<PresignedClaim, AppError> {
    params.check()?;
    let secp = Secp256k1::signing_only();
    if PublicKey::from_secret_key(&secp, key).serialize() != params.claim_pubkey {
        return Err(AppError::InvalidInput(
            "the claim key does not belong to this lock".into(),
        ));
    }
    let to_value = lock_value
        .checked_sub(fee)
        .filter(|v| *v > 0)
        .ok_or_else(|| AppError::InvalidInput("the HNS claim's fee is the whole lock".into()))?;
    let parts = PresignedClaimParts {
        to_address,
        to_value,
    };
    let script = params.script();
    let tx = claim_tx(
        network,
        &parts,
        Outpoint {
            hash: [0u8; 32],
            index: 0,
        },
    )?;
    let sighash = tx.signature_hash(0, &script, lock_value, CLAIM_SIGHASH)?;
    let sig = sign(key, sighash, CLAIM_SIGHASH);
    Ok(PresignedClaim {
        script_hex: hex::encode(&script),
        lock_value,
        to_address: to_address.to_owned(),
        to_value,
        signature_hex: hex::encode(sig),
    })
}

fn decode_sig(hex_sig: &str) -> Result<[u8; 65], AppError> {
    let v =
        hex::decode(hex_sig).map_err(|_| AppError::InvalidInput("signature is not hex".into()))?;
    v.try_into()
        .map_err(|_| AppError::InvalidInput("a signature must be 65 bytes".into()))
}

/// Check a 65-byte hsd signature over input 0 of `tx` spending `script`
/// worth `value`: the sighash byte must be `hash_type`, S low, and the
/// signature valid for `pubkey`.
pub fn verify_input_signature(
    tx: &Transaction,
    script: &[u8],
    value: u64,
    sig: &[u8; 65],
    hash_type: u32,
    pubkey: &[u8; 33],
) -> bool {
    if u32::from(sig[64]) != hash_type {
        return false;
    }
    let Ok(sighash) = tx.signature_hash(0, script, value, hash_type) else {
        return false;
    };
    let Ok(s) = Signature::from_compact(&sig[..64]) else {
        return false;
    };
    let mut low = s;
    low.normalize_s();
    if low != s {
        return false; // high S: hsd refuses it (`isLowS`)
    }
    let Ok(pk) = PublicKey::from_slice(pubkey) else {
        return false;
    };
    Secp256k1::verification_only()
        .verify_ecdsa(Message::from_digest(sighash), &s, &pk)
        .is_ok()
}

impl PresignedClaim {
    /// The lock script, refusing one that is not `params`'s.
    fn script_for(&self, params: &HtlcParams) -> Result<Vec<u8>, AppError> {
        let script = hex::decode(&self.script_hex)
            .map_err(|_| AppError::InvalidInput("claim script is not hex".into()))?;
        if !params.matches(&script) {
            return Err(AppError::InvalidInput(
                "the presigned claim is for another lock".into(),
            ));
        }
        Ok(script)
    }

    /// True if the stored signature is a valid `0xc1` signature of the claim
    /// key over this claim. Checked when the claim is stored and before it
    /// is sent.
    pub fn verify(&self, network: Network, params: &HtlcParams) -> bool {
        let Ok(script) = self.script_for(params) else {
            return false;
        };
        let Ok(sig) = decode_sig(&self.signature_hex) else {
            return false;
        };
        let parts = PresignedClaimParts {
            to_address: &self.to_address,
            to_value: self.to_value,
        };
        let Ok(tx) = claim_tx(
            network,
            &parts,
            Outpoint {
                hash: [0u8; 32],
                index: 0,
            },
        ) else {
            return false;
        };
        verify_input_signature(
            &tx,
            &script,
            self.lock_value,
            &sig,
            CLAIM_SIGHASH,
            &params.claim_pubkey,
        )
    }

    /// The finished claim of the lock at `outpoint`, with `secret` in its
    /// witness. Refuses a secret that does not hash to the lock's `H`.
    pub fn finish(
        &self,
        network: Network,
        params: &HtlcParams,
        outpoint: Outpoint,
        secret: &[u8; 32],
    ) -> Result<Transaction, AppError> {
        use sha2::Sha256;
        let script = self.script_for(params)?;
        let h: [u8; 32] = Sha256::digest(secret).into();
        if h != params.hash {
            return Err(AppError::InvalidInput(
                "the secret does not open this lock".into(),
            ));
        }
        let sig = decode_sig(&self.signature_hex)?;
        let parts = PresignedClaimParts {
            to_address: &self.to_address,
            to_value: self.to_value,
        };
        let mut tx = claim_tx(network, &parts, outpoint)?;
        tx.inputs[0].witness = script::claim_witness(&sig, secret, &script);
        Ok(tx)
    }
}

/// The Seller's signed refund of the lock at `outpoint` worth `lock_value`,
/// paying `lock_value - fee` to `to_address` once the refund time is open.
pub fn sign_refund(
    network: Network,
    params: &HtlcParams,
    outpoint: Outpoint,
    lock_value: u64,
    key: &SecretKey,
    to_address: &str,
    fee: u64,
) -> Result<Transaction, AppError> {
    params.check()?;
    let secp = Secp256k1::signing_only();
    if PublicKey::from_secret_key(&secp, key).serialize() != params.refund_pubkey {
        return Err(AppError::InvalidInput(
            "the refund key does not belong to this lock".into(),
        ));
    }
    let to_value = lock_value
        .checked_sub(fee)
        .filter(|v| *v > 0)
        .ok_or_else(|| AppError::InvalidInput("the HNS refund's fee is the whole lock".into()))?;
    let script = params.script();
    let mut tx = Transaction::new();
    tx.locktime = params.refund_lock_time;
    let mut input = Input::new(outpoint);
    input.sequence = REFUND_SEQUENCE;
    tx.inputs.push(input);
    tx.outputs.push(Output {
        value: to_value,
        address: output_address_from_string(network, to_address)?,
        covenant: Covenant::default(),
    });
    let sighash = tx.signature_hash(0, &script, lock_value, REFUND_SIGHASH)?;
    let sig = sign(key, sighash, REFUND_SIGHASH);
    tx.inputs[0].witness = script::refund_witness(&sig, &script);
    Ok(tx)
}

/// The secret in input `index` of `tx` if that input claims `params`'s lock.
pub fn secret_from_spend(tx: &Transaction, index: usize, params: &HtlcParams) -> Option<[u8; 32]> {
    tx.inputs
        .get(index)
        .and_then(|i| script::secret_from_claim_witness(params, &i.witness))
}
