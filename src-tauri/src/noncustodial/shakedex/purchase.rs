//! Purchase: `[lock coin (seller-signed), our funding...] ->
//! [TRANSFER to us at the lock address, market fee?, change?, payment]`.
//! The payment must be the last output: 0x84 at input 0 commits to it.
//! Finalize out of the lock: `[transfer coin ([lockScript] witness), our
//! funding...] -> [FINALIZE to us, change?]`.

use crate::error::AppError;
use crate::noncustodial::actions::{
    rebuild_unsigned, DraftPlan, PlanInput, PlanOutput, PlanResult, FINAL_SEQUENCE,
};
use crate::noncustodial::address;
use crate::noncustodial::covenants;
use crate::noncustodial::names;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{NodeCoin, NodeCovenant};
use crate::noncustodial::send::{SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::shakedex::listing_file::ListingFile;
use crate::noncustodial::shakedex::script::{lock_address, lock_script};
use crate::noncustodial::shakedex::template::{encode_lock_time, STEP_SEQUENCE, STEP_SIGHASH};
use crate::noncustodial::sync::COV_TRANSFER;
use crate::noncustodial::tx::{sighash, Covenant, OutputAddress};
use crate::noncustodial::types::doos_to_hns_string;

/// `wallet_tx_drafts.action` of a purchase draft.
pub const PURCHASE_ACTION: &str = "shakedex_purchase";
/// `wallet_tx_drafts.action` of a draft finalizing a purchased name.
pub const PURCHASE_FINALIZE_ACTION: &str = "shakedex_purchase_finalize";

/// Fee floor for Shakedex transactions, in doos per virtual byte: the
/// 5000 doos/kB floor shakedex itself uses (spec R4).
pub const SHAKEDEX_MIN_RATE_PER_BYTE: u64 = 5;
/// Handshake's money supply cap in dollarydoos: hsd 8.0.0
/// lib/protocol/consensus.js `MAX_MONEY = 2.04e9 * COIN` with `COIN = 10^6`.
pub const MAX_MONEY: u64 = 2_040_000_000_000_000;
/// Witness item sizes of a signed P2WPKH input: signature + sighash byte,
/// compressed public key.
const DUMMY_SIG_AND_PUBKEY: [usize; 2] = [65, 33];

/// The summary a purchase draft stores (`wallet_tx_drafts.summary_json`):
/// what the purchase dialog, the draft list and the secure confirmation
/// window read. Every field is required when it is read back, so a summary
/// missing one is refused rather than shown as zero.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseSummary {
    pub action: String,
    pub name: String,
    pub price_doos: u64,
    /// 0 when no market fee is paid.
    pub market_fee_doos: u64,
    /// `None` when no market fee is paid.
    pub market_fee_address: Option<String>,
    pub fee_doos: u64,
    pub total_doos: u64,
    pub send_total_doos: u64,
    pub change_doos: u64,
    /// Our own coins only: the lock coin carries the name's value, which
    /// comes straight back in the TRANSFER output and is not money spent.
    pub input_total_doos: u64,
    pub num_inputs: usize,
    pub recipient_address: String,
    pub payment_address: String,
    pub destination_address: String,
    /// [`finalize_wait_text`] for the profile's network.
    pub finalize_wait: String,
    pub txid: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// The summary a draft finalizing a purchased name stores; see
/// [`PurchaseSummary`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseFinalizeSummary {
    pub action: String,
    pub name: String,
    pub purchase_id: String,
    pub send_total_doos: u64,
    pub fee_doos: u64,
    pub total_doos: u64,
    pub change_doos: u64,
    /// Our own coins only: the TRANSFER's value is the name's own and comes
    /// straight back in the FINALIZE output.
    pub input_total_doos: u64,
    pub num_inputs: usize,
    pub recipient_address: String,
    pub destination_address: String,
    pub txid: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MarketFee {
    pub address: OutputAddress,
    pub value: u64,
}

pub struct PurchaseInput<'a> {
    pub network: Network,
    pub account: u32,
    pub listing: &'a ListingFile,
    pub step: usize,
    pub lock_value: u64,
    pub name_height: u32,
    pub dest: OutputAddress,
    pub market_fee: Option<MarketFee>,
    pub funding: &'a [SpendableCoin],
    pub change_address: &'a str,
    pub rate: u64,
    #[cfg(test)]
    pub fixed_fee: Option<u64>,
}

