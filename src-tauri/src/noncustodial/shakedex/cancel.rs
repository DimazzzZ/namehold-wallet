//! Cancelling a listing (R28): the lock key signs the lock coin
//! `ANYONECANPAY|SINGLE` into a TRANSFER at the lock address committing to
//! a reserved address of ours, and after the lockup a FINALIZE out of the
//! lock brings the name there.

use crate::error::AppError;
use crate::noncustodial::actions::{DraftPlan, PlanInput, PlanResult, FINAL_SEQUENCE};
use crate::noncustodial::address;
use crate::noncustodial::covenants;
use crate::noncustodial::derivation::BRANCH_RECEIVE;
use crate::noncustodial::hd::{bip44_path, ExtendedPrivKey, HARDENED_OFFSET};
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::SpendableCoin;
use crate::noncustodial::shakedex::funding::{cov_out, fund};
use crate::noncustodial::shakedex::lock_key::LockKey;
use crate::noncustodial::shakedex::purchase::{build_purchase_finalize_plan, FinalizeInput};
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::sync::COV_TRANSFER;
use crate::noncustodial::tx::sighash;
use crate::noncustodial::types::doos_to_hns_string;

/// `ANYONECANPAY|SINGLE`: the lock key commits to the lock input and the
/// TRANSFER at the same index only, as shakedex's cancel does.
pub const CANCEL_SIGHASH: u32 = sighash::ANYONECANPAY | sighash::SINGLE;

/// `wallet_tx_drafts.action` of a cancel draft.
pub const CANCEL_ACTION: &str = "shakedex_cancel";
/// `wallet_tx_drafts.action` of a draft finalizing a cancelled listing's
/// name out of the lock.
pub const CANCEL_FINALIZE_ACTION: &str = "shakedex_cancel_finalize";

/// R28: what a cancel cannot stop before it is mined, shown in its prompt
/// and kept on its draft.
pub const CANCEL_STILL_BUYABLE: &str = "Anyone holding the listing file can still buy the \
     name at its current price until this cancel is mined.";
/// R28: hsd answers `sendrawtransaction` with the txid whatever its mempool
/// does, so a cancel beaten by a purchase already there reads as sent.
pub const CANCEL_MEMPOOL_PURCHASE: &str = "If a purchase of the name is already in your \
     node's mempool, the node does not take this cancel and it is never mined, although the \
     node still answers it with a transaction id.";
/// R28: the way home after the cancel is mined.
pub const CANCEL_THEN_FINALIZE: &str = "Once the cancel is mined and the transfer lockup is \
     over, finalize it to bring the name home.";
/// Why a cancel draft that can never land was dropped (R28,
/// `queries::release_losing_cancel`).
pub const CANCEL_LOST_TO_PURCHASE: &str = "a purchase of the name was mined first: this \
     cancel can never be mined, and its coins are free again";
pub const CANCEL_LOST_TO_ANOTHER: &str = "another transfer out of the lock was mined first: \
     this cancel can never be mined, and its coins are free again";

/// What the cancel's prompt (R28) is built from.
pub struct CancelRows<'a> {
    pub name: &'a str,
    pub fee: u64,
    pub cancel_address: &'a str,
    pub lock_address: &'a str,
    /// The current step's price at the node's MTP (R3); `None` for a lock
    /// restored by name, whose steps this device does not know.
    pub current_price: Option<u64>,
}

/// R28: the rows of the cancel's own confirmation, `{ "rows": [...] }` as
/// the secure window renders a `confirm` request.
pub fn cancel_rows(r: &CancelRows) -> serde_json::Value {
    let row = |label: &str, value: String| serde_json::json!({ "label": label, "value": value });
    serde_json::json!({ "rows": [
        row("Action", "Cancel the listing".into()),
        row("Name", r.name.into()),
        row("Network fee (cancel)", doos_to_hns_string(r.fee)),
        row(
            "Current price",
            match r.current_price {
                Some(price) => doos_to_hns_string(price),
                None => "not known on this device (a lock restored by name)".into(),
            },
        ),
        row("Name comes home to", r.cancel_address.into()),
        row("Lock address", r.lock_address.into()),
        row("Until it is mined", CANCEL_STILL_BUYABLE.into()),
        row("A purchase already sent", CANCEL_MEMPOOL_PURCHASE.into()),
        row("Then", CANCEL_THEN_FINALIZE.into()),
    ] })
}

