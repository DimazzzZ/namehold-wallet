//! What the secure window shows for a name action, read from the plan that
//! is signed (`noncustodial::review`).

use crate::noncustodial::actions::{
    rebuild_unsigned, DraftPlan, PlanInput, PlanOutput, FINAL_SEQUENCE,
};
use crate::noncustodial::address::encode_p2wpkh;
use crate::noncustodial::bids::compute_blind;
use crate::noncustodial::covenants;
use crate::noncustodial::names::hash_name;
use crate::noncustodial::network::Network;
use crate::noncustodial::review::{
    covenant_rows, plan_fee, transfer_recipient, BidOnRecord, DraftClaims, Row, BID_REVEAL_WARNING,
};
use crate::noncustodial::tx::Covenant;

fn addr(byte: u8) -> String {
    encode_p2wpkh(Network::Main, &[byte; 20]).unwrap()
}

fn input(byte: u8, value: u64) -> PlanInput {
    PlanInput {
        txid: hex::encode([byte; 32]),
        vout: 0,
        value,
        branch: 0,
        child_index: 0,
        sighash_type: 1,
        sequence: FINAL_SEQUENCE,
        foreign_witness_hex: None,
        lock_key_name: None,
    }
}

fn out(value: u64, address: &str, cov: &Covenant) -> PlanOutput {
    PlanOutput {
        value,
        address: address.to_string(),
        covenant_type: cov.covenant_type,
        covenant_items_hex: cov.items.iter().map(hex::encode).collect(),
    }
}

fn plain(value: u64, address: &str) -> PlanOutput {
    out(value, address, &Covenant::default())
}

/// A plan spending `inputs` into `outputs`, the last output being change.
fn plan(inputs: Vec<PlanInput>, outputs: Vec<PlanOutput>) -> DraftPlan {
    let change = outputs.len() - 1;
    DraftPlan {
        version: 0,
        locktime: 0,
        account: 0,
        network: "main".into(),
        inputs,
        outputs,
        change_output_index: Some(change),
    }
}

fn nh(name: &str) -> [u8; 32] {
    hash_name(name).unwrap()
}

fn transfer_to(name: &str, to: u8) -> Covenant {
    covenants::transfer(&nh(name), 100, 0, &[to; 20])
}

fn claims<'a>(action: &'a str, names: &'a [String], recipient: Option<&'a str>) -> DraftClaims<'a> {
    DraftClaims {
        action,
        names,
        recipient,
        summary_txid: None,
        warnings: &[],
    }
}

fn no_bids(_: &str) -> Option<BidOnRecord> {
    None
}