#[cfg(test)]
impl PurchaseInput<'_> {
    /// Pin the fee instead of sizing it, to reproduce a vector's fee.
    pub fn with_fixed_fee_for_tests(mut self, fee: u64) -> Self {
        self.fixed_fee = Some(fee);
        self
    }
}

pub struct FinalizeInput<'a> {
    pub network: Network,
    pub account: u32,
    pub transfer_outpoint: ([u8; 32], u32),
    pub transfer_value: u64,
    pub lock_pubkey: [u8; 33],
    pub name: &'a str,
    pub name_height: u32,
    pub weak: bool,
    pub claimed: u32,
    pub renewals: u32,
    pub renewal_block: [u8; 32],
    pub dest_address: &'a str,
    pub funding: &'a [SpendableCoin],
    pub change_address: &'a str,
    pub rate: u64,
    #[cfg(test)]
    pub fixed_fee: Option<u64>,
}

#[cfg(test)]
impl FinalizeInput<'_> {
    /// Pin the fee instead of sizing it, to reproduce a vector's fee.
    pub fn with_fixed_fee_for_tests(mut self, fee: u64) -> Self {
        self.fixed_fee = Some(fee);
        self
    }
}

fn addr_string(network: Network, a: &OutputAddress) -> Result<String, AppError> {
    if a.version != 0 {
        return Err(AppError::InvalidInput(format!(
            "unsupported address version {}",
            a.version
        )));
    }
    // Only the market fee reaches here, and `fee_output_address` admits only
    // a 20-byte program.
    if let Ok(hash) = <[u8; 20]>::try_from(a.hash.as_slice()) {
        return address::encode_p2wpkh(network, &hash);
    }
    Err(AppError::InvalidInput(format!(
        "unsupported address program length {}",
        a.hash.len()
    )))
}

fn plain(value: u64, address: String) -> PlanOutput {
    PlanOutput {
        value,
        address,
        covenant_type: 0,
        covenant_items_hex: vec![],
    }
}

fn cov_out(value: u64, address: String, c: &Covenant) -> PlanOutput {
    PlanOutput {
        value,
        address,
        covenant_type: c.covenant_type,
        covenant_items_hex: c.items.iter().map(hex::encode).collect(),
    }
}

fn own_input(c: &SpendableCoin) -> PlanInput {
    PlanInput {
        txid: c.txid.clone(),
        vout: c.vout,
        value: c.value,
        branch: c.branch,
        child_index: c.child_index,
        sighash_type: sighash::ALL,
        sequence: FINAL_SEQUENCE,
        foreign_witness_hex: None,
    }
}

/// Exact vsize of `plan` once our inputs carry P2WPKH witnesses; foreign
/// inputs already carry their finished witnesses.
fn signed_vsize(plan: &DraftPlan, network: Network) -> Result<u64, AppError> {
    let mut tx = rebuild_unsigned(plan, network)?;
    for (i, inp) in plan.inputs.iter().enumerate() {
        if inp.foreign_witness_hex.is_none() {
            tx.inputs[i].witness = DUMMY_SIG_AND_PUBKEY.iter().map(|n| vec![0u8; *n]).collect();
        }
    }
    Ok(tx.vsize())
}

fn overflow() -> AppError {
    AppError::InvalidInput("amounts overflow: this purchase cannot be built".into())
}

/// Sum money values, refusing (never wrapping or panicking) on overflow.
fn checked_sum(values: impl IntoIterator<Item = u64>) -> Result<u64, AppError> {
    values
        .into_iter()
        .try_fold(0u64, |acc, v| acc.checked_add(v))
        .ok_or_else(overflow)
}

/// The encoded lock time a purchase draft's plan carries: that of the price
/// step it pays, which tells the step apart from others at the same price.
pub fn plan_lock_time(plan_json: &str) -> Result<u32, AppError> {
    serde_json::from_str::<crate::noncustodial::actions::DraftPlan>(plan_json)
        .map(|plan| plan.locktime)
        .map_err(|e| {
            AppError::InvalidInput(format!(
                "the purchase's plan is unreadable ({e}), so the price could not be re-checked; \
                 the purchase was not sent"
            ))
        })
}

