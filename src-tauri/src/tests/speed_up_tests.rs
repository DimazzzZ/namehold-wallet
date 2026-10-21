//! Speed up (child pays for parent), the pure part: hsd's fee arithmetic, the
//! mempool entry as hsd sends it, the child's plan and fee (R7, R8), the
//! refusals that rest on the parent's entry and the target (R5, R6, R9, R10),
//! the signed child's re-check, and the death wording (R15).

use crate::error::AppError;
use crate::noncustodial::actions::{sign_plan, FINAL_SEQUENCE};
use crate::noncustodial::hd::{self, ExtendedPrivKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::send::{SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::speed_up::{
    child_fee, get_min_fee, get_rate, parent_change_index, parent_died_reason, parse_mempool_entry,
    plan_child, recheck_signed_child, refuse_parent_entry, ChildRequest, MempoolEntry,
    TargetSource, ABSURD_FEE_FACTOR, BELOW_RELAY, CHANGE_TOO_SMALL, EXPIRES_SOON, HAS_ANCESTOR,
    MEMPOOL_EXPIRY_SECS, RATE_NOT_HIGHER, SPEED_UP_ACTION,
};

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
const PARENT_TXID: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const NOW: i64 = 1_761_000_000;

fn master() -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&hd::seed_from_mnemonic(MNEMONIC, "").unwrap()).unwrap()
}

/// The change address (branch 1, index 0) of account 0 on regtest.
fn change_address() -> String {
    let seed = hd::seed_from_mnemonic(MNEMONIC, "").unwrap();
    hd::derive_address(Network::Regtest, &seed, 0, 1, 0)
        .unwrap()
        .2
}

/// hsd 8.0.0's `getmempoolentry` reply (`lib/node/rpc.js` `entryToJSON`):
/// `fee` is `Amount.coin(deltaFee, true)`, a number in HNS, while
/// `descendantfees` is the raw doo count.
fn entry_json(
    size: u64,
    fee_hns: f64,
    time: i64,
    desc_size: u64,
    desc_fees: u64,
) -> serde_json::Value {
    serde_json::json!({
        "size": size,
        "fee": fee_hns,
        "modifiedfee": 0,
        "time": time,
        "height": 120,
        "startingpriority": 0,
        "currentpriority": 0,
        "descendantcount": 0,
        "descendantsize": desc_size,
        "descendantfees": desc_fees,
        "ancestorcount": 0,
        "ancestorsize": 0,
        "ancestorfees": 0,
        "depends": []
    })
}

/// A 1-in-2-out plain send at 1 doo/vB: 140 vbytes, 140 doos, entered the
/// mempool an hour ago, nothing spending it yet.
fn entry() -> MempoolEntry {
    parse_mempool_entry(&entry_json(140, 0.00014, NOW - 3600, 140, 140)).unwrap()
}

fn change(value: u64) -> SpendableCoin {
    SpendableCoin {
        txid: PARENT_TXID.into(),
        vout: 1,
        value,
        branch: 1,
        child_index: 0,
    }
}

fn request<'a>(
    coin: &'a SpendableCoin,
    entry: &'a MempoolEntry,
    addr: &'a str,
    rate: u64,
) -> ChildRequest<'a> {
    ChildRequest {
        network: Network::Regtest,
        account: 0,
        change: coin,
        change_address: addr,
        entry,
        target_rate: rate,
        target_source: TargetSource::Typed,
        now: NOW,
        parent_txid: PARENT_TXID,
        parent_draft_id: "parent-draft",
        parent_is_purchase: false,
    }
}

