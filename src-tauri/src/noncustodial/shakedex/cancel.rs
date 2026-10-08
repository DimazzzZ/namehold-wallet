//! Cancelling a listing (R28): the lock key signs the lock coin
//! `ANYONECANPAY|SINGLE` into a TRANSFER at the lock address committing to
//! a reserved address of ours, and after the lockup a FINALIZE out of the
//! lock brings the name there.

use crate::error::AppError;
use crate::noncustodial::actions::{PlanInput, PlanResult, FINAL_SEQUENCE};
use crate::noncustodial::address;
use crate::noncustodial::covenants;
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::SpendableCoin;
use crate::noncustodial::shakedex::funding::{cov_out, fund};
use crate::noncustodial::shakedex::purchase::{build_purchase_finalize_plan, FinalizeInput};
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::tx::sighash;

/// `ANYONECANPAY|SINGLE`: the lock key commits to the lock input and the
/// TRANSFER at the same index only, as shakedex's cancel does.
pub const CANCEL_SIGHASH: u32 = sighash::ANYONECANPAY | sighash::SINGLE;

/// `wallet_tx_drafts.action` of a cancel draft.
pub const CANCEL_ACTION: &str = "shakedex_cancel";
/// `wallet_tx_drafts.action` of a draft finalizing a cancelled listing's
/// name out of the lock.
pub const CANCEL_FINALIZE_ACTION: &str = "shakedex_cancel_finalize";

/// What the caller supplies from the name state to build a cancel; the caller
/// checks the lock coin's TRANSFER commitment.
pub struct CancelInput<'a> {
    pub network: Network,
    pub account: u32,
    pub name: &'a str,
    pub name_height: u32,
    /// The listing's lock coin (the FINALIZE into the lock).
    pub lock_outpoint: ([u8; 32], u32),
    pub lock_value: u64,
    /// The listing's lock key's public key; the signer re-derives the key
    /// and refuses the cancel if its lock address differs.
    pub lock_pubkey: [u8; 33],
    /// A reserved address of ours (R21) the name comes home to.
    pub cancel_address: &'a str,
    /// The derivation path of `cancel_address` (receive branch, unhardened
    /// index). The signer re-derives the address from it and refuses the
    /// cancel if the TRANSFER commits anywhere else.
    pub cancel_branch: u32,
    pub cancel_index: u32,
    pub funding: &'a [SpendableCoin],
    pub change_address: &'a str,
    pub rate: u64,
    #[cfg(test)]
    pub fixed_fee: Option<u64>,
}

#[cfg(test)]
impl CancelInput<'_> {
    /// Pin the fee instead of sizing it, to reproduce a vector's fee.
    pub fn with_fixed_fee_for_tests(mut self, fee: u64) -> Self {
        self.fixed_fee = Some(fee);
        self
    }
}

/// The cancel: `[our lock coin (lock key, 0x83), our funding...] ->
/// [TRANSFER at the lock address committing to our cancel address,
/// change?]`. Spending the lock coin ends every price step signed over it.
/// The lock input's `branch`/`child_index` carry the cancel address's path,
/// which the signer checks the TRANSFER against.
pub fn build_cancel_plan(c: &CancelInput) -> Result<PlanResult, AppError> {
    let nh = names::hash_name(c.name)?;
    let (version, hash) = address::decode(c.network, c.cancel_address)?;
    let lead = PlanInput {
        txid: hex::encode(c.lock_outpoint.0),
        vout: c.lock_outpoint.1,
        value: c.lock_value,
        branch: c.cancel_branch,
        child_index: c.cancel_index,
        sighash_type: CANCEL_SIGHASH,
        sequence: FINAL_SEQUENCE,
        foreign_witness_hex: None,
        lock_key_name: Some(c.name.to_owned()),
    };
    let transfer = covenants::transfer(&nh, c.name_height, version, &hash);
    let before = vec![cov_out(
        c.lock_value,
        lock_address(c.network, &c.lock_pubkey)?,
        &transfer,
    )];
    #[cfg(test)]
    let fixed = c.fixed_fee;
    #[cfg(not(test))]
    let fixed = None;
    fund(
        c.network,
        c.account,
        0,
        lead,
        c.lock_value,
        before,
        vec![],
        c.funding,
        c.change_address,
        c.rate,
        fixed,
    )
}

/// The cancel's FINALIZE is the same transaction as a buyer's FINALIZE out
/// of the lock: the cancel's TRANSFER coin with the `[lockScript]` witness,
/// to our cancel address, the fee from our coins.
pub type CancelFinalizeInput<'a> = FinalizeInput<'a>;