/// Choose funding in the order given (callers pass load_spendable_coins
/// order, largest-first) until `spend + fee` is covered, sizing the
/// fee on the real transaction. `before_change`/`after_change` are the
/// outputs on either side of the change slot.
#[allow(clippy::too_many_arguments)]
fn fund(
    network: Network,
    account: u32,
    locktime: u32,
    foreign: PlanInput,
    foreign_value: u64,
    before_change: Vec<PlanOutput>,
    after_change: Vec<PlanOutput>,
    funding: &[SpendableCoin],
    change_address: &str,
    rate: u64,
    fixed_fee: Option<u64>,
) -> Result<PlanResult, AppError> {
    let rate = rate.max(SHAKEDEX_MIN_RATE_PER_BYTE);
    let out_total = checked_sum(
        before_change
            .iter()
            .chain(after_change.iter())
            .map(|o| o.value),
    )?;
    let have = checked_sum(funding.iter().map(|c| c.value))?;
    for taken in 0..=funding.len() {
        let mut inputs = vec![foreign.clone()];
        inputs.extend(funding[..taken].iter().map(own_input));
        let in_total = checked_sum(
            std::iter::once(foreign_value).chain(funding[..taken].iter().map(|c| c.value)),
        )?;
        for with_change in [true, false] {
            let mut outputs = before_change.clone();
            let change_index = with_change.then_some(outputs.len());
            if with_change {
                outputs.push(plain(0, change_address.to_owned()));
            }
            outputs.extend(after_change.iter().cloned());
            let mut plan = DraftPlan {
                version: 0,
                locktime,
                account,
                network: network.as_str().into(),
                inputs: inputs.clone(),
                outputs,
                change_output_index: change_index,
            };
            let fee = match fixed_fee {
                Some(f) => f,
                None => signed_vsize(&plan, network)?
                    .checked_mul(rate)
                    .ok_or_else(overflow)?,
            };
            let spend = out_total.checked_add(fee).ok_or_else(overflow)?;
            let Some(rest) = in_total.checked_sub(spend) else {
                continue;
            };
            if let Some(idx) = change_index {
                if rest < DUST_THRESHOLD {
                    continue;
                }
                plan.outputs[idx].value = rest;
            }
            let (fee, change) = if with_change {
                (fee, rest)
            } else {
                // rest < in_total - spend, so fee + rest <= in_total.
                (fee + rest, 0)
            };
            let tx = rebuild_unsigned(&plan, network)?;
            return Ok(PlanResult {
                unsigned_tx_hex: tx.to_hex(),
                txid: tx.txid(),
                fee,
                change,
                input_total: in_total,
                plan,
            });
        }
    }
    // The TRANSFER/FINALIZE output carries the foreign coin's value back,
    // so what the buyer adds is everything else: price and market fee.
    let need = out_total.saturating_sub(foreign_value);
    Err(AppError::InvalidInput(format!(
        "not enough HNS: need {} plus fees, have {}",
        doos_to_hns_string(need),
        doos_to_hns_string(have)
    )))
}

/// Buy the name at price step `p.step`: spend the seller's lock coin with
/// its signed step and pay from our funding coins.
pub fn build_purchase_plan(p: &PurchaseInput) -> Result<PlanResult, AppError> {
    let step = p
        .listing
        .steps
        .get(p.step)
        .ok_or_else(|| AppError::InvalidInput("no such price step".into()))?;
    // The price and market fee come from the seller and the market, so
    // bound them before any arithmetic: nothing above the money supply.
    let market_fee = p.market_fee.as_ref().map_or(0, |f| f.value);
    match step.price.checked_add(market_fee) {
        Some(total) if total <= MAX_MONEY => {}
        _ => {
            return Err(AppError::InvalidInput(format!(
                "price plus market fee exceeds the {} HNS money supply",
                MAX_MONEY / 1_000_000
            )))
        }
    }
    let nh = names::hash_name(&p.listing.name)?;
    let script = lock_script(&p.listing.public_key);
    let lock_addr = lock_address(p.network, &p.listing.public_key)?;
    let foreign = PlanInput {
        txid: hex::encode(p.listing.lock_txid),
        vout: p.listing.lock_vout,
        value: p.lock_value,
        branch: 0,
        child_index: 0,
        sighash_type: STEP_SIGHASH,
        sequence: STEP_SEQUENCE,
        foreign_witness_hex: Some(vec![hex::encode(step.signature), hex::encode(&script)]),
    };
    let transfer = covenants::transfer(&nh, p.name_height, p.dest.version, &p.dest.hash);
    let mut before = vec![cov_out(p.lock_value, lock_addr, &transfer)];
    if let Some(fee) = &p.market_fee {
        before.push(plain(fee.value, addr_string(p.network, &fee.address)?));
    }
    let after = vec![plain(step.price, p.listing.payment_addr.clone())];
    #[cfg(test)]
    let fixed = p.fixed_fee;
    #[cfg(not(test))]
    let fixed = None;
    fund(
        p.network,
        p.account,
        encode_lock_time(step.lock_time)?,
        foreign,
        p.lock_value,
        before,
        after,
        p.funding,
        p.change_address,
        p.rate,
        fixed,
    )
}