fn invalid_input(e: AppError) -> String {
    match e {
        AppError::InvalidInput(m) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// --- hsd's arithmetic -------------------------------------------------------

#[test]
fn min_fee_and_rate_are_hsds() {
    // policy.getMinFee: floor(1000 * size / 1000), 0 for size 0.
    assert_eq!(get_min_fee(0), 0);
    assert_eq!(get_min_fee(108), 108);
    // policy.getRate: floor(fee * 1000 / size), 0 for size 0.
    assert_eq!(get_rate(0, 5), 0);
    assert_eq!(get_rate(248, 2480), 10_000);
    assert_eq!(get_rate(3, 1), 333);
}

#[test]
fn mempool_entry_reads_hsds_reply() {
    let e = entry();
    assert_eq!(
        e,
        MempoolEntry {
            size: 140,
            fee: 140,
            time: NOW - 3600,
            descendant_size: 140,
            descendant_fees: 140,
            ancestors: 0,
        }
    );
    // A fee of 1.23456789 HNS is not hsd's (6 decimals), but rounds, never
    // truncates, to doos.
    let e = parse_mempool_entry(&entry_json(140, 0.000141, NOW, 140, 141)).unwrap();
    assert_eq!(e.fee, 141);
}

#[test]
fn mempool_entry_missing_a_field_is_not_hsds_answer() {
    for key in [
        "size",
        "fee",
        "time",
        "descendantsize",
        "descendantfees",
        "ancestorcount",
        "depends",
    ] {
        let mut v = entry_json(140, 0.00014, NOW, 140, 140);
        v.as_object_mut().unwrap().remove(key);
        let err = parse_mempool_entry(&v).unwrap_err();
        assert!(
            matches!(&err, AppError::Rpc(m) if m.contains(key)),
            "{key}: {err:?}"
        );
    }
}

// --- R8: the fee ------------------------------------------------------------

#[test]
fn child_fee_lifts_the_package_to_the_target() {
    // 10 doos/vB over parent 140 + child 108 vbytes, less the 140 paid.
    assert_eq!(child_fee(10, 140, 140, 108), 10 * 248 - 140);
}

#[test]
fn child_fee_counts_descendants_that_are_not_ours() {
    // A foreign descendant of 200 vbytes paying 5000 doos already lifts the
    // package past 10 doos/vB: the child pays only its own relay floor.
    assert_eq!(child_fee(10, 340, 5140, 108), get_min_fee(108));
    // One paying badly raises what the child pays.
    assert_eq!(child_fee(10, 340, 340, 108), 10 * 448 - 340);
}

#[test]
fn child_fee_never_falls_below_the_childs_own_relay_floor() {
    assert_eq!(child_fee(1, 140, 140, 108), 108);
    assert_eq!(child_fee(2, 140, 280, 108), 216);
}

// --- R7, R8: the plan ---------------------------------------------------------

#[test]
fn child_plan_is_one_input_one_output_to_our_change() {
    let coin = change(1_000_000);
    let e = entry();
    let addr = change_address();
    let child = plan_child(&request(&coin, &e, &addr, 10)).unwrap();

    let plan = &child.plan;
    assert_eq!(plan.inputs.len(), 1);
    let input = &plan.inputs[0];
    assert_eq!((input.txid.as_str(), input.vout), (PARENT_TXID, 1));
    assert_eq!((input.branch, input.child_index), (1, 0));
    assert_eq!(input.sequence, FINAL_SEQUENCE);
    assert!(input.foreign_witness_hex.is_none() && input.lock_key_name.is_none());
    assert_eq!(plan.locktime, 0);
    assert_eq!(plan.outputs.len(), 1);
    assert_eq!(plan.outputs[0].address, addr);
    assert_eq!(plan.outputs[0].covenant_type, 0);
    assert_eq!(plan.change_output_index, Some(0));
    assert!(!plan.has_foreign_or_custom_inputs());
    assert_eq!(plan.own_inputs(), vec![(PARENT_TXID.to_string(), 1)]);

    // The child's size is hsd's vsize of the signed 1-in-1-out child.
    let s = &child.summary;
    assert_eq!(s.child_size, 108);
    assert_eq!(s.fee_doos, (10 * (140 + 108) - 140) as i64);
    assert_eq!(plan.outputs[0].value, 1_000_000 - s.fee_doos as u64);
    assert_eq!(s.change_doos, plan.outputs[0].value as i64);
    assert_eq!(s.input_total_doos, 1_000_000);
    assert_eq!(s.num_inputs, 1);
    assert_eq!(s.send_total_doos, 0);
    assert!(s.recipient_address.is_none());
    assert_eq!(s.action, SPEED_UP_ACTION);
    assert_eq!(s.parent_txid, PARENT_TXID);
    assert_eq!(s.parent_draft_id, "parent-draft");
    assert_eq!(s.txid, child.txid);
    assert_eq!(s.package_rate_before, 1000);
    assert_eq!(s.package_rate_after, 10_000);
    assert_eq!(s.expires_at, NOW - 3600 + MEMPOOL_EXPIRY_SECS);
    assert_eq!(s.target_rate_per_vbyte, 10);
    assert_eq!(s.target_source, TargetSource::Typed);
}

#[test]
fn child_plan_signs_to_the_size_it_was_priced_on() {
    let coin = change(1_000_000);
    let e = entry();
    let addr = change_address();
    let child = plan_child(&request(&coin, &e, &addr, 10)).unwrap();
    let mut session = SignerSession::unlock("p".into(), Network::Regtest, master(), 60_000);
    let (hex, txid) = sign_plan(&mut session, &child.plan).unwrap();
    assert_eq!(txid, child.txid);
    let tx = crate::noncustodial::tx::Transaction::decode(&hex::decode(&hex).unwrap()).unwrap();
    assert_eq!(tx.vsize(), child.summary.child_size);
    recheck_signed_child(&hex, &child.summary).unwrap();
}

#[test]
fn signed_child_that_differs_from_its_summary_is_refused() {
    let coin = change(1_000_000);
    let e = entry();
    let addr = change_address();
    let child = plan_child(&request(&coin, &e, &addr, 10)).unwrap();
    let mut session = SignerSession::unlock("p".into(), Network::Regtest, master(), 60_000);
    let (hex, _) = sign_plan(&mut session, &child.plan).unwrap();

    // A summary claiming a higher fee than the signed child pays.
    let mut s = child.summary.clone();
    s.fee_doos += 1;
    assert!(recheck_signed_child(&hex, &s).is_err());
    // The signed child larger than priced: below the target rate.
    let mut s = child.summary.clone();
    s.child_size -= 1;
    s.package_rate_after = get_rate(
        s.package_size + s.child_size,
        s.package_fees_doos + s.fee_doos as u64,
    );
    let err = recheck_signed_child(&hex, &s).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
    // A summary naming another transaction.
    let mut s = child.summary.clone();
    s.txid = "00".repeat(32);
    assert!(recheck_signed_child(&hex, &s).is_err());
    // Not a transaction.
    assert!(recheck_signed_child("zz", &child.summary).is_err());
}

#[test]
fn signed_child_over_the_absurd_cap_or_below_dust_is_refused() {
    // Build a legitimate child, then claim a summary whose numbers make the
    // signed child pay past hsd's cap: the re-check reads the fee from the
    // signed output against the input total.
    let coin = change(1_000_000);
    let e = entry();
    let addr = change_address();
    let child = plan_child(&request(&coin, &e, &addr, 10)).unwrap();
    let mut session = SignerSession::unlock("p".into(), Network::Regtest, master(), 60_000);
    let (hex, _) = sign_plan(&mut session, &child.plan).unwrap();
    let mut s = child.summary.clone();
    s.input_total_doos += (get_min_fee(108) * ABSURD_FEE_FACTOR) as i64;
    s.fee_doos += (get_min_fee(108) * ABSURD_FEE_FACTOR) as i64;
    let err = invalid_input(recheck_signed_child(&hex, &s).unwrap_err());
    assert!(err.contains("more in fee than your node accepts"), "{err}");
}

/// A signed child whose one output is dust, or that leaves the package
/// below its target, is refused as signed.
#[test]
fn signed_child_below_dust_or_below_its_target_is_refused() {
    let coin = change(1_000_000);
    let e = entry();
    let addr = change_address();
    let child = plan_child(&request(&coin, &e, &addr, 10)).unwrap();
    let mut session = SignerSession::unlock("p".into(), Network::Regtest, master(), 60_000);

    // The package's target raised past what the fee pays.
    let (hex, _) = sign_plan(&mut session, &child.plan).unwrap();
    let mut s = child.summary.clone();
    s.target_rate_per_vbyte += 1;
    let err = invalid_input(recheck_signed_child(&hex, &s).unwrap_err());
    assert!(err.contains("below the target rate"), "{err}");

    // The same child paying all but 500 doos of its input as fee.
    let mut plan = child.plan.clone();
    plan.outputs[0].value = 500;
    let (hex, txid) = sign_plan(&mut session, &plan).unwrap();
    let mut s = child.summary.clone();
    s.txid = txid;
    s.fee_doos = 1_000_000 - 500;
    s.change_doos = 500;
    s.target_rate_per_vbyte = 1;
    let err = invalid_input(recheck_signed_child(&hex, &s).unwrap_err());
    assert!(err.contains("too small"), "{err}");
}

#[test]
fn mempool_entry_whose_package_is_smaller_than_itself_is_not_hsds() {
    let err = parse_mempool_entry(&entry_json(140, 0.00014, NOW, 139, 140)).unwrap_err();
    assert!(matches!(err, AppError::Rpc(_)), "{err:?}");
}

// --- R5, R6: the parent's entry -----------------------------------------------

/// hsd expires only a transaction without a mempool parent, and queues one
/// only after its parents: a parent that itself waits on another unconfirmed
/// transaction is refused, whether hsd says so by `ancestorcount` or by
/// `depends`.
#[test]
fn parent_waiting_on_another_unconfirmed_transaction_is_refused() {
    let mut v = entry_json(140, 0.00014, NOW, 140, 140);
    v["ancestorcount"] = serde_json::json!(1);
    v["depends"] = serde_json::json!(["11".repeat(32)]);
    let e = parse_mempool_entry(&v).unwrap();
    assert_eq!(e.ancestors, 1);
    assert_eq!(
        invalid_input(refuse_parent_entry(&e, NOW).unwrap_err()),
        HAS_ANCESTOR
    );
    let coin = change(1_000_000);
    let addr = change_address();
    assert_eq!(
        invalid_input(plan_child(&request(&coin, &e, &addr, 10)).unwrap_err()),
        HAS_ANCESTOR
    );
    // `depends` alone names one.
    let mut v = entry_json(140, 0.00014, NOW, 140, 140);
    v["depends"] = serde_json::json!(["11".repeat(32)]);
    let e = parse_mempool_entry(&v).unwrap();
    assert_eq!(
        invalid_input(refuse_parent_entry(&e, NOW).unwrap_err()),
        HAS_ANCESTOR
    );
}

#[test]
fn parent_close_to_its_mempool_expiry_is_refused() {
    // Exactly 6 hours left: still offered.
    let at_margin =
        parse_mempool_entry(&entry_json(140, 0.00014, NOW - 66 * 3600, 140, 140)).unwrap();
    refuse_parent_entry(&at_margin, NOW).unwrap();
    // One second less.
    let late =
        parse_mempool_entry(&entry_json(140, 0.00014, NOW - 66 * 3600 - 1, 140, 140)).unwrap();
    assert_eq!(
        invalid_input(refuse_parent_entry(&late, NOW).unwrap_err()),
        EXPIRES_SOON
    );
    let coin = change(1_000_000);
    let addr = change_address();
    assert_eq!(
        invalid_input(plan_child(&request(&coin, &late, &addr, 10)).unwrap_err()),
        EXPIRES_SOON
    );
}

#[test]
fn parent_below_the_relay_floor_is_refused() {
    // 139 doos for 140 vbytes: below getMinFee(140) = 140.
    let free = parse_mempool_entry(&entry_json(140, 0.000139, NOW, 140, 139)).unwrap();
    assert_eq!(
        invalid_input(refuse_parent_entry(&free, NOW).unwrap_err()),
        BELOW_RELAY
    );
    let coin = change(1_000_000);
    let addr = change_address();
    assert_eq!(
        invalid_input(plan_child(&request(&coin, &free, &addr, 10)).unwrap_err()),
        BELOW_RELAY
    );
    // Exactly at the floor passes.
    let at_floor = parse_mempool_entry(&entry_json(140, 0.00014, NOW, 140, 140)).unwrap();
    refuse_parent_entry(&at_floor, NOW).unwrap();
}

// --- R9, R10: the target and the change ---------------------------------------

#[test]
fn target_at_or_below_the_package_rate_is_refused() {
    let coin = change(1_000_000);
    let addr = change_address();
    // Package at 10 doos/vB.
    let e = parse_mempool_entry(&entry_json(140, 0.0014, NOW, 140, 1400)).unwrap();
    for rate in [1, 10] {
        assert_eq!(
            invalid_input(plan_child(&request(&coin, &e, &addr, rate)).unwrap_err()),
            RATE_NOT_HIGHER,
            "rate {rate}"
        );
    }
    plan_child(&request(&coin, &e, &addr, 11)).unwrap();
}

#[test]
fn change_too_small_to_pay_is_refused() {
    let e = entry();
    let addr = change_address();
    // Fee at 10 doos/vB is 2340: 3000 leaves 660, below dust.
    let coin = change(3000);
    assert_eq!(
        invalid_input(plan_child(&request(&coin, &e, &addr, 10)).unwrap_err()),
        CHANGE_TOO_SMALL
    );
    // Exactly dust left is allowed.
    let coin = change(2340 + DUST_THRESHOLD);
    let child = plan_child(&request(&coin, &e, &addr, 10)).unwrap();
    assert_eq!(child.plan.outputs[0].value, DUST_THRESHOLD);
}

#[test]
fn fee_past_hsds_absurd_cap_is_refused_with_the_cap() {
    let e = entry();
    let addr = change_address();
    let coin = change(100_000_000);
    // 5000 doos/vB over 248 vbytes is far past 108 * 10000 doos.
    let err = invalid_input(plan_child(&request(&coin, &e, &addr, 5000)).unwrap_err());
    assert_eq!(
        err,
        "This would pay more in fee than your node accepts from one transaction (about 1.080000 HNS); choose a lower rate."
    );
    // Exactly at the cap passes: 108 * 10000 doos.
    let cap = get_min_fee(108) * ABSURD_FEE_FACTOR;
    assert!(child_fee(4355, 140, 140, 108) <= cap);
    plan_child(&request(&coin, &e, &addr, 4355)).unwrap();
}

// --- R2: where the parent's change is -----------------------------------------

#[test]
fn a_plain_sends_change_is_output_one_when_it_has_one() {
    let with = r#"{"action":"send_hns","sendTotalDoos":5,"feeDoos":140,"changeDoos":9000,"inputTotalDoos":9145,"numInputs":1,"recipientAddress":"rs1q","txid":"aa"}"#;
    let without = r#"{"action":"send_hns","sendTotalDoos":5,"feeDoos":140,"changeDoos":0,"inputTotalDoos":145,"numInputs":1,"recipientAddress":"rs1q","txid":"aa"}"#;
    assert_eq!(
        parent_change_index("send_hns", "{}", with).unwrap(),
        Some(1)
    );
    assert_eq!(
        parent_change_index("send_hns", "{}", without).unwrap(),
        None
    );
}

#[test]
fn a_plans_change_is_its_change_output_if_covenant_free() {
    let plan = |idx: &str, cov: u8| {
        format!(
            r#"{{"version":0,"locktime":0,"account":0,"network":"regtest","inputs":[],
               "outputs":[{{"value":1,"address":"a","covenant_type":9,"covenant_items_hex":[]}},
                          {{"value":2,"address":"b","covenant_type":{cov},"covenant_items_hex":[]}}],
               "change_output_index":{idx}}}"#
        )
    };
    assert_eq!(
        parent_change_index("update", &plan("1", 0), "{}").unwrap(),
        Some(1)
    );
    assert_eq!(
        parent_change_index("update", &plan("null", 0), "{}").unwrap(),
        None
    );
    // A change index naming a covenant output is no change of ours to spend.
    assert_eq!(
        parent_change_index("update", &plan("1", 3), "{}").unwrap(),
        None
    );
    assert_eq!(
        parent_change_index("update", &plan("7", 0), "{}").unwrap(),
        None
    );
    assert!(parent_change_index("update", "not a plan", "{}").is_err());
}

// --- R15: the death wording ---------------------------------------------------

#[test]
fn a_childs_death_names_its_parent() {
    assert_eq!(
        parent_died_reason(PARENT_TXID),
        format!(
            "The transaction it sped up ({PARENT_TXID}) did not confirm, so this one can never confirm. Nothing was paid by either."
        )
    );
}