fn value<'a>(rows: &'a [Row], label: &str) -> Option<&'a str> {
    rows.iter()
        .find(|r| r.label == label)
        .map(|r| r.value.as_str())
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_transfer_shows_its_name_and_the_recipient_its_covenant_commits_to() {
    let p = plan(
        vec![input(1, 1_000_000), input(2, 5_000_000)],
        vec![
            out(1_000_000, &addr(9), &transfer_to("alice", 7)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice"]);
    let rows = covenant_rows(&claims("transfer", &n, None), &p, &no_bids).unwrap();
    assert_eq!(value(&rows, "Action"), Some("Transfer to another wallet"));
    assert_eq!(value(&rows, "Name"), Some("alice"));
    // The covenant's address, not the output's (which stays ours until finalize).
    assert_eq!(value(&rows, "To"), Some(addr(7).as_str()));
    assert_eq!(value(&rows, "Fee"), Some("0.010000 HNS"));
}

#[test]
fn a_batch_transfer_lists_every_name_and_its_one_recipient_once() {
    let p = plan(
        vec![input(1, 1), input(2, 1), input(3, 5_000_000)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            out(1, &addr(9), &transfer_to("bob", 7)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice", "bob"]);
    let rows = covenant_rows(&claims("batch-transfer", &n, Some(&addr(7))), &p, &no_bids).unwrap();
    assert_eq!(value(&rows, "Names (2)"), Some("alice, bob"));
    assert_eq!(rows.iter().filter(|r| r.label.starts_with("To")).count(), 1);
    assert_eq!(value(&rows, "To"), Some(addr(7).as_str()));
}

#[test]
fn names_going_to_different_recipients_get_one_row_each() {
    let p = plan(
        vec![input(1, 1), input(2, 1), input(3, 5_000_000)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            out(1, &addr(9), &transfer_to("bob", 6)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice", "bob"]);
    let rows = covenant_rows(&claims("batch-transfer", &n, None), &p, &no_bids).unwrap();
    assert_eq!(value(&rows, "To (alice)"), Some(addr(7).as_str()));
    assert_eq!(value(&rows, "To (bob)"), Some(addr(6).as_str()));
}

#[test]
fn a_plan_acting_on_a_name_it_does_not_show_is_refused() {
    let p = plan(
        vec![input(1, 1), input(2, 5_000_000)],
        vec![
            out(1, &addr(9), &transfer_to("bob", 7)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice"]);
    let err = covenant_rows(&claims("transfer", &n, None), &p, &no_bids).unwrap_err();
    assert!(
        err.to_string().contains("acts on a name it does not show"),
        "{err}"
    );
}

#[test]
fn a_name_shown_but_not_acted_on_is_refused() {
    let p = plan(
        vec![input(1, 1), input(2, 5_000_000)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice", "bob"]);
    let err = covenant_rows(&claims("batch-transfer", &n, None), &p, &no_bids).unwrap_err();
    assert!(err.to_string().contains("'bob' is shown"), "{err}");
}

#[test]
fn a_recipient_claim_other_than_the_covenants_is_refused() {
    let p = plan(
        vec![input(1, 1), input(2, 5_000_000)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice"]);
    let err = covenant_rows(&claims("transfer", &n, Some(&addr(6))), &p, &no_bids).unwrap_err();
    assert!(err.to_string().contains("recipient shown is not"), "{err}");
}

#[test]
fn a_finalize_shows_where_the_name_goes() {
    let cov = covenants::finalize(&nh("alice"), 100, b"alice", 0, 0, 1, &[3; 32]);
    let p = plan(
        vec![input(1, 1_000_000), input(2, 5_000_000)],
        vec![out(1_000_000, &addr(7), &cov), plain(4_990_000, &addr(8))],
    );
    let n = names(&["alice"]);
    let rows = covenant_rows(&claims("finalize", &n, None), &p, &no_bids).unwrap();
    assert_eq!(value(&rows, "Name goes to"), Some(addr(7).as_str()));
}

fn bid_plan(value: u64, lockup: u64, nonce: [u8; 32]) -> DraftPlan {
    let blind = compute_blind(value, &nonce);
    let cov = covenants::bid(&nh("alice"), 100, b"alice", &blind);
    plan(
        vec![input(2, 10_000_000)],
        vec![
            out(lockup, &addr(9), &cov),
            plain(10_000_000 - lockup - 10_000, &addr(8)),
        ],
    )
}

#[test]
fn a_bid_shows_the_recorded_value_and_the_lockup_with_the_reveal_warning() {
    let nonce = [5u8; 32];
    let p = bid_plan(2_000_000, 5_000_000, nonce);
    let n = names(&["alice"]);
    let rec = |_: &str| {
        Some(BidOnRecord {
            value: 2_000_000,
            nonce,
        })
    };
    let rows = covenant_rows(&claims("bid", &n, None), &p, &rec).unwrap();
    assert_eq!(value(&rows, "Bid"), Some("2.000000 HNS"));
    assert_eq!(value(&rows, "Locked until reveal"), Some("5.000000 HNS"));
    assert!(rows
        .iter()
        .any(|r| r.label == "Warning" && r.value == BID_REVEAL_WARNING));
}

#[test]
fn a_bid_without_a_record_is_refused() {
    let p = bid_plan(2_000_000, 5_000_000, [5; 32]);
    let n = names(&["alice"]);
    let err = covenant_rows(&claims("bid", &n, None), &p, &no_bids).unwrap_err();
    assert!(err.to_string().contains("no record of its value"), "{err}");
}

#[test]
fn a_recorded_bid_that_is_not_the_blind_the_plan_commits_to_is_refused() {
    let nonce = [5u8; 32];
    let p = bid_plan(2_000_000, 5_000_000, nonce);
    let n = names(&["alice"]);
    // The record says 1 HNS; the BID commits to 2 HNS.
    let rec = |_: &str| {
        Some(BidOnRecord {
            value: 1_000_000,
            nonce,
        })
    };
    let err = covenant_rows(&claims("bid", &n, None), &p, &rec).unwrap_err();
    assert!(
        err.to_string()
            .contains("not the one the transaction commits to"),
        "{err}"
    );
}

#[test]
fn a_bid_above_its_lockup_is_refused() {
    let nonce = [5u8; 32];
    let p = bid_plan(6_000_000, 5_000_000, nonce);
    let n = names(&["alice"]);
    let rec = |_: &str| {
        Some(BidOnRecord {
            value: 6_000_000,
            nonce,
        })
    };
    let err = covenant_rows(&claims("bid", &n, None), &p, &rec).unwrap_err();
    assert!(err.to_string().contains("exceeds its lockup"), "{err}");
}

#[test]
fn a_revoke_warns_that_it_burns_the_name() {
    let cov = covenants::revoke(&nh("alice"), 100);
    let p = plan(
        vec![input(1, 1_000_000), input(2, 5_000_000)],
        vec![out(1_000_000, &addr(9), &cov), plain(4_990_000, &addr(8))],
    );
    let n = names(&["alice"]);
    let rows = covenant_rows(&claims("revoke", &n, None), &p, &no_bids).unwrap();
    assert_eq!(value(&rows, "Action"), Some("Revoke name (burns it)"));
    assert!(rows
        .iter()
        .any(|r| r.label == "Warning" && r.value.contains("burns alice for good")));
}

#[test]
fn a_plain_output_that_is_not_change_is_shown_as_a_payment() {
    let p = plan(
        vec![input(1, 1_000_000), input(2, 5_000_000)],
        vec![
            out(1_000_000, &addr(9), &transfer_to("alice", 7)),
            plain(2_000_000, &addr(6)),
            plain(2_990_000, &addr(8)),
        ],
    );
    let n = names(&["alice"]);
    let rows = covenant_rows(&claims("transfer", &n, None), &p, &no_bids).unwrap();
    assert_eq!(
        value(&rows, "Pays"),
        Some(format!("2.000000 HNS to {}", addr(6)).as_str())
    );
}

#[test]
fn the_txid_is_the_plans_and_a_summary_of_another_transaction_is_refused() {
    let p = plan(
        vec![input(1, 1), input(2, 5_000_000)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            plain(4_990_000, &addr(8)),
        ],
    );
    let txid = rebuild_unsigned(&p, Network::Main).unwrap().txid();
    let n = names(&["alice"]);
    let mut c = claims("transfer", &n, None);
    c.summary_txid = Some(&txid);
    let rows = covenant_rows(&c, &p, &no_bids).unwrap();
    assert_eq!(value(&rows, "Txid"), Some(txid.as_str()));
    let other = "00".repeat(32);
    c.summary_txid = Some(&other);
    let err = covenant_rows(&c, &p, &no_bids).unwrap_err();
    assert!(
        err.to_string()
            .contains("summary is of another transaction"),
        "{err}"
    );
}

#[test]
fn the_fee_is_what_the_inputs_bring_minus_what_the_outputs_carry() {
    let p = plan(
        vec![input(1, 3), input(2, 7)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            plain(4, &addr(8)),
        ],
    );
    assert_eq!(plan_fee(&p).unwrap(), 5);
    let over = plan(
        vec![input(1, 3)],
        vec![
            out(1, &addr(9), &transfer_to("alice", 7)),
            plain(4, &addr(8)),
        ],
    );
    assert!(plan_fee(&over).is_err());
}

#[test]
fn a_transfer_to_an_address_version_the_wallet_cannot_show_is_refused() {
    let cov = covenants::transfer(&nh("alice"), 100, 1, &[7; 32]);
    let o = out(1, &addr(9), &cov);
    let err = transfer_recipient(Network::Main, &o).unwrap_err();
    assert!(
        err.to_string().contains("cannot show the recipient"),
        "{err}"
    );
    // A 32-byte version-0 program (a Shakedex lock) is shown.
    let lock = out(
        1,
        &addr(9),
        &covenants::transfer(&nh("alice"), 100, 0, &[7; 32]),
    );
    assert!(transfer_recipient(Network::Main, &lock)
        .unwrap()
        .starts_with("hs1q"));
}

#[test]
fn a_plan_acting_on_no_name_is_refused() {
    let p = plan(vec![input(2, 5_000_000)], vec![plain(4_990_000, &addr(8))]);
    let n = names(&["alice"]);
    let err = covenant_rows(&claims("renew", &n, None), &p, &no_bids).unwrap_err();
    assert!(err.to_string().contains("acts on no name"), "{err}");
}
