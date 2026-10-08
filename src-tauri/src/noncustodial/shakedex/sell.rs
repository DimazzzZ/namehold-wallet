//! The seller's side of a Shakedex listing: price steps signed by the lock
//! key (R17), the lock's self-check before a name enters it (R18), the Buy
//! Now lock time (R19) and the FINALIZE into the lock.

use crate::error::AppError;
use crate::noncustodial::actions::PlanResult;
use crate::noncustodial::covenants;
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::SpendableCoin;
use crate::noncustodial::shakedex::funding::{cov_out, fund, own_input};
use crate::noncustodial::shakedex::lock_key::LockKey;
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::template::{verify_step_signature, StepTemplate};
use crate::noncustodial::tx::{Covenant, OutputAddress};

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
    sign_step(key, &probe).map_err(|e| fail(&format!("test signature: {e}")))?;
    Ok(())
}

/// R19: a Buy Now's lock time, taken from the MTP of the tip when the step is
/// signed: one lock-time unit back, so its encoded value is below that MTP
/// and the step is valid in the next block (`template::is_valid_at`).
pub fn buy_now_lock_time(mtp: u64) -> u64 {
    mtp.saturating_sub(LOCK_TIME_UNIT_SECS)
}

/// `wallet_tx_drafts.action` of the day-0 TRANSFER committing our name to its
/// lock address.
pub const LOCK_ACTION: &str = "shakedex_lock";

/// R19, day 0: the TRANSFER covenant committing `name` (registered at
/// `name_height`) to the lock of `lock_pubkey`: version 0, and the program
/// SHA3-256 of the lock script.
pub fn lock_transfer_covenant(
    name: &str,
    name_height: u32,
    lock_pubkey: &[u8; 33],
) -> Result<Covenant, AppError> {
    let nh = names::hash_name(name)?;
    Ok(covenants::transfer(
        &nh,
        name_height,
        0,
        &script::lock_program(lock_pubkey),
    ))
}

/// What locking costs (R27), shown with the lock draft. The lock script lets
/// the name out only by a TRANSFER signed with the lock key or a FINALIZE, and
/// hsd renews a name on every FINALIZE.
pub const LOCK_COSTS: &str = "Once the name is finalized into the lock, its DNS records \
     cannot be changed and it cannot be renewed; the listing lasts at most one renewal \
     window from that finalize, which renews the name.";

/// R31's warning when the name expires less than six months after the tip.
pub fn near_expiry_warning(blocks_left: i64) -> String {
    format!(
        "The name expires in {blocks_left} blocks: do Finalize & sign before then. The \
         FINALIZE into the lock renews the name, so this expiry does not cut the listing short."
    )
}

/// The summary a lock TRANSFER draft stores. It reads as a plain `TxSummary`
/// (the secure window's generic rows, `warnings` as Warning rows), and its
/// `name` ties the draft to the name (`pending_broadcast_actions_for_name`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockSummary {
    pub action: String,
    pub name: String,
    pub send_total_doos: i64,
    pub fee_doos: i64,
    pub change_doos: i64,
    pub input_total_doos: i64,
    pub num_inputs: i64,
    /// The lock address the TRANSFER commits the name to.
    pub recipient_address: Option<String>,
    pub txid: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// `wallet_tx_drafts.action` of a draft finalizing our name into its lock.
pub const LOCK_FINALIZE_ACTION: &str = "shakedex_lock_finalize";

/// What the caller supplies from the name state to build the FINALIZE into
/// the lock; the caller checks the TRANSFER commitment.
pub struct LockFinalizeInput<'a> {
    pub network: Network,
    pub account: u32,
    /// Our name's TRANSFER coin, at our own address, whose covenant commits
    /// to the lock (checked by the caller against the re-derived key, R18).
    pub transfer: &'a SpendableCoin,
    pub lock_pubkey: [u8; 33],
    pub name: &'a str,
    pub name_height: u32,
    pub weak: bool,
    pub claimed: u32,
    pub renewals: u32,
    pub renewal_block: [u8; 32],
    pub funding: &'a [SpendableCoin],
    pub change_address: &'a str,
    pub rate: u64,
    #[cfg(test)]
    pub fixed_fee: Option<u64>,
}

