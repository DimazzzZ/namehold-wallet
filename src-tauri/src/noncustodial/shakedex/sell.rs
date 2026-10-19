//! The seller's side of a Shakedex listing: price steps signed by the lock
//! key (R17), the lock's self-check before a name enters it (R18), the Buy
//! Now lock time (R19) and the FINALIZE into the lock.

use crate::error::AppError;
use crate::noncustodial::actions::{DraftPlan, PlanResult};
use crate::noncustodial::covenants;
use crate::noncustodial::names;
use crate::noncustodial::network::{NameParams, Network};
use crate::noncustodial::send::{SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::shakedex::funding::{cov_out, fund, own_input};
use crate::noncustodial::shakedex::lock_key::LockKey;
use crate::noncustodial::shakedex::purchase::MAX_MONEY;
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::template::{
    secs_until_valid, valid_from_mtp, verify_step_signature, StepTemplate,
};
use crate::noncustodial::sync::COV_FINALIZE;
use crate::noncustodial::tx::{Covenant, OutputAddress};
use crate::noncustodial::types::doos_to_hns_string;

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

/// R19: a Buy Now's lock time, from the MTP the node reports at Finalize &
/// sign (read just before its confirmation): one lock-time unit back, so its encoded value is below that MTP
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

/// R31's verdict on locking a name now, or on finalizing it into the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryNotice {
    Ok,
    /// Allowed, with R31's warning: the name expires less than 180 R9 days
    /// after the tip.
    Warn {
        blocks_left: i64,
    },
    /// The name expires at or before `tip + 1 + remaining lockup + day`: it
    /// would expire on a TRANSFER coin before its FINALIZE into the lock, or
    /// within a day of it.
    Refuse {
        expiry_end: i64,
    },
}

/// Six months, in R9 days (R31's warning).
const LOCK_WARN_DAYS: i64 = 180;

/// R31. `name_info` is hsd's `getnameinfo` reply; its renewal height and
/// claimed count are read field by field, and a reply without either is "could
/// not check" (fail closed). `remaining_lockup` is the full transfer lockup at
/// Lock and `NameParams::blocks_until_finalize` of the lock TRANSFER at
/// Finalize & sign.
pub fn lock_expiry_guard(
    params: &NameParams,
    name_info: &serde_json::Value,
    tip: i64,
    remaining_lockup: i64,
) -> Result<ExpiryNotice, AppError> {
    let info = match name_info.get("info") {
        None => {
            return Err(AppError::Rpc(
                "node did not report the name's info: could not check when it expires".into(),
            ))
        }
        Some(serde_json::Value::Null) => {
            return Err(AppError::InvalidInput(
                "the name has no on-chain state or has expired".into(),
            ))
        }
        Some(i) => i,
    };
    let Some(renewal) = info
        .get("renewal")
        .and_then(serde_json::Value::as_u64)
        .and_then(|r| u32::try_from(r).ok())
    else {
        return Err(AppError::Rpc(
            "node did not report the name's renewal height: could not check when it expires".into(),
        ));
    };
    let Some(claimed) = info.get("claimed").and_then(serde_json::Value::as_u64) else {
        return Err(AppError::Rpc(
            "node did not report whether the name was claimed: could not check when it expires"
                .into(),
        ));
    };
    let end = params.expiry_end(i64::from(renewal), claimed > 0);
    if end <= params.finalize_margin(tip, remaining_lockup) {
        return Ok(ExpiryNotice::Refuse { expiry_end: end });
    }
    let blocks_left = end - tip;
    if blocks_left < LOCK_WARN_DAYS * i64::from(params.margin_day()) {
        Ok(ExpiryNotice::Warn { blocks_left })
    } else {
        Ok(ExpiryNotice::Ok)
    }
}

/// R31's refusal of a lock (or of its send) that the name would not survive.
pub fn expires_before_the_lock(name: &str, expiry_end: i64) -> AppError {
    AppError::InvalidInput(format!(
        "'{name}' expires at block {expiry_end}, before its transfer into the lock could be \
         finalized: renew it first"
    ))
}

