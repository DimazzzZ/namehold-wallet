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
use crate::noncustodial::hd::bip44_path;
use crate::noncustodial::network::Network;
use crate::noncustodial::send::{estimate_fee, SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::tx::{
    output_address_from_string, sighash, Covenant, Input, Outpoint, Output, Transaction,
};

/// One input of a draft plan: prevout + the derivation path needed to re-sign.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanInput {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub branch: u32,
    pub child_index: u32,
    pub sighash_type: u32,
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
pub struct PrimaryOutput {
    pub value: u64,
    pub address: String,
    pub covenant: Covenant,
}

/// Result of planning: the plan plus a preview (unsigned hex, txid, fee/change).
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

    let mut taken = 0usize; // funding coins used
    let (fee, change) = loop {
        let funded: u64 = funding[..taken].iter().map(|c| c.value).sum();
        let total_in = name_value + funded;
        let n_in = base_in + taken as u64;

        if n_in >= 1 {
            let fee_wc = estimate_fee(n_in, 2, rate);
            let fee_nc = estimate_fee(n_in, 1, rate);
            if total_in >= primary.value + fee_wc {
                let change = total_in - primary.value - fee_wc;
                if change >= DUST_THRESHOLD {
                    break (fee_wc, change);
                }
                break (total_in - primary.value, 0); // fold dust into fee
            }
            if total_in >= primary.value + fee_nc {
                break (total_in - primary.value, 0);
            }
        }
        if taken >= funding.len() {
            return Err(AppError::InvalidInput(
                "insufficient funds to cover output and fee".into(),
            ));
        }
        taken += 1;
    };

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
        });
    }

    // Outputs: covenant output, then change (plain) if any.
    let mut plan_outputs = vec![PlanOutput {
        value: primary.value,
        address: primary.address.clone(),
        covenant_type: primary.covenant.covenant_type,
        covenant_items_hex: primary.covenant.items.iter().map(hex::encode).collect(),
    }];
    if change > 0 {
        plan_outputs.push(PlanOutput {
            value: change,
            address: change_address.to_string(),
            covenant_type: 0,
            covenant_items_hex: Vec::new(),
        });
    }

    let plan = DraftPlan {
        version: 0,
        locktime: 0,
        account,
        network: network.as_str().to_string(),
        inputs: plan_inputs,
        outputs: plan_outputs,
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

/// Reconstruct the unsigned [`Transaction`] from a plan (no witnesses).
fn rebuild_unsigned(plan: &DraftPlan, network: Network) -> Result<Transaction, AppError> {
    let mut tx = Transaction::new();
    tx.version = plan.version;
    tx.locktime = plan.locktime;
    for inp in &plan.inputs {
        tx.inputs.push(Input::new(Outpoint {
            hash: outpoint_hash(&inp.txid)?,
            index: inp.vout,
        }));
    }
    for out in &plan.outputs {
        let items = out
            .covenant_items_hex
            .iter()
            .map(|h| hex::decode(h).map_err(|e| AppError::InvalidInput(format!("bad covenant item: {e}"))))
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
    let network = Network::from_str_opt(&plan.network)
        .ok_or_else(|| AppError::InvalidInput(format!("bad network '{}'", plan.network)))?;
    let mut tx = rebuild_unsigned(plan, network)?;
    let master = session.master()?;
    for (i, inp) in plan.inputs.iter().enumerate() {
        let path = bip44_path(network, plan.account, inp.branch, inp.child_index);
        let child = master.derive_path(&path)?;
        let pubkey = child.compressed_pubkey();
        let hash160 = address::pubkey_to_hash160(&pubkey);
        tx.sign_p2wpkh_input(i, &child.secret, &hash160, inp.value, inp.sighash_type)?;
    }
    Ok((tx.to_hex(), tx.txid()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::covenants;
    use crate::noncustodial::hd::ExtendedPrivKey;

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
            PrimaryOutput { value: 0, address: ADDR.into(), covenant: cov },
            &funding,
            ADDR,
            1,
        )
        .unwrap();
        // OPEN output value 0 + change, funded by the one coin.
        assert_eq!(res.plan.inputs.len(), 1);
        assert!(res.fee > 0);
        // Conservation: inputs == outputs(0 + change) + fee.
        assert_eq!(res.input_total, 0 + res.change + res.fee);
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
            PrimaryOutput { value: 2_000_000, address: ADDR.into(), covenant: cov },
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
            PrimaryOutput { value: 0, address: ADDR.into(), covenant: covenants::open(&nh, b"abc") },
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
}
