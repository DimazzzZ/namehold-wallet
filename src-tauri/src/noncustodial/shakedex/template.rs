//! One Shakedex price step is a signature by the lock key, type
//! `ANYONECANPAY|SINGLEREVERSE` (0x84), over a template whose input 0 is the
//! lock coin (sequence 0xfffffffe) and whose last output pays the seller.
//! 0x84 at input 0 commits to that input, the last output and the lock time
//! only, so a buyer may add inputs and other outputs freely.

use secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1, SecretKey};

use crate::error::AppError;
use crate::noncustodial::shakedex::script::lock_script;
use crate::noncustodial::tx::{Covenant, Input, Outpoint, Output, OutputAddress, Transaction};

pub const STEP_SEQUENCE: u32 = 0xffff_fffe;
pub const STEP_SIGHASH: u32 = 0x84;
const LOCKTIME_FLAG: u32 = 0x8000_0000;
const LOCKTIME_MASK: u32 = 0x7fff_ffff;
const GRANULARITY: u32 = 9;

pub fn encode_lock_time(secs: u64) -> Result<u32, AppError> {
    if secs >= 1u64 << 40 {
        return Err(AppError::InvalidInput(
            "lock time must fit in 40 bits".into(),
        ));
    }
    Ok(((secs >> GRANULARITY) as u32) | LOCKTIME_FLAG)
}

/// hsd `TX.isFinal` for a time lock: valid in the next block when
/// `(locktime & MASK) * 512 < MTP` — strictly less.
pub fn is_valid_at(encoded: u32, mtp: u64) -> bool {
    ((encoded & LOCKTIME_MASK) as u64) << GRANULARITY < mtp
}

/// The smallest MTP at which a step signed with `lock_time_secs` is valid
/// for the next block: [`is_valid_at`] accepts it from here on.
pub fn valid_from_mtp(lock_time_secs: u64) -> u64 {
    (lock_time_secs >> GRANULARITY << GRANULARITY) + 1
}

/// Seconds of MTP until a step signed with `lock_time_secs` is valid for the
/// next block — 0 when it already is at `mtp`.
pub fn secs_until_valid(lock_time_secs: u64, mtp: u64) -> u64 {
    valid_from_mtp(lock_time_secs).saturating_sub(mtp)
}

#[derive(Clone, Debug)]
pub struct StepTemplate<'a> {
    pub lock_outpoint: ([u8; 32], u32),
    pub lock_value: u64,
    pub lock_pubkey: &'a [u8; 33],
    pub payment: OutputAddress,
    pub price: u64,
    pub lock_time_secs: u64,
}

fn template_tx(t: &StepTemplate) -> Result<Transaction, AppError> {
    let mut tx = Transaction::new();
    let mut input = Input::new(Outpoint {
        hash: t.lock_outpoint.0,
        index: t.lock_outpoint.1,
    });
    input.sequence = STEP_SEQUENCE;
    tx.inputs.push(input);
    tx.outputs.push(Output {
        value: t.price,
        address: t.payment.clone(),
        covenant: Covenant::default(),
    });
    tx.locktime = encode_lock_time(t.lock_time_secs)?;
    Ok(tx)
}

impl StepTemplate<'_> {
    /// Sign this step with the lock key's secret: `0x84` over input 0 of the
    /// template, the whole lock script as the script code, low-S (the
    /// secp256k1 crate normalizes on signing). Callers verify the result
    /// against the template (`sell::sign_step`).
    pub(super) fn sign(&self, secret: &SecretKey) -> Result<[u8; 65], AppError> {
        template_tx(self)?.sign_p2wsh_input(
            0,
            secret,
            &lock_script(self.lock_pubkey),
            self.lock_value,
            STEP_SIGHASH,
        )
    }
}

pub fn step_sighash(t: &StepTemplate) -> Result<[u8; 32], AppError> {
    template_tx(t)?.signature_hash(0, &lock_script(t.lock_pubkey), t.lock_value, STEP_SIGHASH)
}

/// The ECDSA part of a 65-byte price-step signature, refused unless it is
/// well-formed and low-S (R2) — hsd's own standardness rule.
pub fn low_s_signature(sig65: &[u8; 65]) -> Result<Signature, AppError> {
    let sig = Signature::from_compact(&sig65[..64])
        .map_err(|_| AppError::InvalidInput("price step signature is malformed".into()))?;
    let mut normalized = sig;
    normalized.normalize_s();
    if normalized != sig {
        return Err(AppError::InvalidInput(
            "price step signature is not low-S".into(),
        ));
    }
    Ok(sig)
}