/// T1b carry (R21, R28), the command's half of the destination rule: the
/// cancel plan is for the profile's `account`, and its lock-key input 0 is
/// a `0x83` input carrying the listing's cancel path (receive branch,
/// `cancel_index`). The signer re-derives the TRANSFER's commitment from
/// that path ([`check_lock_key_input`]); the command has checked that the
/// path is the listing's reserved cancel address
/// (`queries::receive_address_at`).
pub fn check_cancel_plan(
    plan: &DraftPlan,
    account: u32,
    cancel_index: u32,
) -> Result<(), AppError> {
    match plan.inputs.first() {
        Some(i)
            if plan.account == account
                && i.lock_key_name.is_some()
                && i.sighash_type == CANCEL_SIGHASH
                && i.branch == BRANCH_RECEIVE
                && i.child_index == cancel_index =>
        {
            Ok(())
        }
        _ => Err(AppError::Other(
            "the cancel plan is not for this listing's cancel address: nothing was signed".into(),
        )),
    }
}

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

/// R17, enforced by the signer for a lock-key input of a plan: a lock key signs price steps (`shakedex::sell::sign_step`, outside
/// any plan) and cancels, nothing else. A cancel signs its lock input `i`
/// `ANYONECANPAY|SINGLE`, which commits to output `i` alone, so that output
/// must exist (hsd's SINGLE past the last output commits to none) and be the
/// TRANSFER at this key's lock address that consensus requires of a cancel,
/// of this name (item 0), to the address of ours the input's path names
/// (items 2 and 3: version 0 and its key hash). A TRANSFER to any other
/// address would give the name away once finalized. That path must be a
/// receive address (the cancel commits to a reserved one, R21/R28) at an
/// unhardened index: any other path derives from the seed but is never
/// synced or restored, which would strand the name. The input's sequence must
/// be final and the plan's lock time 0, as in shakedex's cancel: the
/// signature commits to both, and a far lock time would make a cancel that
/// cannot be mined while the price steps stay fillable.
pub(crate) fn check_lock_key_input(
    plan: &DraftPlan,
    i: usize,
    name: &str,
    key: &LockKey,
    master: &ExtendedPrivKey,
    network: Network,
) -> Result<(), AppError> {
    let inp = &plan.inputs[i];
    let sighash_type = inp.sighash_type;
    if sighash_type != CANCEL_SIGHASH {
        return Err(AppError::InvalidInput(format!(
            "a lock key signs only a cancel (sighash 0x83), not sighash {sighash_type:#04x}"
        )));
    }
    if inp.sequence != FINAL_SEQUENCE || plan.locktime != 0 {
        return Err(AppError::InvalidInput(
            "a cancel has a final sequence and no lock time".into(),
        ));
    }
    if inp.branch != BRANCH_RECEIVE || inp.child_index >= HARDENED_OFFSET {
        return Err(AppError::InvalidInput(
            "a cancel commits the name to a receive address of ours".into(),
        ));
    }
    let dest_path = bip44_path(network, plan.account, inp.branch, inp.child_index);
    let dest = address::pubkey_to_hash160(&master.derive_path(&dest_path)?.compressed_pubkey());
    let name_hash = hex::encode(crate::noncustodial::names::hash_name(name)?);
    match plan.outputs.get(i) {
        Some(o)
            if o.covenant_type == COV_TRANSFER
                && o.address == key.address
                && o.covenant_items_hex.len() == 4
                && o.covenant_items_hex[0] == name_hash
                && o.covenant_items_hex[2] == "00"
                && o.covenant_items_hex[3] == hex::encode(dest) =>
        {
            Ok(())
        }
        _ => Err(AppError::InvalidInput(
            "a cancel's lock input must be matched by a TRANSFER of its name at its lock \
             address to an address of ours"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::actions::sign_plan;
    use crate::noncustodial::session::SignerSession;
    use crate::noncustodial::shakedex::lock_key::derive_lock_key;
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

    /// R28: the cancel's own prompt says what the cancel cannot stop (a buyer
    /// until it is mined; a purchase already in the node's mempool, which
    /// hsd still answers with the cancel's txid), the fee, where the name
    /// comes home to, and the current price, or that this device does not
    /// know it (a lock restored by name).
    #[test]
    fn cancel_rows_say_what_r28_says() {
        let rows = |price| {
            cancel_rows(&CancelRows {
                name: "dexreviews",
                fee: 2_340,
                cancel_address: "hs1qcancel",
                lock_address: "hs1qlock",
                current_price: price,
            })["rows"]
                .as_array()
                .unwrap()
                .clone()
        };
        let value = |rows: &[serde_json::Value], label: &str| {
            rows.iter()
                .find(|r| r["label"] == label)
                .map(|r| r["value"].as_str().unwrap().to_string())
        };
        let r = rows(Some(5_000_000));
        assert_eq!(value(&r, "Action").as_deref(), Some("Cancel the listing"));
        assert_eq!(value(&r, "Name").as_deref(), Some("dexreviews"));
        assert_eq!(
            value(&r, "Network fee (cancel)").as_deref(),
            Some("0.002340 HNS")
        );
        assert_eq!(value(&r, "Current price").as_deref(), Some("5.000000 HNS"));
        assert_eq!(
            value(&r, "Name comes home to").as_deref(),
            Some("hs1qcancel")
        );
        assert_eq!(value(&r, "Lock address").as_deref(), Some("hs1qlock"));
        assert_eq!(
            value(&r, "Until it is mined").as_deref(),
            Some(CANCEL_STILL_BUYABLE)
        );
        assert_eq!(
            value(&r, "A purchase already sent").as_deref(),
            Some(CANCEL_MEMPOOL_PURCHASE)
        );
        assert_eq!(value(&r, "Then").as_deref(), Some(CANCEL_THEN_FINALIZE));
        assert!(CANCEL_STILL_BUYABLE.contains("until this cancel is mined"));
        assert!(CANCEL_MEMPOOL_PURCHASE.contains("mempool"));
        assert!(CANCEL_MEMPOOL_PURCHASE.contains("transaction id"));
        let r = rows(None);
        assert_eq!(
            value(&r, "Current price").as_deref(),
            Some("not known on this device (a lock restored by name)")
        );
    }

    /// T1b carry: the command refuses a cancel plan that is not for the
    /// profile's account, or whose lock input does not carry the listing's
    /// cancel path (the receive branch at the stored index): the signer
    /// re-derives the TRANSFER's commitment from that path, so the path is
    /// what ties the signature to the reserved address.
    #[test]
    fn cancel_plan_is_for_the_profiles_account_and_path() {
        let k = key();
        let to = cancel_to();
        let funding = [coin(1, 1_000_000, 10)];
        let res = build_cancel_plan(&input(&k, &to, &funding, 5)).unwrap();
        check_cancel_plan(&res.plan, 0, CANCEL_INDEX).expect("ours");
        assert!(
            check_cancel_plan(&res.plan, 1, CANCEL_INDEX).is_err(),
            "another account"
        );
        assert!(
            check_cancel_plan(&res.plan, 0, CANCEL_INDEX + 1).is_err(),
            "another index"
        );
        let mut change_branch = res.plan.clone();
        change_branch.inputs[0].branch = 1;
        assert!(
            check_cancel_plan(&change_branch, 0, CANCEL_INDEX).is_err(),
            "change branch"
        );
        let mut not_lock = res.plan.clone();
        not_lock.inputs[0].lock_key_name = None;
        assert!(
            check_cancel_plan(&not_lock, 0, CANCEL_INDEX).is_err(),
            "no lock-key input"
        );
        let mut other_sighash = res.plan;
        other_sighash.inputs[0].sighash_type = sighash::ALL;
        assert!(
            check_cancel_plan(&other_sighash, 0, CANCEL_INDEX).is_err(),
            "not 0x83"
        );
    }
}