/// R18/R19: whether the lock TRANSFER `(lock_transfer_txid, 0)` owns the
/// name, from hsd's `getnameinfo`: `info.owner` is that outpoint and
/// `info.revoked` is 0 (a REVOKE leaves `owner` at the coin it spent and
/// sets `revoked`, hsd `chain.js`). Finalize & sign and the sync job ask it
/// the same way.
pub fn lock_transfer_owns_name(
    owner_hash: &str,
    owner_index: u64,
    revoked: u64,
    lock_transfer_txid: &str,
) -> bool {
    revoked == 0 && owner_index == 0 && owner_hash.eq_ignore_ascii_case(lock_transfer_txid)
}

/// The block of the name's TRANSFER, hsd's `info.transfer` in a
/// `getnameinfo` reply's `info`. Finalize & sign and the sync job both take
/// the lockup from this fact, not from the TRANSFER coin's height: it is the
/// field hsd's FINALIZE rule reads (`chain.js`, `height < ns.transfer +
/// transferLockup`), set when the block holding the TRANSFER is connected.
/// `None` when the reply leaves it out, gives a number no block can have
/// (above `i64::MAX`), or says 0, which in hsd means "no TRANSFER" (set on
/// FINALIZE, UPDATE and REVOKE): with our TRANSFER the owner, not hsd's
/// whole answer.
pub fn transfer_height(info: &serde_json::Value) -> Option<i64> {
    info.get("transfer")
        .and_then(serde_json::Value::as_u64)
        .and_then(|t| i64::try_from(t).ok())
        .filter(|t| *t > 0)
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

/// The lock coin a FINALIZE into the lock creates: `(index, value)` of the
/// plan's one FINALIZE output paying `lock_address`. The price steps are
/// signed over that output and the listing records its index; a plan with
/// none, or more than one, is refused.
pub fn lock_output(plan: &DraftPlan, lock_address: &str) -> Result<(u32, u64), AppError> {
    let mut found = plan
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.covenant_type == COV_FINALIZE && o.address == lock_address);
    match (found.next(), found.next()) {
        (Some((i, o)), None) => {
            let i = u32::try_from(i)
                .map_err(|_| AppError::Other("FINALIZE output index out of range".into()))?;
            Ok((i, o.value))
        }
        _ => Err(AppError::Other(
            "the FINALIZE plan does not pay exactly one FINALIZE into the lock".into(),
        )),
    }
}

/// R23: a listing file's `expiresAt` is the MTP at signing plus 365 days.
pub const LISTING_LIFETIME_SECS: u64 = 365 * 86_400;

/// R20: what each signed price step means, shown in the confirmation and
/// kept on the FINALIZE draft.
pub const STEP_SIGNATURE_PERMANENCE: &str = "Each signature lets anyone buy the name at its \
     price until the listing is cancelled and the cancel is mined. A price can be lowered \
     later, never raised.";

/// R18 at Finalize & sign: the confirmed lock TRANSFER does not commit to
/// the lock this wallet derives for the name.
pub const LOCK_COMMITMENT_MISMATCH: &str = "the lock transfer does not commit to this \
     wallet's lock for the name: nothing was finalized or signed";

/// Finalize & sign is refused while the listing's Cancel transfer may still
/// be mined: both spend the lock TRANSFER coin.
pub const CANCEL_TRANSFER_PENDING: &str = "a Cancel transfer of this listing is pending and \
     spends the same coin: Finalize & sign waits until it is dropped, failed or deleted";

/// R19: the price a person typed, in HNS, as doos. Refused, with the reason:
/// not a plain decimal, more than 6 decimals, 0, below the dust limit, above
/// the money supply. Never rounds.
pub fn parse_step_price(text: &str) -> Result<u64, AppError> {
    let bad = |why: String| AppError::InvalidInput(format!("price {text:?}: {why}"));
    let t = text.trim();
    if t.is_empty() {
        return Err(bad("enter a price in HNS".into()));
    }
    // No '.' is a whole number of HNS: no decimals, not an error.
    let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && frac.is_empty()) || !digits(whole) || !digits(frac) {
        return Err(bad("not a number of HNS".into()));
    }
    if frac.len() > 6 {
        return Err(bad("HNS has at most 6 decimals".into()));
    }
    let supply = || bad("above the money supply".into());
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| supply())?
    };
    // `frac` is 0..=6 ASCII digits, so the padded string always parses.
    let frac: u64 = format!("{frac:0<6}").parse().map_err(|_| supply())?;
    let doos = whole
        .checked_mul(1_000_000)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(supply)?;
    if doos == 0 {
        return Err(bad("a price must be above 0".into()));
    }
    if doos < DUST_THRESHOLD {
        return Err(bad(format!(
            "below the dust limit of {}",
            doos_to_hns_string(DUST_THRESHOLD)
        )));
    }
    if doos > MAX_MONEY {
        return Err(supply());
    }
    Ok(doos)
}