/// Finalize the purchased name out of the lock to `f.dest_address`.
pub fn build_purchase_finalize_plan(f: &FinalizeInput) -> Result<PlanResult, AppError> {
    let nh = names::hash_name(f.name)?;
    let foreign = PlanInput {
        txid: hex::encode(f.transfer_outpoint.0),
        vout: f.transfer_outpoint.1,
        value: f.transfer_value,
        branch: 0,
        child_index: 0,
        sighash_type: sighash::ALL,
        sequence: FINAL_SEQUENCE,
        foreign_witness_hex: Some(vec![hex::encode(lock_script(&f.lock_pubkey))]),
    };
    let fin = covenants::finalize(
        &nh,
        f.name_height,
        f.name.as_bytes(),
        u8::from(f.weak),
        f.claimed,
        f.renewals,
        &f.renewal_block,
    );
    let before = vec![cov_out(f.transfer_value, f.dest_address.to_owned(), &fin)];
    #[cfg(test)]
    let fixed = f.fixed_fee;
    #[cfg(not(test))]
    let fixed = None;
    fund(
        f.network,
        f.account,
        0,
        foreign,
        f.transfer_value,
        before,
        vec![],
        f.funding,
        f.change_address,
        f.rate,
        fixed,
    )
}

/// Whether `coin` is a TRANSFER whose covenant (items 2–3: address version
/// and hash) commits to `destination` — the check that a purchase moves the
/// name to us. `coin` is the purchase's own output 0, which its txid commits
/// to being a TRANSFER: a reply that leaves out the covenant, reports another
/// type, or a TRANSFER without both items, is an error, not a "no" — hsd
/// always sends them, and a "no" marks a purchase lost (R13).
pub fn transfer_commits_to(
    coin: &NodeCoin,
    network: Network,
    destination: &str,
) -> Result<bool, AppError> {
    let cov = coin
        .covenant
        .as_ref()
        .ok_or_else(|| AppError::Rpc("node did not report the transfer coin's covenant".into()))?;
    let (version, hash) = address::decode(network, destination)?;
    if cov.kind != COV_TRANSFER {
        return Err(AppError::Rpc(format!(
            "node reported covenant type {} for the purchase's TRANSFER",
            cov.kind
        )));
    }
    let [_, _, cov_version, cov_hash, ..] = cov.items.as_slice() else {
        return Err(AppError::Rpc(
            "node did not report the transfer covenant's address".into(),
        ));
    };
    Ok(cov_version.eq_ignore_ascii_case(&hex::encode([version]))
        && cov_hash.eq_ignore_ascii_case(&hex::encode(hash)))
}

/// The name height a covenant commits to (item 1), as hsd writes it: 4
/// little-endian bytes in hex. `None` for anything else, which is not hsd's
/// reply and proves nothing.
pub fn covenant_name_height(cov: &NodeCovenant) -> Option<u32> {
    cov.items
        .get(1)
        .and_then(|h| hex::decode(h).ok())
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_le_bytes)
}

/// The market fee as basis points of the price, for display. `None` when
/// there is no price or the figure does not fit a u32 (a fee over 429,496
/// times the price, which a foreign listing file may still carry): such a
/// fee is shown without a percentage, never as a wrapped small one.
pub fn fee_percent_bps(fee: u64, price: u64) -> Option<u32> {
    if price == 0 {
        return None;
    }
    u32::try_from(u128::from(fee) * 10_000 / u128::from(price)).ok()
}

/// "1.99%": the fee's share of the price in exact hundredths of a percent,
/// as both the purchase dialog and the secure window show it. `None` where
/// [`fee_percent_bps`] has no figure.
pub fn fee_percent_text(fee: u64, price: u64) -> Option<String> {
    fee_percent_bps(fee, price).map(|bps| format!("{}.{:02}%", bps / 100, bps % 100))
}