/// FINALIZE the cancelled listing's name out of the lock to our cancel
/// address (`f.dest_address`), after the transfer lockup.
pub fn build_cancel_finalize_plan(f: &CancelFinalizeInput) -> Result<PlanResult, AppError> {
    build_purchase_finalize_plan(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::actions::{sign_plan, DraftPlan};
    use crate::noncustodial::hd::{bip44_path, ExtendedPrivKey};
    use crate::noncustodial::session::SignerSession;
    use crate::noncustodial::shakedex::lock_key::{derive_lock_key, LockKey};
    use crate::noncustodial::sync::COV_TRANSFER;
    use crate::noncustodial::tx::Transaction;

    const CHANGE: &str = "hs1qdhtaj7ws7chd2z2tulrmakqww428myx08d6w3v";
    const CANCEL_INDEX: u32 = 11;

    fn master() -> ExtendedPrivKey {
        ExtendedPrivKey::from_seed(&[7u8; 64]).unwrap()
    }

    fn key() -> LockKey {
        derive_lock_key(&master(), Network::Main, 0, "dexreviews").unwrap()
    }

    /// Our receive address at `CANCEL_INDEX` under the test seed.
    fn cancel_to() -> String {
        let path = bip44_path(Network::Main, 0, 0, CANCEL_INDEX);
        let pk = master().derive_path(&path).unwrap().compressed_pubkey();
        address::encode_p2wpkh(Network::Main, &address::pubkey_to_hash160(&pk)).unwrap()
    }

    fn coin(txid_byte: u8, value: u64, child: u32) -> SpendableCoin {
        SpendableCoin {
            txid: hex::encode([txid_byte; 32]),
            vout: 0,
            value,
            branch: 0,
            child_index: child,
        }
    }

    fn input<'a>(
        k: &LockKey,
        to: &'a str,
        funding: &'a [SpendableCoin],
        rate: u64,
    ) -> CancelInput<'a> {
        CancelInput {
            network: Network::Main,
            account: 0,
            name: "dexreviews",
            name_height: 120,
            lock_outpoint: ([0x2c; 32], 0),
            lock_value: 1_000_000,
            lock_pubkey: k.pubkey,
            cancel_address: to,
            cancel_branch: 0,
            cancel_index: CANCEL_INDEX,
            funding,
            change_address: CHANGE,
            rate,
            fixed_fee: None,
        }
    }

    fn signed(plan: &DraftPlan) -> Transaction {
        let mut session = SignerSession::unlock("p".into(), Network::Main, master(), 60_000);
        let (hex, _) = sign_plan(&mut session, plan).unwrap();
        Transaction::decode(&hex::decode(hex).unwrap()).unwrap()
    }

    /// Input 0 is our lock coin, marked for the lock key, carrying the cancel
    /// address's path, 0x83 with a final sequence; output 0 is the TRANSFER at
    /// the lock address whose covenant commits to our cancel address; the
    /// signer accepts the plan.
    #[test]
    fn cancel_spends_the_lock_into_a_transfer_to_our_cancel_address() {
        let k = key();
        let to = cancel_to();
        let funding = [coin(1, 1_000_000, 10)];
        let res = build_cancel_plan(&input(&k, &to, &funding, 5)).unwrap();
        let inp0 = &res.plan.inputs[0];
        assert_eq!(inp0.txid, hex::encode([0x2c; 32]));
        assert_eq!(inp0.lock_key_name.as_deref(), Some("dexreviews"));
        assert_eq!((inp0.branch, inp0.child_index), (0, CANCEL_INDEX));
        assert!(inp0.foreign_witness_hex.is_none());
        assert_eq!(inp0.sighash_type, CANCEL_SIGHASH);
        assert_eq!(inp0.sequence, FINAL_SEQUENCE);
        assert_eq!(res.plan.locktime, 0);
        let out0 = &res.plan.outputs[0];
        assert_eq!(out0.covenant_type, COV_TRANSFER);
        assert_eq!(out0.address, k.address);
        assert_eq!(out0.value, 1_000_000);
        let (version, hash) = address::decode(Network::Main, &to).unwrap();
        assert_eq!(out0.covenant_items_hex[2], hex::encode([version]));
        assert_eq!(out0.covenant_items_hex[3], hex::encode(hash));
        assert_eq!(res.plan.own_inputs(), vec![(funding[0].txid.clone(), 0)]);
        signed(&res.plan);
    }

    /// R4: the fee is sized with the real lock witness ([65-byte signature,
    /// 44-byte script]), not a P2WPKH one, so it equals the signed
    /// transaction's own vsize times the rate. That this vsize is hsd's is
    /// pinned by `cancel_matches_hsd_signed_hex`.
    #[test]
    fn fee_covers_hsd_vsize_with_the_lock_witness() {
        let k = key();
        let to = cancel_to();
        let funding = [coin(1, 1_000_000, 10)];
        let res = build_cancel_plan(&input(&k, &to, &funding, 7)).unwrap();
        let tx = signed(&res.plan);
        assert_eq!(
            tx.inputs[0].witness[1], k.script,
            "the lock witness is signed in"
        );
        let without_lock_witness = {
            let mut t = tx.clone();
            t.inputs[0].witness.clear();
            t.vsize()
        };
        assert!(tx.vsize() > without_lock_witness);
        assert_eq!(res.fee, tx.vsize() * 7, "fee sized on the exact vsize");
        let out: u64 = res.plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(res.input_total, out + res.fee);
    }
}
