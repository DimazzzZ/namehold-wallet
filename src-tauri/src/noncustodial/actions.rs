//! Covenant transaction planning + signing.
//!
//! A covenant action is built into a fully-formed unsigned [`Transaction`] at
//! build time (coin selection + outputs + covenant), and persisted as a
//! serializable [`DraftPlan`]. At sign time the plan is reconstructed and each
//! input is signed — no re-selection — so the signed tx matches the preview.
//!
//! Name covenants live on OUTPUTS; the inputs being spent are ordinary P2WPKH
//! (the name UTXO is P2WPKH-locked), so signing reuses `tx::sign_p2wpkh_input`.
//! Each input carries its own sighash type (default `SIGHASH_ALL`).

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::derivation::BRANCH_RECEIVE;
use crate::noncustodial::hd::{bip44_path, ExtendedPrivKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::noncustodial::send::{estimate_fee_with_primary, SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::shakedex::cancel::CANCEL_SIGHASH;
use crate::noncustodial::shakedex::lock_key::{derive_lock_key, LockKey};
use crate::noncustodial::sync::COV_TRANSFER;
use crate::noncustodial::tx::{
    output_address_from_string, sighash, Covenant, Input, Outpoint, Output, Transaction,
};

/// hsd's default input sequence: final, no relative or absolute lock.
pub const FINAL_SEQUENCE: u32 = 0xffff_ffff;

fn final_sequence() -> u32 {
    FINAL_SEQUENCE
}

/// One input of a draft plan: prevout + the derivation path needed to re-sign.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanInput {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub branch: u32,
    pub child_index: u32,
    pub sighash_type: u32,
    /// Input sequence. Plans stored before this field existed default to
    /// final, which is what every builder used.
    #[serde(default = "final_sequence")]
    pub sequence: u32,
    /// A foreign input, a coin someone else controls such as a Shakedex
    /// seller's lock, carries its finished witness here, hex per stack item.
    /// We never sign it; `branch`/`child_index` are ignored for it.
    #[serde(default)]
    pub foreign_witness_hex: Option<Vec<String>>,
    /// One of our lock coins, signed with the lock key of this name. The key
    /// is derived from the seed at sign time (`derive_lock_key` with the
    /// plan's network and account), never stored, and signs only a cancel
    /// (R17); the witness is `[signature, lock script]`. For such an input
    /// `branch` and `child_index` are the derivation path of the address of
    /// ours the cancel's TRANSFER commits the name to, which the signer
    /// re-derives and checks; its coin is not a tracked coin.
    #[serde(default)]
    pub lock_key_name: Option<String>,
}

/// One output of a draft plan (value + address + covenant items as hex).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanOutput {
    pub value: u64,
    pub address: String,
    pub covenant_type: u8,
    pub covenant_items_hex: Vec<String>,
}

/// A persisted, sign-ready plan (stored in `wallet_tx_drafts.signing_inputs_json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPlan {
    pub version: u32,
    pub locktime: u32,
    pub account: u32,
    pub network: String,
    pub inputs: Vec<PlanInput>,
    pub outputs: Vec<PlanOutput>,
    /// Zero-based index of the change output (if any). Used by Ledger to skip
    /// prompting the user for change verification.
    #[serde(default)]
    pub change_output_index: Option<usize>,
}

impl DraftPlan {
    /// True when the plan carries anything the Ledger signer cannot
    /// represent: a foreign input, a lock-key input, a non-final sequence, or
    /// a lock time.
    pub fn has_foreign_or_custom_inputs(&self) -> bool {
        self.locktime != 0
            || self.inputs.iter().any(|i| {
                i.foreign_witness_hex.is_some()
                    || i.lock_key_name.is_some()
                    || i.sequence != FINAL_SEQUENCE
            })
    }

    /// The outpoints of the wallet's own tracked coins, which a draft
    /// reserves: every input but the foreign ones (a seller's lock coin) and
    /// our own lock coin signed by a lock key
    /// (`insert_tx_draft_reserving_coins_in_tx` refuses an outpoint it cannot
    /// claim).
    pub fn own_inputs(&self) -> Vec<(String, u32)> {
        self.inputs
            .iter()
            .filter(|i| i.foreign_witness_hex.is_none() && i.lock_key_name.is_none())
            .map(|i| (i.txid.clone(), i.vout))
            .collect()
    }
}

/// The name UTXO a covenant action spends (when applicable).
pub struct NameInputSpec {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub branch: u32,
    pub child_index: u32,
    pub sighash_type: u32,
}

/// The covenant output an action creates.
#[derive(Clone)]
pub struct PrimaryOutput {
    pub value: u64,
    pub address: String,
    pub covenant: Covenant,
}

/// Result of planning: the plan plus a preview (unsigned hex, txid, fee/change).
#[derive(Debug)]
pub struct PlanResult {
    pub plan: DraftPlan,
    pub unsigned_tx_hex: String,
    pub txid: String,
    pub fee: u64,
    pub change: u64,
    pub input_total: u64,
}