/// "after a finalize, 288 blocks (about 2 days) after the purchase is mined":
/// when a purchased name becomes the buyer's on `network`, as both the
/// purchase dialog and the secure window say it. The time is given only where
/// miners produce blocks on schedule
/// ([`Network::has_wall_clock_block_timing`]).
pub fn finalize_wait_text(network: Network) -> String {
    let blocks = network.name_params().transfer_lockup;
    if !network.has_wall_clock_block_timing() {
        return format!("after a finalize, {blocks} blocks after the purchase is mined");
    }
    let mins = u64::from(blocks) * crate::noncustodial::network::TARGET_SPACING_SECS / 60;
    let approx = if mins < 60 {
        format!("{mins} minutes")
    } else if mins < 48 * 60 {
        format!("{} hours", (mins + 30) / 60)
    } else {
        format!("{} days", (mins + 720) / 1440)
    };
    format!("after a finalize, {blocks} blocks (about {approx}) after the purchase is mined")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::{self, ExtendedPrivKey};
    use crate::noncustodial::shakedex::listing_file::PriceStep;
    use crate::noncustodial::sync::{COV_FINALIZE, COV_TRANSFER};
    use crate::noncustodial::tx::Transaction;

    const PAYMENT: &str = "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx";
    const CHANGE: &str = "hs1qdhtaj7ws7chd2z2tulrmakqww428myx08d6w3v";
    const PUBKEY: [u8; 33] = [2u8; 33];
    const PRICE: u64 = 250_000_000;
    const LOCK_TIME: u64 = 1_783_696_480;
    const LOCK_VALUE: u64 = 1_000_000;

    fn listing(price: u64) -> ListingFile {
        let mut signature = [1u8; 65];
        signature[64] = 0x84;
        ListingFile::for_tests(
            "dexreviews",
            [0x44; 32],
            0,
            PUBKEY,
            PAYMENT,
            vec![PriceStep {
                price,
                lock_time: LOCK_TIME,
                signature,
                fee: 0,
            }],
        )
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

    fn dest() -> OutputAddress {
        OutputAddress {
            version: 0,
            hash: vec![7; 20],
        }
    }

    fn purchase<'a>(
        listing: &'a ListingFile,
        funding: &'a [SpendableCoin],
        market_fee: Option<MarketFee>,
        rate: u64,
    ) -> PurchaseInput<'a> {
        PurchaseInput {
            network: Network::Main,
            account: 0,
            listing,
            step: 0,
            lock_value: LOCK_VALUE,
            name_height: 120,
            dest: dest(),
            market_fee,
            funding,
            change_address: CHANGE,
            rate,
            fixed_fee: None,
        }
    }

    fn finalize_input(funding: &[SpendableCoin], rate: u64) -> FinalizeInput<'_> {
        FinalizeInput {
            network: Network::Main,
            account: 0,
            transfer_outpoint: ([0x7d; 32], 0),
            transfer_value: LOCK_VALUE,
            lock_pubkey: PUBKEY,
            name: "dexreviews",
            name_height: 120,
            weak: false,
            claimed: 0,
            renewals: 0,
            renewal_block: [0x66; 32],
            dest_address: PAYMENT,
            funding,
            change_address: CHANGE,
            rate,
            fixed_fee: None,
        }
    }

    fn master() -> ExtendedPrivKey {
        // Any key gives real 65-byte signatures; the vsize is what matters.
        ExtendedPrivKey::from_seed(&[7u8; 64]).unwrap()
    }

    /// The plan's transaction with each of our inputs really signed, so the
    /// vsize is measured on what would be broadcast.
    fn really_signed(plan: &DraftPlan) -> Transaction {
        let mut tx = rebuild_unsigned(plan, Network::Main).unwrap();
        let master = master();
        for (i, inp) in plan.inputs.iter().enumerate() {
            if inp.foreign_witness_hex.is_some() {
                continue;
            }
            let path = hd::bip44_path(Network::Main, plan.account, inp.branch, inp.child_index);
            let child = master.derive_path(&path).unwrap();
            let hash160 = address::pubkey_to_hash160(&child.compressed_pubkey());
            tx.sign_p2wpkh_input(i, &child.secret, &hash160, inp.value, inp.sighash_type)
                .unwrap();
        }
        tx
    }

    fn assert_balanced(res: &PlanResult) {
        let out: u64 = res.plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(res.input_total, out + res.fee);
    }

    #[test]
    fn payment_is_last_and_transfer_first() {
        let l = listing(PRICE);
        let funding = [coin(1, 400_000_000, 0)];
        let res = build_purchase_plan(&purchase(&l, &funding, None, 5)).unwrap();
        let outs = &res.plan.outputs;
        assert_eq!(outs.len(), 3);
        assert_eq!(outs[0].covenant_type, COV_TRANSFER);
        assert_eq!(outs[0].value, LOCK_VALUE);
        assert_eq!(
            outs[0].address,
            lock_address(Network::Main, &PUBKEY).unwrap()
        );
        assert_eq!(outs[1].address, CHANGE);
        assert_eq!(res.plan.change_output_index, Some(1));
        let last = outs.last().unwrap();
        assert_eq!(last.address, l.payment_addr);
        assert_eq!(last.value, PRICE);
        assert_eq!(last.covenant_type, 0);
        let inp0 = &res.plan.inputs[0];
        assert_eq!(inp0.sequence, STEP_SEQUENCE);
        assert_eq!(inp0.sighash_type, STEP_SIGHASH);
        assert_eq!(inp0.txid, hex::encode([0x44; 32]));
        assert_eq!(
            inp0.foreign_witness_hex.as_deref().unwrap(),
            [
                hex::encode(l.steps[0].signature),
                hex::encode(lock_script(&PUBKEY))
            ]
        );
        assert_eq!(res.plan.inputs[1].sequence, FINAL_SEQUENCE);
        assert!(res.plan.inputs[1].foreign_witness_hex.is_none());
        assert_eq!(res.plan.locktime, encode_lock_time(LOCK_TIME).unwrap());
        assert_balanced(&res);
    }

    #[test]
    fn fee_covers_hsd_vsize_with_foreign_witness() {
        let l = listing(PRICE);
        let funding = [coin(1, 200_000_000, 0), coin(2, 100_000_000, 1)];
        let res = build_purchase_plan(&purchase(&l, &funding, None, 7)).unwrap();
        assert_eq!(res.plan.inputs.len(), 3, "both funding coins are needed");
        let tx = really_signed(&res.plan);
        let vsize = tx.vsize();
        // The foreign witness (65-byte signature + lock script) is counted.
        let without_foreign = {
            let mut t = tx.clone();
            t.inputs[0].witness.clear();
            t.vsize()
        };
        assert!(vsize > without_foreign);
        assert!(res.fee >= vsize * 7, "fee {} < {} * 7", res.fee, vsize);
        assert_eq!(res.fee, vsize * 7, "fee sized on the exact vsize");
        assert_balanced(&res);
    }

    #[test]
    fn market_fee_output_sits_before_change() {
        let l = listing(PRICE);
        let funding = [coin(1, 400_000_000, 0)];
        let fee_addr = OutputAddress {
            version: 0,
            hash: vec![9; 20],
        };
        let mf = MarketFee {
            address: fee_addr.clone(),
            value: 2_500_000,
        };
        let res = build_purchase_plan(&purchase(&l, &funding, Some(mf), 5)).unwrap();
        let outs = &res.plan.outputs;
        assert_eq!(outs.len(), 4);
        assert_eq!(outs[0].covenant_type, COV_TRANSFER);
        assert_eq!(outs[1].value, 2_500_000);
        assert_eq!(
            outs[1].address,
            address::encode_p2wpkh(Network::Main, &[9; 20]).unwrap()
        );
        assert_eq!(outs[2].address, CHANGE);
        assert_eq!(res.plan.change_output_index, Some(2));
        assert_eq!(outs[3].address, PAYMENT);
        assert_eq!(outs[3].value, PRICE);
        assert_eq!(
            res.change,
            400_000_000 - PRICE - 2_500_000 - res.fee,
            "market fee is paid on top of the price"
        );
        assert_balanced(&res);
    }

    #[test]
    fn rate_is_floored_at_five() {
        let l = listing(PRICE);
        let funding = [coin(1, 400_000_000, 0)];
        let res = build_purchase_plan(&purchase(&l, &funding, None, 1)).unwrap();
        let vsize = really_signed(&res.plan).vsize();
        assert_eq!(res.fee, vsize * SHAKEDEX_MIN_RATE_PER_BYTE);
        let fin = build_purchase_finalize_plan(&finalize_input(&funding, 0)).unwrap();
        let vsize = really_signed(&fin.plan).vsize();
        assert_eq!(fin.fee, vsize * SHAKEDEX_MIN_RATE_PER_BYTE);
    }

    #[test]
    fn purchase_plan_reports_shortfall() {
        let l = listing(PRICE);
        let funding = [coin(1, 10_000_000, 0)];
        let err = build_purchase_plan(&purchase(&l, &funding, None, 5)).unwrap_err();
        match err {
            AppError::InvalidInput(msg) => {
                assert!(msg.contains("not enough HNS"), "{msg}");
                assert!(msg.contains("need 250.000000 HNS plus fees"), "{msg}");
                assert!(msg.contains("have 10.000000 HNS"), "{msg}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    fn invalid_input(res: Result<PlanResult, AppError>) -> String {
        match res {
            Err(AppError::InvalidInput(msg)) => msg,
            Err(other) => panic!("expected InvalidInput, got {other:?}"),
            Ok(_) => panic!("expected InvalidInput, got a plan"),
        }
    }

    #[test]
    fn price_of_u64_max_is_refused_without_panic() {
        let l = listing(u64::MAX);
        let funding = [coin(1, 400_000_000, 0)];
        let msg = invalid_input(build_purchase_plan(&purchase(&l, &funding, None, 5)));
        assert!(msg.contains("money supply"), "{msg}");
    }

    #[test]
    fn price_above_max_money_is_refused() {
        let funding = [coin(1, 400_000_000, 0)];
        let l = listing(MAX_MONEY + 1);
        let msg = invalid_input(build_purchase_plan(&purchase(&l, &funding, None, 5)));
        assert!(msg.contains("money supply"), "{msg}");
        // Exactly MAX_MONEY passes the cap and fails only for lack of funds.
        let l = listing(MAX_MONEY);
        let msg = invalid_input(build_purchase_plan(&purchase(&l, &funding, None, 5)));
        assert!(msg.contains("not enough HNS"), "{msg}");
    }

    #[test]
    fn fee_percent_is_basis_points_of_the_price_or_none() {
        assert_eq!(fee_percent_bps(4_350_000, 435_000_000), Some(100));
        assert_eq!(fee_percent_bps(1, 3), Some(3_333));
        assert_eq!(fee_percent_bps(1, 0), None, "no price");
        // A foreign file may carry any u64 fee: no wrap to a small figure.
        assert_eq!(fee_percent_bps(u64::MAX, 1), None, "past u32");
        assert_eq!(fee_percent_bps(u64::MAX, u64::MAX), Some(10_000));
    }

    /// The dialog and the secure window both show this text: exact
    /// hundredths, so 199 bps is "1.99%" on both, never "1.9%" on one and
    /// "2.0%" on the other.
    #[test]
    fn fee_percent_text_is_exact_to_the_hundredth() {
        assert_eq!(
            fee_percent_text(4_350_000, 435_000_000).as_deref(),
            Some("1.00%")
        );
        assert_eq!(fee_percent_text(199, 10_000).as_deref(), Some("1.99%"));
        assert_eq!(fee_percent_text(1, 3).as_deref(), Some("33.33%"));
        assert_eq!(fee_percent_text(1, 100_000).as_deref(), Some("0.00%"));
        assert_eq!(fee_percent_text(1, 0), None);
        assert_eq!(fee_percent_text(u64::MAX, 1), None);
    }

    /// Main and testnet have miners at ten minutes a block; regtest and
    /// simnet mine on demand, so no time is promised there.
    #[test]
    fn finalize_wait_text_times_only_networks_with_miners() {
        assert_eq!(
            finalize_wait_text(Network::Main),
            "after a finalize, 288 blocks (about 2 days) after the purchase is mined"
        );
        assert_eq!(
            finalize_wait_text(Network::Testnet),
            "after a finalize, 288 blocks (about 2 days) after the purchase is mined"
        );
        assert_eq!(
            finalize_wait_text(Network::Regtest),
            "after a finalize, 10 blocks after the purchase is mined"
        );
        assert_eq!(
            finalize_wait_text(Network::Simnet),
            "after a finalize, 5 blocks after the purchase is mined"
        );
    }

    #[test]
    fn market_fee_overflowing_the_price_is_refused() {
        let l = listing(PRICE);
        let funding = [coin(1, 400_000_000, 0)];
        let mf = MarketFee {
            address: dest(),
            value: u64::MAX - PRICE + 1,
        };
        let msg = invalid_input(build_purchase_plan(&purchase(&l, &funding, Some(mf), 5)));
        assert!(msg.contains("money supply"), "{msg}");
    }

    #[test]
    fn funding_sum_overflow_is_refused_without_panic() {
        // Coins can never hold this much on chain; fund() must still refuse
        // rather than wrap.
        let l = listing(PRICE);
        let funding = [coin(1, u64::MAX, 0), coin(2, u64::MAX, 1)];
        let msg = invalid_input(build_purchase_plan(&purchase(&l, &funding, None, 5)));
        assert!(msg.contains("overflow"), "{msg}");
    }

    #[test]
    fn dust_change_is_folded_into_fee() {
        let l = listing(PRICE);
        // Size the fee first, then fund it with less than dust to spare.
        let probe = [coin(1, 400_000_000, 0)];
        let probe_fee = build_purchase_plan(&purchase(&l, &probe, None, 5))
            .unwrap()
            .fee;
        let funding = [coin(1, PRICE + probe_fee + DUST_THRESHOLD - 1, 0)];
        let res = build_purchase_plan(&purchase(&l, &funding, None, 5)).unwrap();
        assert_eq!(res.plan.outputs.len(), 2, "[TRANSFER, payment]");
        assert_eq!(res.plan.change_output_index, None);
        assert_eq!(res.change, 0);
        assert_eq!(res.plan.outputs[1].address, PAYMENT);
        let vsize = really_signed(&res.plan).vsize();
        assert!(res.fee >= vsize * 5);
        assert_balanced(&res);
    }

    #[test]
    fn unknown_step_is_refused() {
        let l = listing(PRICE);
        let funding = [coin(1, 400_000_000, 0)];
        let mut p = purchase(&l, &funding, None, 5);
        p.step = 1;
        assert!(matches!(
            build_purchase_plan(&p),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn finalize_spends_lock_with_script_only_witness() {
        let funding = [coin(3, 1_000_000, 1)];
        let res = build_purchase_finalize_plan(&finalize_input(&funding, 5)).unwrap();
        let inp0 = &res.plan.inputs[0];
        assert_eq!(inp0.txid, hex::encode([0x7d; 32]));
        assert_eq!(inp0.sequence, FINAL_SEQUENCE);
        assert_eq!(
            inp0.foreign_witness_hex.as_deref().unwrap(),
            [hex::encode(lock_script(&PUBKEY))]
        );
        assert_eq!(res.plan.locktime, 0);
        let outs = &res.plan.outputs;
        assert_eq!(outs.len(), 2);
        assert_eq!(outs[0].covenant_type, COV_FINALIZE);
        assert_eq!(outs[0].address, PAYMENT);
        assert_eq!(outs[0].value, LOCK_VALUE);
        assert_eq!(outs[1].address, CHANGE);
        assert_eq!(res.plan.change_output_index, Some(1));
        assert_eq!(res.change, 1_000_000 - res.fee);
        assert_balanced(&res);
    }

    #[test]
    fn zero_value_lock_coin_buys_and_finalizes() {
        // A name won with a single bid: the lock coin, the TRANSFER and the
        // FINALIZE all carry 0, and the buyer still pays price plus fees.
        let l = listing(PRICE);
        let funding = [coin(1, 400_000_000, 0)];
        let mut p = purchase(&l, &funding, None, 5);
        p.lock_value = 0;
        let res = build_purchase_plan(&p).unwrap();
        assert_eq!(res.plan.inputs[0].value, 0);
        assert_eq!(res.plan.outputs[0].covenant_type, COV_TRANSFER);
        assert_eq!(res.plan.outputs[0].value, 0);
        assert_eq!(res.plan.outputs.last().unwrap().value, PRICE);
        assert_eq!(res.input_total, 400_000_000);
        assert_eq!(res.change, 400_000_000 - PRICE - res.fee);
        assert_balanced(&res);
        let vsize = really_signed(&res.plan).vsize();
        assert_eq!(res.fee, vsize * 5);

        let mut f = finalize_input(&funding, 5);
        f.transfer_value = 0;
        let fin = build_purchase_finalize_plan(&f).unwrap();
        assert_eq!(fin.plan.inputs[0].value, 0);
        assert_eq!(fin.plan.outputs[0].covenant_type, COV_FINALIZE);
        assert_eq!(fin.plan.outputs[0].value, 0);
        assert_eq!(fin.change, 400_000_000 - fin.fee);
        assert_balanced(&fin);
    }
}