#[cfg(test)]
impl LockFinalizeInput<'_> {
    /// Pin the fee instead of sizing it, to reproduce a vector's fee.
    pub fn with_fixed_fee_for_tests(mut self, fee: u64) -> Self {
        self.fixed_fee = Some(fee);
        self
    }
}

/// FINALIZE our name's TRANSFER coin into the P2WSH lock address:
/// `[our TRANSFER coin, our funding...] -> [FINALIZE at the lock, change?]`.
/// The generic finalize pays only P2WPKH addresses; this one pays the lock.
pub fn build_lock_finalize_plan(i: &LockFinalizeInput) -> Result<PlanResult, AppError> {
    let nh = names::hash_name(i.name)?;
    let fin = covenants::finalize(
        &nh,
        i.name_height,
        i.name.as_bytes(),
        u8::from(i.weak),
        i.claimed,
        i.renewals,
        &i.renewal_block,
    );
    let lock = script::lock_address(i.network, &i.lock_pubkey)?;
    let before = vec![cov_out(i.transfer.value, lock, &fin)];
    #[cfg(test)]
    let fixed = i.fixed_fee;
    #[cfg(not(test))]
    let fixed = None;
    fund(
        i.network,
        i.account,
        0,
        own_input(i.transfer),
        before,
        vec![],
        i.funding,
        i.change_address,
        i.rate,
        fixed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::actions::{sign_plan, FINAL_SEQUENCE};
    use crate::noncustodial::hd::ExtendedPrivKey;
    use crate::noncustodial::send::SpendableCoin;
    use crate::noncustodial::session::SignerSession;
    use crate::noncustodial::shakedex::lock_key::derive_lock_key;
    use crate::noncustodial::shakedex::template::{
        encode_lock_time, is_valid_at, low_s_signature, verify_step_signature,
    };
    use crate::noncustodial::sync::COV_FINALIZE;
    use crate::noncustodial::tx::{sighash, Transaction};

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

    const CHANGE: &str = "hs1qdhtaj7ws7chd2z2tulrmakqww428myx08d6w3v";

    fn coin(txid_byte: u8, value: u64, child: u32) -> SpendableCoin {
        SpendableCoin {
            txid: hex::encode([txid_byte; 32]),
            vout: 0,
            value,
            branch: 0,
            child_index: child,
        }
    }

    /// Our name's TRANSFER coin is input 0, signed by us as P2WPKH (ALL,
    /// final sequence); output 0 is the FINALIZE at the lock address carrying
    /// the name's value; the fee is the signed vsize times the rate.
    #[test]
    fn lock_finalize_pays_the_lock_address_and_its_fee_on_vsize() {
        let k = key("dexreviews");
        let transfer = coin(0x31, 1_000_000, 7);
        let funding = [coin(1, 2_000_000, 8)];
        let res = build_lock_finalize_plan(&LockFinalizeInput {
            network: Network::Main,
            account: 0,
            transfer: &transfer,
            lock_pubkey: k.pubkey,
            name: "dexreviews",
            name_height: 120,
            weak: false,
            claimed: 0,
            renewals: 0,
            renewal_block: [0x77; 32],
            funding: &funding,
            change_address: CHANGE,
            rate: 7,
            fixed_fee: None,
        })
        .unwrap();
        let inp0 = &res.plan.inputs[0];
        assert_eq!(inp0.txid, transfer.txid);
        assert_eq!((inp0.branch, inp0.child_index), (0, 7));
        assert_eq!(inp0.sighash_type, sighash::ALL);
        assert_eq!(inp0.sequence, FINAL_SEQUENCE);
        assert!(inp0.foreign_witness_hex.is_none());
        assert_eq!(res.plan.locktime, 0);
        let out0 = &res.plan.outputs[0];
        assert_eq!(out0.covenant_type, COV_FINALIZE);
        assert_eq!(out0.address, k.address);
        assert_eq!(out0.value, 1_000_000);
        assert_eq!(res.plan.outputs[1].address, CHANGE);
        let master = ExtendedPrivKey::from_seed(&[7u8; 64]).unwrap();
        let mut session = SignerSession::unlock("p".into(), Network::Main, master, 60_000);
        let (hex, _) = sign_plan(&mut session, &res.plan).unwrap();
        let signed = Transaction::decode(&hex::decode(hex).unwrap()).unwrap();
        assert_eq!(res.fee, signed.vsize() * 7);
        let out: u64 = res.plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(res.input_total, out + res.fee);
    }
}