/// hsd txid hex → 32-byte prevout hash. Handshake does NOT byte-reverse hashes,
/// so this is a plain decode with no reversal (matching the node's coin hash and
/// what gets written into the spending input's prevout).
fn outpoint_hash(txid: &str) -> Result<[u8; 32], AppError> {
    let bytes = hex::decode(txid).map_err(|e| AppError::InvalidInput(format!("bad txid: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::InvalidInput("txid must be 32 bytes".into()));
    }
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    Ok(h)
}

/// Coin selection helper: given the total output value, total vbytes of all
/// covenant outputs, base input count, name value, and funding coins, select
/// the minimum number of funding coins needed to cover outputs + fee, and
/// return (taken, fee, change).
///
/// Coin selection is largest-first (coins are already sorted). Change below
/// dust is folded into the fee.
fn select_funding(
    total_output_value: u64,
    total_primary_vbytes: u64,
    base_in: u64,
    name_value: u64,
    funding: &[SpendableCoin],
    rate: u64,
) -> Result<(usize, u64, u64), AppError> {
    let mut taken = 0usize;
    let (fee, change) = loop {
        let funded: u64 = funding[..taken].iter().map(|c| c.value).sum();
        let total_in = name_value + funded;
        let n_in = base_in + taken as u64;

        if n_in >= 1 {
            crate::noncustodial::send::check_standard_weight(
                crate::noncustodial::send::estimate_size_with_primary(
                    n_in,
                    total_primary_vbytes,
                    1,
                ),
                n_in,
            )?;
            let fee_wc = estimate_fee_with_primary(n_in, total_primary_vbytes, 1, rate);
            let fee_nc = estimate_fee_with_primary(n_in, total_primary_vbytes, 0, rate);
            if total_in >= total_output_value + fee_wc {
                let ch = total_in - total_output_value - fee_wc;
                if ch >= DUST_THRESHOLD {
                    break (fee_wc, ch);
                }
                break (total_in - total_output_value, 0);
            }
            if total_in >= total_output_value + fee_nc {
                break (total_in - total_output_value, 0);
            }
        }
        if taken >= funding.len() {
            return Err(AppError::InvalidInput(
                "insufficient funds to cover outputs and fee".into(),
            ));
        }
        taken += 1;
    };
    Ok((taken, fee, change))
}

/// Build a covenant tx: an optional required name input, the covenant output,
/// funded with extra liquid coins to cover `primary.value + fee`, with change.
///
/// Coin selection is largest-first; change below dust is folded into the fee.
pub fn build_plan(
    network: Network,
    account: u32,
    name_input: Option<NameInputSpec>,
    primary: PrimaryOutput,
    funding: &[SpendableCoin],
    change_address: &str,
    rate: u64,
) -> Result<PlanResult, AppError> {
    let base_in = if name_input.is_some() { 1u64 } else { 0 };
    let name_value = name_input.as_ref().map(|n| n.value).unwrap_or(0);

    // The primary output's REAL serialized size (I4): covenant items (name
    // hash, height, resource, renewal block, …) can make a REGISTER/UPDATE/
    // FINALIZE/TRANSFER output far larger than a plain P2WPKH output.
    // Serialize the actual output and measure it rather than assuming the
    // flat per-output constant, so large-resource covenant txs aren't
    // underpriced below min-relay.
    let primary_addr = output_address_from_string(network, &primary.address)?;
    let primary_vbytes = Output {
        value: 0,
        address: primary_addr,
        covenant: primary.covenant.clone(),
    }
    .encoded_len() as u64;

    let (taken, fee, change) = select_funding(
        primary.value,
        primary_vbytes,
        base_in,
        name_value,
        funding,
        rate,
    )?;

    // Assemble plan inputs: name input first, then funding coins.
    let mut plan_inputs = Vec::new();
    if let Some(n) = &name_input {
        plan_inputs.push(PlanInput {
            txid: n.txid.clone(),
            vout: n.vout,
            value: n.value,
            branch: n.branch,
            child_index: n.child_index,
            sighash_type: n.sighash_type,
            sequence: FINAL_SEQUENCE,
            foreign_witness_hex: None,
            lock_key_name: None,
        });
    }
    for c in &funding[..taken] {
        plan_inputs.push(PlanInput {
            txid: c.txid.clone(),
            vout: c.vout,
            value: c.value,
            branch: c.branch,
            child_index: c.child_index,
            sighash_type: sighash::ALL,
            sequence: FINAL_SEQUENCE,
            foreign_witness_hex: None,
            lock_key_name: None,
        });
    }

    // Outputs: covenant output, then change (plain) if any.
    let mut plan_outputs = vec![PlanOutput {
        value: primary.value,
        address: primary.address.clone(),
        covenant_type: primary.covenant.covenant_type,
        covenant_items_hex: primary.covenant.items.iter().map(hex::encode).collect(),
    }];
    let change_output_index = if change > 0 {
        let idx = plan_outputs.len();
        plan_outputs.push(PlanOutput {
            value: change,
            address: change_address.to_string(),
            covenant_type: 0,
            covenant_items_hex: Vec::new(),
        });
        Some(idx)
    } else {
        None
    };

    let plan = DraftPlan {
        version: 0,
        locktime: 0,
        account,
        network: network.as_str().to_string(),
        inputs: plan_inputs,
        outputs: plan_outputs,
        change_output_index,
    };

    // Materialize an unsigned tx for the preview hex + txid (txid is the
    // no-witness hash, so it is identical before/after signing).
    let tx = rebuild_unsigned(&plan, network)?;
    let input_total = name_value + funding[..taken].iter().map(|c| c.value).sum::<u64>();

    Ok(PlanResult {
        unsigned_tx_hex: tx.to_hex(),
        txid: tx.txid(),
        plan,
        fee,
        change,
        input_total,
    })
}

/// Build a batch covenant tx: multiple covenant outputs (e.g. several renewals
/// or reveals) in a single transaction, funded with liquid coins + change.
///
/// This is the batch counterpart to [`build_plan`]. Each entry in `primaries`
/// represents one covenant output (one name action). All covenant outputs share
/// the same funding coins and change address.
///
/// Coin selection is largest-first; change below dust is folded into the fee.
pub fn build_batch_plan(
    network: Network,
    account: u32,
    name_inputs: &[NameInputSpec],
    primaries: &[PrimaryOutput],
    funding: &[SpendableCoin],
    change_address: &str,
    rate: u64,
) -> Result<PlanResult, AppError> {
    if primaries.is_empty() {
        return Err(AppError::InvalidInput(
            "batch plan requires at least one output".into(),
        ));
    }

    let base_in = name_inputs.len() as u64;
    let name_value: u64 = name_inputs.iter().map(|n| n.value).sum();

    // Total value across all covenant outputs.
    let total_output_value: u64 = primaries.iter().map(|p| p.value).sum();

    // Total vbytes for all covenant outputs (used for fee estimation).
    let total_primary_vbytes: u64 = primaries
        .iter()
        .map(|p| {
            let addr = output_address_from_string(network, &p.address)?;
            Ok::<u64, AppError>(
                Output {
                    value: 0,
                    address: addr,
                    covenant: p.covenant.clone(),
                }
                .encoded_len() as u64,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sum();

    let (taken, fee, change) = select_funding(
        total_output_value,
        total_primary_vbytes,
        base_in,
        name_value,
        funding,
        rate,
    )?;

    let mut plan_inputs = Vec::new();
    for n in name_inputs {
        plan_inputs.push(PlanInput {
            txid: n.txid.clone(),
            vout: n.vout,
            value: n.value,
            branch: n.branch,
            child_index: n.child_index,
            sighash_type: n.sighash_type,
            sequence: FINAL_SEQUENCE,
            foreign_witness_hex: None,
            lock_key_name: None,
        });
    }
    for c in &funding[..taken] {
        plan_inputs.push(PlanInput {
            txid: c.txid.clone(),
            vout: c.vout,
            value: c.value,
            branch: c.branch,
            child_index: c.child_index,
            sighash_type: sighash::ALL,
            sequence: FINAL_SEQUENCE,
            foreign_witness_hex: None,
            lock_key_name: None,
        });
    }

    // Outputs: one covenant output per name action, then change (plain).
    let mut plan_outputs = Vec::new();
    for primary in primaries {
        plan_outputs.push(PlanOutput {
            value: primary.value,
            address: primary.address.clone(),
            covenant_type: primary.covenant.covenant_type,
            covenant_items_hex: primary.covenant.items.iter().map(hex::encode).collect(),
        });
    }
    let change_output_index = if change > 0 {
        let idx = plan_outputs.len();
        plan_outputs.push(PlanOutput {
            value: change,
            address: change_address.to_string(),
            covenant_type: 0,
            covenant_items_hex: Vec::new(),
        });
        Some(idx)
    } else {
        None
    };

    let plan = DraftPlan {
        version: 0,
        locktime: 0,
        account,
        network: network.as_str().to_string(),
        inputs: plan_inputs,
        outputs: plan_outputs,
        change_output_index,
    };
    let tx = rebuild_unsigned(&plan, network)?;
    let unsigned_tx_hex = tx.to_hex();
    let funded_total: u64 = funding[..taken].iter().map(|c| c.value).sum();
    let input_total = name_value + funded_total;
    Ok(PlanResult {
        unsigned_tx_hex,
        txid: tx.txid(),
        plan,
        fee,
        change,
        input_total,
    })
}

/// Build a finalize-with-payment tx: a covenant finalize output (transfers name
/// ownership) plus a plain payment output (buyer pays seller), funded from
/// liquid coins with change.
///
/// Used for atomic name swaps: the buyer finalizes a TRANSFER and pays the
/// seller in a single transaction. Both outputs and the fee are covered by
/// the buyer's funding coins.
// The plan builders take a wide but flat set of primitive tx parameters
// (network, account, inputs, outputs, funding, change, rate); grouping them
// into a struct would add indirection without improving clarity.
#[allow(clippy::too_many_arguments)]
pub fn build_finalize_with_payment_plan(
    network: Network,
    account: u32,
    name_input: NameInputSpec,
    finalize: PrimaryOutput,
    payment_address: String,
    payment_value: u64,
    funding: &[SpendableCoin],
    change_address: &str,
    rate: u64,
) -> Result<PlanResult, AppError> {
    if payment_value == 0 {
        return Err(AppError::InvalidInput(
            "payment value must be non-zero for finalize-with-payment".into(),
        ));
    }

    let name_value = name_input.value;
    let finalize_addr = output_address_from_string(network, &finalize.address)?;
    let finalize_vbytes = Output {
        value: 0,
        address: finalize_addr,
        covenant: finalize.covenant.clone(),
    }
    .encoded_len() as u64;
    let payment_addr = output_address_from_string(network, &payment_address)?;
    let payment_vbytes = Output {
        value: 0,
        address: payment_addr.clone(),
        covenant: Covenant::default(),
    }
    .encoded_len() as u64;
    let total_primary_vbytes = finalize_vbytes + payment_vbytes;
    let total_output_value = finalize.value + payment_value;

    let (taken, fee, change) = select_funding(
        total_output_value,
        total_primary_vbytes,
        1, // base_in: the name input (TRANSFER coin)
        name_value,
        funding,
        rate,
    )?;

    // Inputs: name input (TRANSFER coin) + funding coins.
    let mut plan_inputs = vec![PlanInput {
        txid: name_input.txid.clone(),
        vout: name_input.vout,
        value: name_input.value,
        branch: name_input.branch,
        child_index: name_input.child_index,
        sighash_type: name_input.sighash_type,
        sequence: FINAL_SEQUENCE,
        foreign_witness_hex: None,
        lock_key_name: None,
    }];
    for c in &funding[..taken] {
        plan_inputs.push(PlanInput {
            txid: c.txid.clone(),
            vout: c.vout,
            value: c.value,
            branch: c.branch,
            child_index: c.child_index,
            sighash_type: sighash::ALL,
            sequence: FINAL_SEQUENCE,
            foreign_witness_hex: None,
            lock_key_name: None,
        });
    }

    // Outputs: finalize covenant, payment, then change.
    let mut plan_outputs = vec![
        PlanOutput {
            value: finalize.value,
            address: finalize.address.clone(),
            covenant_type: finalize.covenant.covenant_type,
            covenant_items_hex: finalize.covenant.items.iter().map(hex::encode).collect(),
        },
        PlanOutput {
            value: payment_value,
            address: payment_address,
            covenant_type: 0, // plain P2WPKH
            covenant_items_hex: Vec::new(),
        },
    ];
    let change_output_index = if change > 0 {
        let idx = plan_outputs.len();
        plan_outputs.push(PlanOutput {
            value: change,
            address: change_address.to_string(),
            covenant_type: 0,
            covenant_items_hex: Vec::new(),
        });
        Some(idx)
    } else {
        None
    };

    let plan = DraftPlan {
        version: 0,
        locktime: 0,
        account,
        network: network.as_str().to_string(),
        inputs: plan_inputs,
        outputs: plan_outputs,
        change_output_index,
    };
    let tx = rebuild_unsigned(&plan, network)?;
    let unsigned_tx_hex = tx.to_hex();
    let funded_total: u64 = funding[..taken].iter().map(|c| c.value).sum();
    let input_total = name_value + funded_total;
    Ok(PlanResult {
        unsigned_tx_hex,
        txid: tx.txid(),
        plan,
        fee,
        change,
        input_total,
    })
}

/// Reconstruct the unsigned [`Transaction`] from a plan (no witnesses).
pub fn rebuild_unsigned(plan: &DraftPlan, network: Network) -> Result<Transaction, AppError> {
    let mut tx = Transaction::new();
    tx.version = plan.version;
    tx.locktime = plan.locktime;
    for inp in &plan.inputs {
        let mut input = Input::new(Outpoint {
            hash: outpoint_hash(&inp.txid)?,
            index: inp.vout,
        });
        input.sequence = inp.sequence;
        if let Some(items) = &inp.foreign_witness_hex {
            input.witness = items
                .iter()
                .map(|h| {
                    hex::decode(h)
                        .map_err(|e| AppError::InvalidInput(format!("bad witness hex: {e}")))
                })
                .collect::<Result<_, _>>()?;
        }
        tx.inputs.push(input);
    }
    for out in &plan.outputs {
        let items = out
            .covenant_items_hex
            .iter()
            .map(|h| {
                hex::decode(h)
                    .map_err(|e| AppError::InvalidInput(format!("bad covenant item: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        tx.outputs.push(Output {
            value: out.value,
            address: output_address_from_string(network, &out.address)?,
            covenant: Covenant {
                covenant_type: out.covenant_type,
                items,
            },
        });
    }
    Ok(tx)
}

/// Sign a plan with the unlocked session. Returns `(signed_tx_hex, txid)`.
pub fn sign_plan(
    session: &mut SignerSession,
    plan: &DraftPlan,
) -> Result<(String, String), AppError> {
    let tx = sign_plan_tx(session, plan)?;
    Ok((tx.to_hex(), tx.txid()))
}

/// [`sign_plan`], returning the signed [`Transaction`] itself.
fn sign_plan_tx(session: &mut SignerSession, plan: &DraftPlan) -> Result<Transaction, AppError> {
    let network = Network::from_str_opt(&plan.network)
        .ok_or_else(|| AppError::InvalidInput(format!("bad network '{}'", plan.network)))?;
    let mut tx = rebuild_unsigned(plan, network)?;
    let master = session.master()?;
    for (i, inp) in plan.inputs.iter().enumerate() {
        if let Some(name) = &inp.lock_key_name {
            if inp.foreign_witness_hex.is_some() {
                return Err(AppError::InvalidInput(
                    "a plan input cannot be both foreign and ours to sign".into(),
                ));
            }
            let key = derive_lock_key(master, network, plan.account, name)?;
            refuse_unless_cancel(plan, i, name, &key, master, network)?;
            let sig =
                tx.sign_p2wsh_input(i, &key.secret, &key.script, inp.value, inp.sighash_type)?;
            tx.inputs[i].witness = vec![sig.to_vec(), key.script.clone()];
            continue;
        }
        if inp.foreign_witness_hex.is_some() {
            continue;
        }
        let path = bip44_path(network, plan.account, inp.branch, inp.child_index);
        let child = master.derive_path(&path)?;
        let pubkey = child.compressed_pubkey();
        let hash160 = address::pubkey_to_hash160(&pubkey);
        tx.sign_p2wpkh_input(i, &child.secret, &hash160, inp.value, inp.sighash_type)?;
    }
    Ok(tx)
}

/// R17: a lock key signs price steps (`shakedex::sell::sign_step`, outside
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
fn refuse_unless_cancel(
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
    use crate::noncustodial::covenants;

    fn coin(txid_byte: u8, value: u64, child: u32) -> SpendableCoin {
        SpendableCoin {
            txid: hex::encode([txid_byte; 32]),
            vout: 0,
            value,
            branch: 0,
            child_index: child,
        }
    }

    const ADDR: &str = "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx";

    #[test]
    fn open_plan_funds_fee_and_change() {
        let nh = [1u8; 32];
        let cov = covenants::open(&nh, b"example");
        let funding = vec![coin(1, 1_000_000, 0)];
        let res = build_plan(
            Network::Main,
            0,
            None,
            PrimaryOutput {
                value: 0,
                address: ADDR.into(),
                covenant: cov,
            },
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        // OPEN output value 0 + change, funded by the one coin.
        assert_eq!(res.plan.inputs.len(), 1);
        assert!(res.fee > 0);
        // Conservation: inputs == outputs(0 + change) + fee.
        assert_eq!(res.input_total, res.change + res.fee);
        assert_eq!(res.plan.outputs[0].covenant_type, cov_type_open());
        assert!(!res.txid.is_empty());
    }

    fn cov_type_open() -> u8 {
        crate::noncustodial::sync::COV_OPEN
    }

    #[test]
    fn owner_action_keeps_name_value_and_funds_fee_separately() {
        // TRANSFER-like: name input value == output value; fee must come from
        // an extra funding coin, leaving change.
        let nh = [2u8; 32];
        let cov = covenants::transfer(&nh, 100, 0, &[9u8; 20]);
        let name = NameInputSpec {
            txid: hex::encode([0xaa; 32]),
            vout: 0,
            value: 2_000_000,
            branch: 0,
            child_index: 3,
            sighash_type: sighash::ALL,
        };
        let funding = vec![coin(1, 500_000, 1)];
        let res = build_plan(
            Network::Main,
            0,
            Some(name),
            PrimaryOutput {
                value: 2_000_000,
                address: ADDR.into(),
                covenant: cov,
            },
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        assert_eq!(res.plan.inputs.len(), 2); // name + funding
        assert_eq!(res.input_total, 2_500_000);
        // output value (2,000,000) preserved; fee+change from the 500k funding.
        assert_eq!(res.input_total, 2_000_000 + res.change + res.fee);
    }

    #[test]
    fn build_then_sign_round_trips() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        let mut session = SignerSession::unlock("p1".into(), Network::Main, master, 60_000);
        let nh = [3u8; 32];
        let res = build_plan(
            Network::Main,
            0,
            None,
            PrimaryOutput {
                value: 0,
                address: ADDR.into(),
                covenant: covenants::open(&nh, b"abc"),
            },
            &[coin(1, 1_000_000, 0)],
            ADDR,
            1,
        )
        .unwrap();
        let (signed_hex, txid) = sign_plan(&mut session, &res.plan).unwrap();
        assert!(!signed_hex.is_empty());
        // txid is the no-witness hash, identical pre/post signing.
        assert_eq!(txid, res.txid);
    }

    // --- I4: fee estimation must account for covenant output sizes ---------

    fn name_spec(value: u64) -> NameInputSpec {
        NameInputSpec {
            txid: hex::encode([0xaa; 32]),
            vout: 0,
            value,
            branch: 0,
            child_index: 3,
            sighash_type: sighash::ALL,
        }
    }

    /// A covenant-free primary output is byte-for-byte a plain P2WPKH output
    /// (32 bytes), so `build_plan`'s fee must match the flat plain-send
    /// estimator `send::estimate_fee` still used for ordinary sends. This
    /// pins the new per-output measurement against a regression in the
    /// degenerate (no covenant) case — requirement 4 (plain sends unchanged).
    #[test]
    fn empty_covenant_primary_output_matches_flat_plain_send_estimate() {
        use crate::noncustodial::send::estimate_fee;

        let funding = vec![coin(1, 1_000_000, 0)];
        let res = build_plan(
            Network::Main,
            0,
            None,
            PrimaryOutput {
                value: 100_000,
                address: ADDR.into(),
                covenant: Covenant::default(),
            },
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        let n_in = res.plan.inputs.len() as u64;
        let n_out = res.plan.outputs.len() as u64;
        assert_eq!(res.fee, estimate_fee(n_in, n_out, 1));
    }

    /// A REGISTER covenant carrying a large resource record set must be
    /// estimated (and thus priced) larger than the same REGISTER with a tiny
    /// resource, and the delta must equal EXACTLY the real serialized byte
    /// growth of the covenant output (varint length prefix + payload) — not
    /// some coarse approximation. Before the fix, both were charged the same
    /// flat per-output constant.
    #[test]
    fn register_with_large_resource_increases_fee_by_exact_encoded_len_delta() {
        let nh = [7u8; 32];
        let renewal_block = [8u8; 32];
        let small_resource = vec![0xEEu8; 4];
        let large_resource = vec![0xEEu8; 300]; // far beyond a flat P2WPKH output

        let addr = output_address_from_string(Network::Main, ADDR).unwrap();
        let small_cov = covenants::register(&nh, 100, &small_resource, &renewal_block);
        let large_cov = covenants::register(&nh, 100, &large_resource, &renewal_block);
        let small_vbytes = Output {
            value: 0,
            address: addr.clone(),
            covenant: small_cov.clone(),
        }
        .encoded_len();
        let large_vbytes = Output {
            value: 0,
            address: addr,
            covenant: large_cov.clone(),
        }
        .encoded_len();
        assert!(
            large_vbytes > small_vbytes + 250,
            "large resource must dominate the output size: small={small_vbytes} large={large_vbytes}"
        );

        let funding = vec![coin(1, 5_000_000, 1)];
        let small = build_plan(
            Network::Main,
            0,
            Some(name_spec(1_000_000)),
            PrimaryOutput {
                value: 1_000_000,
                address: ADDR.into(),
                covenant: small_cov,
            },
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        let large = build_plan(
            Network::Main,
            0,
            Some(name_spec(1_000_000)),
            PrimaryOutput {
                value: 1_000_000,
                address: ADDR.into(),
                covenant: large_cov,
            },
            &funding,
            ADDR,
            1,
        )
        .unwrap();

        assert!(
            large.fee > small.fee,
            "large={} small={}",
            large.fee,
            small.fee
        );
        assert_eq!(
            large.fee - small.fee,
            (large_vbytes - small_vbytes) as u64,
            "fee delta must equal the exact covenant-output byte delta at rate=1"
        );
    }

    /// At the relay-floor rate, a REGISTER with a large resource must pay a
    /// fee that covers at least 1 dollarydoo per byte of the ACTUAL signed
    /// broadcast size (min-relay) — not the size a flat per-output constant
    /// would have (under-)estimated. Our estimator is exact for standard
    /// P2WPKH inputs/change plus a measured covenant output, so this holds
    /// with equality, not just `>=`.
    #[test]
    fn register_plan_fee_at_rate_one_covers_actual_signed_tx_vsize() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        let mut session = SignerSession::unlock("p1".into(), Network::Main, master, 60_000);

        let nh = [9u8; 32];
        let renewal_block = [3u8; 32];
        let big_resource = vec![0x42u8; 400]; // a large resource record set
        let cov = covenants::register(&nh, 500, &big_resource, &renewal_block);

        let funding = vec![coin(1, 5_000_000, 1)];
        let res = build_plan(
            Network::Main,
            0,
            Some(name_spec(1_000_000)),
            PrimaryOutput {
                value: 1_000_000,
                address: ADDR.into(),
                covenant: cov,
            },
            &funding,
            ADDR,
            crate::noncustodial::send::MIN_FEE_RATE_PER_BYTE,
        )
        .unwrap();

        let actual_len = sign_plan_tx(&mut session, &res.plan).unwrap().vsize();

        assert!(
            res.fee >= actual_len * crate::noncustodial::send::MIN_FEE_RATE_PER_BYTE,
            "fee {} must cover the actual vsize {} at the min-relay rate",
            res.fee,
            actual_len
        );
        assert_eq!(
            res.fee, actual_len,
            "fee should exactly equal the actual signed vsize at rate=1"
        );
    }

    // --- build_finalize_with_payment_plan tests ---

    #[test]
    fn finalize_with_payment_basic_success() {
        let nh = [0xaa; 32];
        let finalize_cov = covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]);
        let name = NameInputSpec {
            txid: hex::encode([0xcc; 32]),
            vout: 0,
            value: 5_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        };
        let funding = vec![coin(1, 2_000_000, 1)];
        let res = build_finalize_with_payment_plan(
            Network::Main,
            0,
            name,
            PrimaryOutput {
                value: 5_000_000,
                address: ADDR.into(),
                covenant: finalize_cov,
            },
            ADDR.into(),
            1_000_000,
            &funding,
            ADDR,
            1,
        )
        .unwrap();

        assert_eq!(res.plan.outputs.len(), 3); // finalize + payment + change
        let total_out: u64 = res.plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(res.input_total, total_out + res.fee);
        assert_eq!(res.plan.inputs.len(), 2);
    }

    #[test]
    fn finalize_with_payment_with_change() {
        let nh = [0xaa; 32];
        let finalize_cov = covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]);
        let name = NameInputSpec {
            txid: hex::encode([0xcc; 32]),
            vout: 0,
            value: 5_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        };
        let funding = vec![coin(1, 5_000_000, 1)];
        let res = build_finalize_with_payment_plan(
            Network::Main,
            0,
            name,
            PrimaryOutput {
                value: 5_000_000,
                address: ADDR.into(),
                covenant: finalize_cov,
            },
            ADDR.into(),
            1_000_000,
            &funding,
            ADDR,
            1,
        )
        .unwrap();

        assert_eq!(res.plan.outputs.len(), 3);
        assert!(res.change > 0);
        assert_eq!(res.input_total, 10_000_000);
        assert_eq!(
            res.input_total,
            5_000_000 + 1_000_000 + res.change + res.fee
        );
    }

    #[test]
    fn finalize_with_payment_zero_value_errors() {
        let nh = [0xaa; 32];
        let finalize_cov = covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]);
        let name = NameInputSpec {
            txid: hex::encode([0xcc; 32]),
            vout: 0,
            value: 5_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        };
        let res = build_finalize_with_payment_plan(
            Network::Main,
            0,
            name,
            PrimaryOutput {
                value: 5_000_000,
                address: ADDR.into(),
                covenant: finalize_cov,
            },
            ADDR.into(),
            0,
            &[],
            ADDR,
            1,
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("non-zero"));
    }

    #[test]
    fn finalize_with_payment_insufficient_funds() {
        let nh = [0xaa; 32];
        let finalize_cov = covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]);
        let name = NameInputSpec {
            txid: hex::encode([0xcc; 32]),
            vout: 0,
            value: 100,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        };
        let res = build_finalize_with_payment_plan(
            Network::Main,
            0,
            name,
            PrimaryOutput {
                value: 100,
                address: ADDR.into(),
                covenant: finalize_cov,
            },
            ADDR.into(),
            1_000_000,
            &[],
            ADDR,
            1,
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("insufficient"));
    }

    #[test]
    fn finalize_with_payment_fee_conservation() {
        let nh = [0xaa; 32];
        let finalize_cov = covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]);
        let name = NameInputSpec {
            txid: hex::encode([0xcc; 32]),
            vout: 0,
            value: 2_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        };
        let funding = vec![coin(1, 3_000_000, 1)];
        let res = build_finalize_with_payment_plan(
            Network::Main,
            0,
            name,
            PrimaryOutput {
                value: 2_000_000,
                address: ADDR.into(),
                covenant: finalize_cov,
            },
            ADDR.into(),
            500_000,
            &funding,
            ADDR,
            1,
        )
        .unwrap();

        let total_out: u64 = res.plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(res.input_total, total_out + res.fee);
        assert_eq!(res.input_total, 5_000_000);
    }

    // --- batch finalize tests (build_batch_finalize_draft wraps build_batch_plan
    // with FINALIZE covenants; these exercise the plan-building path) ---

    #[test]
    fn batch_finalize_two_names_success() {
        let nh1 = [0x11; 32];
        let nh2 = [0x22; 32];
        // Two TRANSFER coins (owned by this wallet after lockup) → FINALIZE.
        let name_inputs = vec![
            NameInputSpec {
                txid: hex::encode([0xa1; 32]),
                vout: 0,
                value: 2_000_000,
                branch: 0,
                child_index: 0,
                sighash_type: sighash::ALL,
            },
            NameInputSpec {
                txid: hex::encode([0xa2; 32]),
                vout: 0,
                value: 3_000_000,
                branch: 0,
                child_index: 1,
                sighash_type: sighash::ALL,
            },
        ];
        let primaries = vec![
            PrimaryOutput {
                value: 2_000_000,
                address: ADDR.into(),
                covenant: covenants::finalize(&nh1, 100, &[], 0, 0, 0, &[0xbb; 32]),
            },
            PrimaryOutput {
                value: 3_000_000,
                address: ADDR.into(),
                covenant: covenants::finalize(&nh2, 100, &[], 0, 0, 0, &[0xbb; 32]),
            },
        ];
        // A small funding coin covers the fee (name values are conserved).
        let funding = vec![coin(1, 1_000_000, 2)];
        let res = build_batch_plan(
            Network::Main,
            0,
            &name_inputs,
            &primaries,
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        // 2 finalize outputs (+ change when funding leftover exceeds dust).
        assert!(res.plan.outputs.len() >= 2);
        assert!(res.fee > 0);
        let total_out: u64 = res.plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(res.input_total, total_out + res.fee);
    }

    #[test]
    fn batch_finalize_insufficient_funds_errors() {
        let nh = [0x11; 32];
        let name_inputs = vec![NameInputSpec {
            txid: hex::encode([0xa1; 32]),
            vout: 0,
            value: 2_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        }];
        let primaries = vec![PrimaryOutput {
            value: 2_000_000,
            address: ADDR.into(),
            covenant: covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]),
        }];
        // No funding at all → cannot cover the fee on top of the conserved
        // name value.
        let funding: Vec<SpendableCoin> = vec![];
        let err = build_batch_plan(
            Network::Main,
            0,
            &name_inputs,
            &primaries,
            &funding,
            ADDR,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    // --- select_funding direct tests ---

    #[test]
    fn select_funding_change_below_dust_folded_into_fee() {
        // Scenario: total_output_value = 900_000, name_value = 0, one funding
        // coin of 900_200. After fee (~200 vbytes at rate 1 = ~200 doos), the
        // leftover is below DUST_THRESHOLD → folded into fee (change = 0).
        let funding = vec![coin(1, 900_200, 0)];
        let (taken, fee, change) = select_funding(
            900_000, // total_output_value
            34,      // total_primary_vbytes (minimal p2wpkh output)
            0,       // base_in (no name input)
            0,       // name_value
            &funding, 1, // rate
        )
        .unwrap();
        assert_eq!(taken, 1);
        // Change should be 0 (folded into fee) because 200 < DUST_THRESHOLD.
        assert_eq!(change, 0);
        // Fee absorbs the entire leftover.
        assert_eq!(fee, 200);
    }

    /// A name action needing more funding coins than a standard transaction
    /// carries is refused, as a send is (`send::check_standard_weight`).
    #[test]
    fn select_funding_refuses_more_coins_than_a_standard_tx_carries() {
        let funding: Vec<SpendableCoin> = (0..3_000u32)
            .map(|i| SpendableCoin {
                txid: format!("{i:064x}"),
                vout: 0,
                value: 10_000,
                branch: 0,
                child_index: i,
            })
            .collect();
        let err = select_funding(25_000_000, 34, 0, 0, &funding, 1).unwrap_err();
        assert!(err.to_string().contains("standard"), "{err}");
        // A few coins cover a small output.
        assert!(select_funding(50_000, 34, 0, 0, &funding, 1).is_ok());
    }

    // --- Coverage-driven tests ---

    /// `outpoint_hash` rejects a txid that decodes but isn't 32 bytes.
    #[test]
    fn outpoint_hash_rejects_wrong_length() {
        // 31 bytes (62 hex chars) instead of 32.
        let short_txid = hex::encode([0xaa; 31]);
        let err = outpoint_hash(&short_txid).unwrap_err();
        assert!(err.to_string().contains("32 bytes"));
    }

    /// `build_batch_plan` rejects an empty primaries list.
    #[test]
    fn build_batch_plan_rejects_empty_primaries() {
        let name_inputs = vec![NameInputSpec {
            txid: hex::encode([0xa1; 32]),
            vout: 0,
            value: 1_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        }];
        let err = build_batch_plan(
            Network::Main,
            0,
            &name_inputs,
            &[], // empty primaries
            &[coin(1, 1_000_000, 0)],
            ADDR,
            1,
        )
        .unwrap_err();
        assert!(err.to_string().contains("at least one output"));
    }

    /// `build_batch_plan` with tight funding produces no change output
    /// (change folded into fee).
    #[test]
    fn batch_finalize_no_change_when_dust_folded_into_fee() {
        let nh = [0x11; 32];
        let name_inputs = vec![NameInputSpec {
            txid: hex::encode([0xa1; 32]),
            vout: 0,
            value: 1_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        }];
        let primaries = vec![PrimaryOutput {
            value: 1_000_000,
            address: ADDR.into(),
            covenant: covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]),
        }];
        // Funding just barely covers the fee; change will be folded into fee.
        let funding = vec![coin(1, 500, 0)];
        let res = build_batch_plan(
            Network::Main,
            0,
            &name_inputs,
            &primaries,
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        // Only the finalize output (no change output).
        assert_eq!(res.plan.outputs.len(), 1);
        assert_eq!(res.change, 0);
    }

    /// `build_finalize_with_payment_plan` with tight funding produces no change
    /// output (change folded into fee).
    #[test]
    fn finalize_with_payment_no_change_when_dust_folded_into_fee() {
        let nh = [0xaa; 32];
        let name = NameInputSpec {
            txid: hex::encode([0xcc; 32]),
            vout: 0,
            // Name value covers the finalize output, so funding only needs to
            // cover payment_value + fee.
            value: 2_000_000,
            branch: 0,
            child_index: 0,
            sighash_type: sighash::ALL,
        };
        // payment_value = 1_000. With 2 inputs (name + 1 funding), 2 primary
        // outputs (finalize + payment), the fee is ~350 doos at rate 1.
        // Funding of 1_500 covers payment + fee with leftover < DUST_THRESHOLD.
        let funding = vec![coin(1, 1_500, 1)];
        let res = build_finalize_with_payment_plan(
            Network::Main,
            0,
            name,
            PrimaryOutput {
                value: 2_000_000,
                address: ADDR.into(),
                covenant: covenants::finalize(&nh, 100, &[], 0, 0, 0, &[0xbb; 32]),
            },
            ADDR.into(),
            1_000,
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        // Finalize + payment outputs, no change.
        assert_eq!(res.plan.outputs.len(), 2);
        assert_eq!(res.change, 0);
    }

    #[test]
    fn old_plan_json_without_new_fields_still_parses() {
        let json = r#"{"version":0,"locktime":0,"account":0,"network":"main",
            "inputs":[{"txid":"0101010101010101010101010101010101010101010101010101010101010101",
                       "vout":0,"value":5000,"branch":0,"child_index":3,"sighash_type":1}],
            "outputs":[]}"#;
        let plan: DraftPlan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.inputs[0].sequence, FINAL_SEQUENCE);
        assert!(plan.inputs[0].foreign_witness_hex.is_none());
        assert!(plan.inputs[0].lock_key_name.is_none());
        assert!(!plan.has_foreign_or_custom_inputs());
    }

    /// A plan stored before `lock_key_name` existed signs to the same bytes it
    /// signed to before (hex pinned from the signer at c25b655).
    #[test]
    fn old_plan_json_signs_as_before() {
        let json = r#"{"version":0,"locktime":0,"account":0,"network":"main",
            "inputs":[{"txid":"0101010101010101010101010101010101010101010101010101010101010101",
                       "vout":0,"value":5000,"branch":0,"child_index":3,"sighash_type":1}],
            "outputs":[{"value":4000,"address":"hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx",
                        "covenant_type":0,"covenant_items_hex":[]}],
            "change_output_index":null}"#;
        let plan: DraftPlan = serde_json::from_str(json).unwrap();
        assert_eq!(
            plan.own_inputs(),
            vec![(plan.inputs[0].txid.clone(), plan.inputs[0].vout)]
        );
        let mut session = SignerSession::unlock("p1".into(), Network::Main, seed_master(), 60_000);
        let (hex, txid) = sign_plan(&mut session, &plan).unwrap();
        assert_eq!(
            hex,
            "0000000001010101010101010101010101010101010101010101010101010101\
             010101010100000000ffffffff01a00f00000000000000146d5571fdbca1019c\
             d0f0cd792d1b0bdfa7651c7e00000000000002411861b40c9d4b662be02c9226\
             cb8aefb7723e9a75f05e85d3629864d5af550f4a33da353dcc46b3390e66b2ca\
             19c9f9d07031009c328c4e331a3383f9c0d0a9d7012103949384d01f9fd552ec\
             a60eb2bf942b8c547b112a195b5494c5a27fe93953fc5c"
        );
        assert_eq!(
            txid,
            "b71854753e90aa5fd7466fc3601c5e431ab16d41bcfc67ebec0b6d56ae6315e1"
        );
    }

    fn plan_with_foreign_input(witness: Vec<String>) -> DraftPlan {
        let mut plan = build_plan(
            Network::Main,
            0,
            None,
            PrimaryOutput {
                value: 100_000,
                address: ADDR.into(),
                covenant: Covenant::default(),
            },
            &[coin(1, 1_000_000, 0)],
            ADDR,
            1,
        )
        .unwrap()
        .plan;
        plan.inputs.insert(
            0,
            PlanInput {
                txid: hex::encode([9u8; 32]),
                vout: 1,
                value: 0,
                branch: 0,
                child_index: 0,
                sighash_type: 0x84,
                sequence: 0xffff_fffe,
                foreign_witness_hex: Some(witness),
                lock_key_name: None,
            },
        );
        plan
    }

    #[test]
    fn own_inputs_leave_out_the_foreign_ones() {
        let plan = plan_with_foreign_input(vec!["aa".into()]);
        // Input 0 is the seller's coin; only input 1 is ours to reserve.
        let own = plan.own_inputs();
        assert_eq!(
            own,
            vec![(plan.inputs[1].txid.clone(), plan.inputs[1].vout)]
        );
    }

    #[test]
    fn rebuild_applies_sequence_locktime_and_foreign_witness() {
        let mut plan = plan_with_foreign_input(vec!["aa".into(), "bbcc".into()]);
        plan.locktime = 0x8000_1234;
        assert!(plan.has_foreign_or_custom_inputs());
        let tx = rebuild_unsigned(&plan, Network::Main).unwrap();
        assert_eq!(tx.locktime, 0x8000_1234);
        assert_eq!(tx.inputs[0].sequence, 0xffff_fffe);
        assert_eq!(tx.inputs[0].witness, vec![vec![0xaa], vec![0xbb, 0xcc]]);
        assert_eq!(tx.inputs[1].sequence, FINAL_SEQUENCE);
        assert!(tx.inputs[1].witness.is_empty());
    }

    #[test]
    fn sign_plan_leaves_foreign_inputs_untouched() {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        let mut session = SignerSession::unlock("p1".into(), Network::Main, master, 60_000);
        let plan = plan_with_foreign_input(vec!["aa".into()]);
        let tx = sign_plan_tx(&mut session, &plan).unwrap();
        assert_eq!(tx.inputs[0].witness, vec![vec![0xaa]]);
        assert_eq!(tx.inputs[1].witness.len(), 2, "own input signed as P2WPKH");
    }

    fn seed_master() -> ExtendedPrivKey {
        ExtendedPrivKey::from_seed(&[7u8; 64]).unwrap()
    }

    /// The key hash of our address at m/44'/5353'/0'/`branch`/`child` under
    /// [`seed_master`].
    fn our_hash_at(branch: u32, child: u32) -> [u8; 20] {
        let path = bip44_path(Network::Main, 0, branch, child);
        address::pubkey_to_hash160(
            &seed_master()
                .derive_path(&path)
                .unwrap()
                .compressed_pubkey(),
        )
    }

    /// The fixture's cancel commits the name to our address at 0/11.
    fn our_cancel_hash() -> [u8; 20] {
        our_hash_at(0, 11)
    }

    /// `plan`'s lock input names `branch`/`child` and its TRANSFER commits to
    /// our address there, so only the path rule can refuse it.
    fn commit_to_path(mut plan: DraftPlan, branch: u32, child: u32) -> DraftPlan {
        plan.inputs[0].branch = branch;
        plan.inputs[0].child_index = child;
        plan.outputs[0].covenant_items_hex[3] = hex::encode(our_hash_at(branch, child));
        plan
    }

    /// Input 0 spends `key`'s lock coin into output 0, a TRANSFER of
    /// "dexreviews" paying `transfer_to` and committing to our address at
    /// branch 0, index 11 (the input's path); input 1 is one of our coins.
    fn plan_with_lock_key_input(sighash_type: u32, transfer_to: &str) -> DraftPlan {
        let nh = crate::noncustodial::names::hash_name("dexreviews").unwrap();
        let transfer = covenants::transfer(&nh, 120, 0, &our_cancel_hash());
        let mut plan = build_plan(
            Network::Main,
            0,
            None,
            PrimaryOutput {
                value: 100_000,
                address: ADDR.into(),
                covenant: Covenant::default(),
            },
            &[coin(1, 1_000_000, 0)],
            ADDR,
            1,
        )
        .unwrap()
        .plan;
        plan.inputs.insert(
            0,
            PlanInput {
                txid: hex::encode([9u8; 32]),
                vout: 0,
                value: 1_000_000,
                branch: 0,
                child_index: 11,
                sighash_type,
                sequence: FINAL_SEQUENCE,
                foreign_witness_hex: None,
                lock_key_name: Some("dexreviews".into()),
            },
        );
        plan.outputs.insert(
            0,
            PlanOutput {
                value: 1_000_000,
                address: transfer_to.into(),
                covenant_type: transfer.covenant_type,
                covenant_items_hex: transfer.items.iter().map(hex::encode).collect(),
            },
        );
        plan.change_output_index = plan.change_output_index.map(|i| i + 1);
        plan
    }

    fn lock_key() -> LockKey {
        derive_lock_key(&seed_master(), Network::Main, 0, "dexreviews").unwrap()
    }

    /// The signer derives the lock key from the seed by the input's name and
    /// signs the cancel's lock input 0x83 with witness [signature, script].
    #[test]
    fn sign_plan_signs_a_lock_key_input_with_the_lock_key() {
        let key = lock_key();
        let plan = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        let unsigned = rebuild_unsigned(&plan, Network::Main).unwrap();
        let mut session = SignerSession::unlock("p1".into(), Network::Main, seed_master(), 60_000);
        let tx = sign_plan_tx(&mut session, &plan).unwrap();
        let [sig, script] = tx.inputs[0].witness.as_slice() else {
            panic!("witness is [signature, script]: {:?}", tx.inputs[0].witness);
        };
        assert_eq!(script, &key.script);
        assert_eq!(sig.len(), 65);
        assert_eq!(u32::from(sig[64]), CANCEL_SIGHASH);
        let digest = unsigned
            .signature_hash(0, &key.script, 1_000_000, CANCEL_SIGHASH)
            .unwrap();
        secp256k1::Secp256k1::verification_only()
            .verify_ecdsa(
                &secp256k1::Message::from_digest(digest),
                &secp256k1::ecdsa::Signature::from_compact(&sig[..64]).unwrap(),
                &secp256k1::PublicKey::from_slice(&key.pubkey).unwrap(),
            )
            .unwrap();
        assert_eq!(tx.inputs[1].witness.len(), 2, "own input signed as P2WPKH");
    }

    /// A lock coin is not a tracked coin a draft can reserve, and a Ledger
    /// cannot sign it.
    #[test]
    fn own_inputs_leave_out_a_lock_key_input_and_the_ledger_refuses_it() {
        let plan = plan_with_lock_key_input(CANCEL_SIGHASH, &lock_key().address);
        assert_eq!(
            plan.own_inputs(),
            vec![(plan.inputs[1].txid.clone(), plan.inputs[1].vout)]
        );
        assert!(plan.has_foreign_or_custom_inputs());
    }

    /// R17: a lock key signs only price steps (outside any plan) and
    /// cancels. Any other sighash, an output at the input's index that is
    /// missing, not a TRANSFER, not at the lock address, a TRANSFER of
    /// another name or to an address that is not the input's path of ours,
    /// or an input that is also foreign, is refused before anything is
    /// signed.
    #[test]
    fn sign_plan_refuses_a_lock_key_input_that_is_not_a_cancel() {
        let key = lock_key();
        let other = derive_lock_key(&seed_master(), Network::Main, 0, "namehold").unwrap();
        let mut not_transfer = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        not_transfer.outputs[0].covenant_type = crate::noncustodial::sync::COV_FINALIZE;
        let mut no_output = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        no_output.outputs.clear();
        no_output.change_output_index = None;
        let mut also_foreign = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        also_foreign.inputs[0].foreign_witness_hex = Some(vec!["aa".into()]);
        let mut other_name = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        other_name.outputs[0].covenant_items_hex[0] =
            hex::encode(crate::noncustodial::names::hash_name("namehold").unwrap());
        let mut foreign_dest = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        foreign_dest.outputs[0].covenant_items_hex[3] = hex::encode([7u8; 20]);
        let mut other_path = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        other_path.inputs[0].child_index = 12;
        let mut script_dest = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        script_dest.outputs[0].covenant_items_hex[2] = "01".into();
        let mut short_transfer = plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        short_transfer.outputs[0].covenant_items_hex.truncate(2);
        let cancel = || plan_with_lock_key_input(CANCEL_SIGHASH, &key.address);
        let change_branch = commit_to_path(cancel(), 1, 11);
        let hardened_index = commit_to_path(cancel(), 0, HARDENED_OFFSET + 11);
        let mut not_final = cancel();
        not_final.inputs[0].sequence = 0xffff_fffe;
        let mut lock_time = cancel();
        lock_time.locktime = 500_000_000;
        for (case, plan) in [
            ("price step", plan_with_lock_key_input(0x84, &key.address)),
            ("ALL", plan_with_lock_key_input(sighash::ALL, &key.address)),
            (
                "another lock",
                plan_with_lock_key_input(CANCEL_SIGHASH, &other.address),
            ),
            ("not a TRANSFER", not_transfer),
            ("no output", no_output),
            ("also foreign", also_foreign),
            ("another name", other_name),
            ("a foreign destination", foreign_dest),
            ("a destination off the input's path", other_path),
            ("destination version 1", script_dest),
            ("a TRANSFER without its address items", short_transfer),
            ("a change-branch destination", change_branch),
            ("a hardened destination index", hardened_index),
            ("a non-final sequence", not_final),
            ("a lock time", lock_time),
        ] {
            let mut session =
                SignerSession::unlock("p1".into(), Network::Main, seed_master(), 60_000);
            let Err(err) = sign_plan_tx(&mut session, &plan) else {
                panic!("{case}: signed");
            };
            assert!(matches!(err, AppError::InvalidInput(_)), "{case}: {err:?}");
        }
    }
}
