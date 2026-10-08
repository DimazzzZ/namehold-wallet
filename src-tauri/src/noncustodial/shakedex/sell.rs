//! The seller's side of a Shakedex listing: price steps signed by the lock
//! key (R17), the lock's self-check before a name enters it (R18), the Buy
//! Now lock time (R19) and the FINALIZE into the lock.

use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::lock_key::LockKey;
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::template::{verify_step_signature, StepTemplate};
use crate::noncustodial::tx::OutputAddress;

/// One lock-time unit: hsd encodes a time lock in 512-second steps.
const LOCK_TIME_UNIT_SECS: u64 = 512;

/// Sign one price step with the lock key, refusing to return a signature
/// that does not verify against the template (a template for another lock,
/// or a key whose secret is not its public key's).
pub fn sign_step(key: &LockKey, t: &StepTemplate) -> Result<[u8; 65], AppError> {
    let sig = t.sign(&key.secret)?;
    verify_step_signature(t, &sig)?;
    Ok(sig)
}

/// R18, day 0: the lock key's script, program and address are the ones its
/// public key defines on `network`, and a test `0x84` signature over a dummy
/// template verifies. A name sent to a lock that fails this could never be
/// moved out again except by a FINALIZE to the same lock.
pub fn lock_self_check(key: &LockKey, network: Network) -> Result<(), AppError> {
    let fail = |what: &str| AppError::Other(format!("the lock key failed its self-check ({what})"));
    if key.script != script::lock_script(&key.pubkey) {
        return Err(fail("script"));
    }
    if key.program != script::lock_program(&key.pubkey) {
        return Err(fail("program"));
    }
    if key.address != script::lock_address(network, &key.pubkey)? {
        return Err(fail("address"));
    }
    let probe = StepTemplate {
        lock_outpoint: ([0; 32], 0),
        lock_value: 0,
        lock_pubkey: &key.pubkey,
        payment: OutputAddress {
            version: 0,
            hash: vec![0; 20],
        },
        price: 1,
        lock_time_secs: 0,
    };
    sign_step(key, &probe).map_err(|_| fail("test signature"))?;
    Ok(())
}

/// R19: a Buy Now's lock time, taken from the MTP of the tip when the step is
/// signed: one lock-time unit back, so its encoded value is below that MTP
/// and the step is valid in the next block (`template::is_valid_at`).
pub fn buy_now_lock_time(mtp: u64) -> u64 {
    mtp.saturating_sub(LOCK_TIME_UNIT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::ExtendedPrivKey;
    use crate::noncustodial::shakedex::lock_key::derive_lock_key;
    use crate::noncustodial::shakedex::template::{
        encode_lock_time, is_valid_at, low_s_signature, verify_step_signature,
    };

    fn key(name: &str) -> LockKey {
        let master = ExtendedPrivKey::from_seed(&[7u8; 64]).unwrap();
        derive_lock_key(&master, Network::Main, 0, name).unwrap()
    }

    fn template(pubkey: &[u8; 33], price: u64, lock_time_secs: u64) -> StepTemplate<'_> {
        StepTemplate {
            lock_outpoint: ([0x44; 32], 0),
            lock_value: 1_000_000,
            lock_pubkey: pubkey,
            payment: OutputAddress {
                version: 0,
                hash: vec![7; 20],
            },
            price,
            lock_time_secs,
        }
    }

    /// R1/R2: every step we sign carries 0x84, is low-S (hsd's standardness
    /// rule) and verifies; a template for another lock is refused rather
    /// than signed with this key.
    #[test]
    fn sign_step_is_0x84_low_s_and_verifies() {
        let k = key("dexreviews");
        for i in 0..32u64 {
            let t = template(&k.pubkey, 1_000_000 + i, 1_783_696_480 + i * 900);
            let sig = sign_step(&k, &t).unwrap();
            assert_eq!(sig[64], 0x84, "step {i}");
            low_s_signature(&sig).unwrap();
            verify_step_signature(&t, &sig).unwrap();
        }
        let other = key("namehold");
        assert!(sign_step(&k, &template(&other.pubkey, 1_000_000, 1_783_696_480)).is_err());
    }

    /// R18 day 0: each part of a lock key that disagrees with its public key
    /// fails the self-check, and so does a lock address for another network.
    #[test]
    fn lock_self_check_fails_on_a_wrong_key() {
        lock_self_check(&key("dexreviews"), Network::Main).unwrap();
        let other = key("namehold");

        let mut wrong_secret = key("dexreviews");
        wrong_secret.secret = other.secret;
        let mut wrong_script = key("dexreviews");
        wrong_script.script = other.script.clone();
        let mut wrong_program = key("dexreviews");
        wrong_program.program = other.program;
        let wrong_address = key("dexreviews");

        for (case, k, network) in [
            ("secret", wrong_secret, Network::Main),
            ("script", wrong_script, Network::Main),
            ("program", wrong_program, Network::Main),
            ("address", wrong_address, Network::Regtest),
        ] {
            let err = lock_self_check(&k, network).unwrap_err();
            assert!(
                matches!(&err, AppError::Other(m) if m.contains("self-check")),
                "{case}: {err:?}"
            );
        }
    }

    /// R19: a Buy Now signed at the tip's MTP is valid in the next block,
    /// including at an MTP that is a multiple of 512, where the MTP itself
    /// would not be (`is_valid_at` is strictly-less).
    #[test]
    fn buy_now_lock_time_is_valid_at_the_next_block() {
        let on_the_unit = 1_783_696_384; // 3_483_782 * 512
        assert!(!is_valid_at(
            encode_lock_time(on_the_unit).unwrap(),
            on_the_unit
        ));
        for mtp in [
            1,
            511,
            512,
            513,
            1024,
            on_the_unit,
            1_783_696_480,
            1_783_697_000,
        ] {
            let enc = encode_lock_time(buy_now_lock_time(mtp)).unwrap();
            assert!(is_valid_at(enc, mtp), "mtp {mtp}");
        }
        assert_eq!(buy_now_lock_time(1_783_696_480), 1_783_695_968);
        assert_eq!(buy_now_lock_time(100), 0, "never below zero");
    }
}