pub fn verify_step_signature(t: &StepTemplate, sig65: &[u8; 65]) -> Result<(), AppError> {
    if sig65[64] as u32 != STEP_SIGHASH {
        return Err(AppError::InvalidInput(format!(
            "price step signed with sighash type {:#04x}, expected 0x84",
            sig65[64]
        )));
    }
    let secp = Secp256k1::verification_only();
    let sig = low_s_signature(sig65)?;
    let pubkey = PublicKey::from_slice(t.lock_pubkey)
        .map_err(|_| AppError::InvalidInput("listing public key is invalid".into()))?;
    let msg = Message::from_digest(step_sighash(t)?);
    secp.verify_ecdsa(msg, &sig, &pubkey)
        .map_err(|_| AppError::InvalidInput("price step signature does not verify".into()))
}

/// Index of the cheapest step valid for the next block, if any.
pub fn current_step_index(steps: &[(u64, u32)], mtp: u64) -> Option<usize> {
    steps
        .iter()
        .enumerate()
        .filter(|(_, (_, enc))| is_valid_at(*enc, mtp))
        .min_by_key(|(_, (price, _))| *price)
        .map(|(i, _)| i)
}

/// R12/R24: the step that next changes the price at `mtp`: of the steps not
/// valid yet, those cheaper than `current` (any, when no step is valid yet),
/// the one valid soonest, the cheapest of those valid from the same time (it
/// is the one [`current_step_index`] then names). The buyer's verdict and the
/// seller's My listings read it from here.
pub fn next_step_index(steps: &[(u64, u32)], mtp: u64, current: Option<usize>) -> Option<usize> {
    steps
        .iter()
        .enumerate()
        .filter(|(_, (price, enc))| {
            !is_valid_at(*enc, mtp) && current.is_none_or(|c| *price < steps[c].0)
        })
        .min_by_key(|(_, (price, enc))| (*enc, *price))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_at_mtp() {
        let enc = encode_lock_time(1024).unwrap(); // 1024 / 512 = 2
        assert_eq!(enc, 0x8000_0002);
        assert!(!is_valid_at(enc, 1024), "equal is not yet valid");
        assert!(is_valid_at(enc, 1025));
        let enc2 = encode_lock_time(1535).unwrap(); // still 2: valid up to 511 s early
        assert!(is_valid_at(enc2, 1025));
    }

    #[test]
    fn step_validity_countdown_follows_512_second_rounding() {
        // floor(1024/512)*512 = 1024 must be < MTP: valid from MTP 1025.
        assert_eq!(secs_until_valid(1024, 1000), 25);
        assert_eq!(secs_until_valid(1535, 1000), 25);
        assert_eq!(secs_until_valid(1024, 2000), 0);
    }

    #[test]
    fn valid_from_mtp_is_the_first_mtp_that_accepts_the_step() {
        for secs in [0, 1, 511, 512, 1024, 1535, 1_700_000_000] {
            let enc = encode_lock_time(secs).unwrap();
            let from = valid_from_mtp(secs);
            assert!(is_valid_at(enc, from), "{secs} at {from}");
            assert!(!is_valid_at(enc, from - 1), "{secs} at {}", from - 1);
        }
        assert_eq!(valid_from_mtp(1535), 1025);
    }

    #[test]
    fn sequence_is_fffffffe() {
        assert_eq!(STEP_SEQUENCE, 0xffff_fffe);
        assert_eq!(STEP_SIGHASH, 0x84);
    }

    #[test]
    fn current_step_is_cheapest_valid() {
        let steps = [
            (900, encode_lock_time(1_000).unwrap()),
            (800, encode_lock_time(5_000).unwrap()),
            (700, encode_lock_time(9_000).unwrap()),
        ];
        assert_eq!(current_step_index(&steps, 500), None);
        assert_eq!(current_step_index(&steps, 6_000), Some(1));
        assert_eq!(current_step_index(&steps, 99_999), Some(2));
    }

    /// The next step is the soonest step not valid yet that is cheaper than
    /// the current one; with no current step, the soonest of all.
    #[test]
    fn next_step_is_the_soonest_cheaper_step() {
        let at = |secs: u64| encode_lock_time(secs).unwrap();
        // 10 HNS now, 12 first (dearer: never next while 10 is current),
        // then 9, then 8.
        let steps = [
            (10, at(0)),
            (9, at(1_000_960)),
            (8, at(1_001_472)),
            (12, at(1_000_448)),
        ];
        let mtp = 1_000_000;
        assert_eq!(current_step_index(&steps, mtp), Some(0));
        assert_eq!(next_step_index(&steps, mtp, Some(0)), Some(1));
        assert_eq!(next_step_index(&steps, mtp, None), Some(3));
        assert_eq!(next_step_index(&steps, 2_000_000, Some(2)), None);
    }

    /// Of cheaper steps valid from the same median time, the cheapest is the
    /// one that becomes current (`current_step_index`), whatever their order.
    #[test]
    fn next_step_of_steps_valid_together_is_the_cheapest() {
        let at = |secs: u64| encode_lock_time(secs).unwrap();
        let mtp = 1_000_000;
        let dearer_first = [(10, at(0)), (9, at(1_000_448)), (8, at(1_000_448))];
        assert_eq!(next_step_index(&dearer_first, mtp, Some(0)), Some(2));
        assert_eq!(current_step_index(&dearer_first, 1_000_449), Some(2));
        let cheaper_first = [(10, at(0)), (8, at(1_000_448)), (9, at(1_000_448))];
        assert_eq!(next_step_index(&cheaper_first, mtp, Some(0)), Some(1));
    }

    #[test]
    fn lock_time_over_40_bits_refused() {
        assert!(encode_lock_time(1u64 << 40).is_err());
    }

    fn template_for<'a>(pubkey: &'a [u8; 33], lock_time_secs: u64) -> StepTemplate<'a> {
        StepTemplate {
            lock_outpoint: ([4u8; 32], 0),
            lock_value: 1_000_000,
            lock_pubkey: pubkey,
            payment: OutputAddress {
                version: 0,
                hash: vec![9; 20],
            },
            price: 5_000_000,
            lock_time_secs,
        }
    }

    fn signed_by(secret: &SecretKey, t: &StepTemplate) -> [u8; 65] {
        t.sign(secret).unwrap()
    }

    /// A step whose lock time has no encoding is neither signed nor hashed
    /// nor verified: each says so rather than signing a truncated time.
    #[test]
    fn unencodable_lock_time_is_never_signed_or_verified() {
        let secret = SecretKey::from_byte_array([7u8; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &secret).serialize();
        let good = template_for(&pubkey, 1_700_000_000);
        let sig = signed_by(&secret, &good);
        assert!(verify_step_signature(&good, &sig).is_ok());
        let bad = template_for(&pubkey, 1u64 << 40);
        for e in [
            bad.sign(&secret).unwrap_err(),
            step_sighash(&bad).unwrap_err(),
            verify_step_signature(&bad, &sig).unwrap_err(),
        ] {
            assert!(e.to_string().contains("40 bits"), "{e}");
        }
    }

    /// Verification refuses, each with its reason: an `r` or `s` past the
    /// group order (malformed), a high-S signature, and a lock key that is
    /// not a curve point.
    #[test]
    fn verify_refuses_malformed_high_s_and_bad_key() {
        let secret = SecretKey::from_byte_array([7u8; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &secret).serialize();
        let t = template_for(&pubkey, 1_700_000_000);
        let reason = |t: &StepTemplate, sig: &[u8; 65]| {
            verify_step_signature(t, sig).unwrap_err().to_string()
        };
        let mut malformed = [0xffu8; 65];
        malformed[64] = 0x84;
        assert!(reason(&t, &malformed).contains("malformed"));
        let low = signed_by(&secret, &t);
        let mut sig = Signature::from_compact(&low[..64]).unwrap();
        let mut high = [0u8; 65];
        // `normalize_s` only ever lowers: negate by hand via the compact form.
        let compact = sig.serialize_compact();
        let n = secp256k1::constants::CURVE_ORDER;
        let mut s = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let d = i16::from(n[i]) - i16::from(compact[32 + i]) - borrow;
            borrow = i16::from(d < 0);
            s[i] = (d + 256 * borrow) as u8;
        }
        high[..32].copy_from_slice(&compact[..32]);
        high[32..64].copy_from_slice(&s);
        high[64] = 0x84;
        sig.normalize_s();
        assert_eq!(
            Signature::from_compact(&high[..64]).map(|mut h| {
                h.normalize_s();
                h
            }),
            Ok(sig),
            "the same signature, high-S"
        );
        assert!(reason(&t, &high).contains("not low-S"));
        let not_a_point = [0u8; 33];
        let t = template_for(&not_a_point, 1_700_000_000);
        assert!(reason(&t, &low).contains("public key is invalid"));
    }
}
