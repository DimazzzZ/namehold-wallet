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
use crate::noncustodial::send::{estimate_fee_with_primary, SpendableCoin, DUST_THRESHOLD};
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
    /// Zero-based index of the change output (if any). Used by Ledger to skip
    /// prompting the user for change verification.
    #[serde(default)]
    pub change_output_index: Option<usize>,
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
        change_output_index: if change > 0 { Some(1) } else { None },
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
        change_output_index: if change > 0 { Some(1) } else { None },
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
    }];
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
        change_output_index: if change > 0 { Some(1) } else { None },
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
        tx.inputs.push(Input::new(Outpoint {
            hash: outpoint_hash(&inp.txid)?,
            index: inp.vout,
        }));
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
    fn register_plan_fee_at_rate_one_covers_actual_signed_tx_size() {
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

        let (signed_hex, _txid) = sign_plan(&mut session, &res.plan).unwrap();
        let actual_len = hex::decode(&signed_hex).unwrap().len() as u64;

        assert!(
            res.fee >= actual_len * crate::noncustodial::send::MIN_FEE_RATE_PER_BYTE,
            "fee {} must cover the actual size {} at the min-relay rate",
            res.fee,
            actual_len
        );
        assert_eq!(
            res.fee, actual_len,
            "fee should exactly equal the actual signed size at rate=1"
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
}