/// One signed price step as `shakedex_listings.steps_json` stores it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredStep {
    pub price: u64,
    pub lock_time: u64,
    /// 65 bytes, hex: the low-S signature and the 0x84 sighash byte.
    pub signature: String,
}

/// What the Finalize & sign confirmation (R20) is built from.
pub struct FinalizeAndSignRows<'a> {
    pub name: &'a str,
    pub finalize_fee: u64,
    pub lock_address: &'a str,
    pub payment_address: &'a str,
    /// `(price, lock time)` of every step about to be signed.
    pub steps: &'a [(u64, u64)],
    /// The MTP the steps' validity is judged at (R3).
    pub mtp: u64,
}

/// R20: the rows of the Finalize & sign confirmation, `{ "rows": [...] }`
/// as the secure window renders a `confirm` request.
pub fn finalize_and_sign_rows(r: &FinalizeAndSignRows) -> serde_json::Value {
    let row = |label: String, value: String| serde_json::json!({ "label": label, "value": value });
    let mut rows = vec![
        row(
            "Action".into(),
            "Finalize into the lock and sign the price".into(),
        ),
        row("Name".into(), r.name.into()),
        row(
            "Network fee (finalize into the lock)".into(),
            doos_to_hns_string(r.finalize_fee),
        ),
    ];
    for (i, &(price, lock_time)) in r.steps.iter().enumerate() {
        let when = if secs_until_valid(lock_time, r.mtp) == 0 {
            "valid at once".to_string()
        } else {
            match i64::try_from(valid_from_mtp(lock_time))
                .ok()
                .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            {
                Some(t) => format!("valid from {}", t.format("%Y-%m-%d %H:%M UTC")),
                // A lock time past what a date holds: say so rather than guess.
                None => "valid from a time too far away to show".into(),
            }
        };
        rows.push(row(
            format!("Price step {}", i + 1),
            format!("{}, {when}", doos_to_hns_string(price)),
        ));
    }
    rows.push(row("Paid to".into(), r.payment_address.into()));
    rows.push(row("Lock address".into(), r.lock_address.into()));
    rows.push(row("Warning".into(), STEP_SIGNATURE_PERMANENCE.into()));
    serde_json::json!({ "rows": rows })
}

/// R31's refusal at Finalize & sign: the name would expire within a day of
/// its FINALIZE into the lock. Renewing needs the TRANSFER undone first.
pub fn expires_before_finalize(name: &str, expiry_end: i64) -> AppError {
    AppError::InvalidInput(format!(
        "'{name}' expires at block {expiry_end}, too soon to finalize it into the lock: \
         cancel the transfer (Cancel transfer) and renew the name first"
    ))
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

    /// The lock coin is the plan's one FINALIZE output to the lock address,
    /// wherever it sits: its index and value are read from the plan.
    #[test]
    fn lock_output_is_the_plans_finalize_into_the_lock() {
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
        assert_eq!(lock_output(&res.plan, &k.address).unwrap(), (0, 1_000_000));
        let mut swapped = res.plan.clone();
        swapped.outputs.swap(0, 1);
        assert_eq!(lock_output(&swapped, &k.address).unwrap(), (1, 1_000_000));
        let other = key("othername");
        assert!(
            lock_output(&res.plan, &other.address).is_err(),
            "another lock"
        );
        let mut none = res.plan.clone();
        none.outputs[0].covenant_type = 0;
        assert!(lock_output(&none, &k.address).is_err(), "no FINALIZE");
        let mut two = res.plan.clone();
        two.outputs.push(two.outputs[0].clone());
        assert!(lock_output(&two, &k.address).is_err(), "two FINALIZEs");
    }

    /// R19: a price is HNS with at most 6 decimals, at least the dust limit
    /// and at most the money supply; every refusal says why.
    #[test]
    fn step_price_boundaries() {
        use crate::noncustodial::send::DUST_THRESHOLD;
        use crate::noncustodial::shakedex::purchase::MAX_MONEY;
        assert_eq!(parse_step_price("5").unwrap(), 5_000_000);
        assert_eq!(parse_step_price(" 1.5 ").unwrap(), 1_500_000);
        assert_eq!(parse_step_price(".5").unwrap(), 500_000);
        assert_eq!(parse_step_price("1.").unwrap(), 1_000_000);
        assert_eq!(parse_step_price("1.000000").unwrap(), 1_000_000);
        assert_eq!(parse_step_price("0.001").unwrap(), DUST_THRESHOLD);
        assert!(parse_step_price("0.000001")
            .unwrap_err()
            .to_string()
            .contains("dust"));
        assert!(parse_step_price("0.000999")
            .unwrap_err()
            .to_string()
            .contains("dust"));
        assert_eq!(parse_step_price("2040000000").unwrap(), MAX_MONEY);
        for (text, why) in [
            ("0", "above 0"),
            ("0.000000", "above 0"),
            ("2040000000.000001", "money supply"),
            ("99999999999999999999", "money supply"),
            ("1.0000001", "6 decimals"),
            ("", "price in HNS"),
            ("  ", "price in HNS"),
            (".", "not a number"),
            ("-1", "not a number"),
            ("+1", "not a number"),
            ("1e3", "not a number"),
            ("1,5", "not a number"),
            ("1.2.3", "not a number"),
            ("abc", "not a number"),
            // Not ASCII digits, though `char::is_numeric` takes them.
            ("\u{661}", "not a number"),
            ("1\u{b2}", "not a number"),
        ] {
            let e = parse_step_price(text).unwrap_err();
            assert!(matches!(e, AppError::InvalidInput(_)), "{text:?}: {e:?}");
            assert!(e.to_string().contains(why), "{text:?}: {e}");
        }
    }

    /// R20: the fee of the FINALIZE, every step's price and when it becomes
    /// valid, where the money goes, and the permanence of each signature.
    #[test]
    fn finalize_and_sign_rows_list_fee_steps_and_permanence() {
        let mtp = 1_783_696_480;
        let rows = finalize_and_sign_rows(&FinalizeAndSignRows {
            name: "dexsale",
            finalize_fee: 12_345,
            lock_address: "rs1qlock",
            payment_address: "rs1qpay",
            steps: &[
                (5_000_000, buy_now_lock_time(mtp)),
                (4_000_000, mtp + 3_600),
            ],
            mtp,
        });
        let rows = rows["rows"].as_array().unwrap();
        let get = |label: &str| -> Vec<String> {
            rows.iter()
                .filter(|r| r["label"] == label)
                .map(|r| r["value"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(get("Name"), ["dexsale"]);
        assert_eq!(
            get("Network fee (finalize into the lock)"),
            ["0.012345 HNS"]
        );
        assert_eq!(get("Price step 1"), ["5.000000 HNS, valid at once"]);
        let later = &get("Price step 2")[0];
        assert!(later.starts_with("4.000000 HNS, valid from "), "{later}");
        assert!(later.ends_with(" UTC"), "{later}");
        assert_eq!(get("Paid to"), ["rs1qpay"]);
        assert_eq!(get("Lock address"), ["rs1qlock"]);
        assert_eq!(get("Warning"), [STEP_SIGNATURE_PERMANENCE]);
        assert!(STEP_SIGNATURE_PERMANENCE.contains("anyone"));
        assert!(STEP_SIGNATURE_PERMANENCE.contains("cancel is mined"));
    }

    /// The stored step reads back as written (`steps_json`, read by T5/T6/T8).
    #[test]
    fn stored_step_round_trips_in_camel_case() {
        let s = StoredStep {
            price: 5,
            lock_time: 7,
            signature: "ab".into(),
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(
            j,
            serde_json::json!({ "price": 5, "lockTime": 7, "signature": "ab" })
        );
        assert_eq!(serde_json::from_value::<StoredStep>(j).unwrap(), s);
    }
}
