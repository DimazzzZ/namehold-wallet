use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{BlockchainInfo, NodeCoin};
use crate::noncustodial::shakedex::listing_file::ListingFile;
use crate::noncustodial::shakedex::verify::{self, Hidden, Verdict};
use crate::tests::mock_node_rpc::MockNodeRpc;
use serde_json::Value;

const LISTING_FILE: &str = include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");
const COIN: &str = include_str!("../../tests/vectors/shakedex/coin_dexreviews.json");
const NAME: &str = include_str!("../../tests/vectors/shakedex/name_dexreviews.json");

fn coin_value() -> Value {
    let w: Value = serde_json::from_str(COIN).unwrap();
    w["coin"].clone()
}

/// `getnameinfo`'s raw result: `{"info": {...}, "start": ...}`.
fn name_value() -> Value {
    let w: Value = serde_json::from_str(NAME).unwrap();
    w["nameInfo"].clone()
}

fn mock_with(coin: Option<Value>, name: Value, mtp: Option<u64>, tip: i64) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_get_coin(move |_, _| {
            Ok(coin
                .clone()
                .map(|c| serde_json::from_value::<NodeCoin>(c).unwrap()))
        })
        .with_name_info(name)
        .with_blockchain_info(BlockchainInfo {
            blocks: tip,
            mediantime: mtp,
            ..Default::default()
        })
}

fn mock(coin: Option<Value>, mtp: Option<u64>, tip: i64) -> MockNodeRpc {
    mock_with(coin, name_value(), mtp, tip)
}

/// Well after the listing's lock and well inside the name's validity.
fn tip_from_fixture() -> i64 {
    name_value()["info"]["renewal"].as_i64().unwrap() + 1_000
}

fn listing() -> ListingFile {
    ListingFile::parse(LISTING_FILE, Network::Main).unwrap()
}

/// An MTP past the first step's lock time.
fn late_mtp(l: &ListingFile) -> u64 {
    l.steps[0].lock_time + 10_000
}

async fn verdict(m: &MockNodeRpc, l: &ListingFile) -> Verdict {
    verify::verify_listing(m, Network::Main, l).await
}

#[tokio::test]
async fn live_listing_is_buyable() {
    let l = listing();
    let v = verdict(
        &mock(Some(coin_value()), Some(late_mtp(&l)), tip_from_fixture()),
        &l,
    )
    .await;
    match v {
        Verdict::Buyable(b) => {
            assert_eq!(b.current_step, 0);
            assert_eq!(b.next_step, None);
            assert_eq!(b.lock_value, 1_290_000);
            assert_eq!(b.name_height, 81_715);
            assert_eq!(b.expiry_end, 337_627 + 105_120);
            assert!(!b.warn_expiry);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn spent_lock_coin_is_sold_or_cancelled() {
    let l = listing();
    let v = verdict(&mock(None, Some(late_mtp(&l)), tip_from_fixture()), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::SoldOrCancelled)),
        "{v:?}"
    );
}

#[tokio::test]
async fn tampered_price_fails_verification() {
    let mut j: Value = serde_json::from_str(LISTING_FILE).unwrap();
    j["data"][0]["price"] = (j["data"][0]["price"].as_u64().unwrap() + 1).into();
    let l = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
    let v = verdict(
        &mock(Some(coin_value()), Some(late_mtp(&l)), tip_from_fixture()),
        &l,
    )
    .await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

#[tokio::test]
async fn wrong_lock_value_fails_verification() {
    let l = listing();
    let mut c = coin_value();
    c["value"] = 1_290_001.into();
    let v = verdict(&mock(Some(c), Some(late_mtp(&l)), tip_from_fixture()), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

#[tokio::test]
async fn lock_coin_paid_elsewhere_fails_verification() {
    let l = listing();
    let mut c = coin_value();
    // A valid mainnet address that is not this listing's lock.
    c["address"] = l.payment_addr.clone().into();
    let v = verdict(&mock(Some(c), Some(late_mtp(&l)), tip_from_fixture()), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

/// hsd always reports a coin's address and covenant: a reply without one
/// says nothing about the listing, so it could not be checked rather than
/// "failed verification".
#[tokio::test]
async fn lock_coin_without_address_or_covenant_could_not_be_checked() {
    for field in ["address", "covenant"] {
        let l = listing();
        let mut c = coin_value();
        c.as_object_mut().unwrap().remove(field);
        let v = verdict(&mock(Some(c), Some(late_mtp(&l)), tip_from_fixture()), &l).await;
        match v {
            Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
                assert!(reason.contains(field), "{field}: {reason}")
            }
            other => panic!("{field}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn lock_coin_holding_another_name_fails_verification() {
    let l = listing();
    let mut c = coin_value();
    c["covenant"]["items"][2] = hex::encode("othername").into();
    let v = verdict(&mock(Some(c), Some(late_mtp(&l)), tip_from_fixture()), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

#[tokio::test]
async fn covenant_height_mismatch_fails_verification() {
    let l = listing();
    let mut c = coin_value();
    c["covenant"]["items"][1] = "343f0100".into();
    let v = verdict(&mock(Some(c), Some(late_mtp(&l)), tip_from_fixture()), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

#[tokio::test]
async fn re_registered_name_fails_verification() {
    let l = listing();
    let mut n = name_value();
    n["info"]["owner"]["hash"] = "11".repeat(32).into();
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            n,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

/// A name reply that leaves out the owner (or half of it) proves nothing
/// about who owns the name: the listing could not be checked. It is not "the
/// name has changed hands", which would be a guess presented as a fact.
#[tokio::test]
async fn name_without_owner_could_not_be_checked() {
    for remove in [&["owner"][..], &["owner", "hash"], &["owner", "index"]] {
        let l = listing();
        let mut name = name_value();
        match remove {
            [field] => {
                name["info"].as_object_mut().unwrap().remove(*field);
            }
            [_, field] => {
                name["info"]["owner"]
                    .as_object_mut()
                    .unwrap()
                    .remove(*field);
            }
            _ => unreachable!(),
        }
        let v = verdict(
            &mock_with(
                Some(coin_value()),
                name,
                Some(late_mtp(&l)),
                tip_from_fixture(),
            ),
            &l,
        )
        .await;
        match v {
            Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
                assert!(reason.contains("owner"), "{remove:?}: {reason}")
            }
            other => panic!("{remove:?}: {other:?}"),
        }
    }
}

/// hsd answers `"info": null` for a name that has expired: the listing can
/// never be finalized, which is what "expires before finalize" says.
#[tokio::test]
async fn expired_name_is_hidden_as_expiring_before_finalize() {
    let l = listing();
    let mut name = name_value();
    name["info"] = Value::Null;
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            name,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::ExpiresBeforeFinalize)),
        "{v:?}"
    );
}

/// A reply with no `info` key at all is not hsd saying the name expired (hsd
/// always sends the key, `null` when expired): it is a reply the listing
/// cannot be checked against.
#[tokio::test]
async fn name_reply_without_info_key_could_not_be_checked() {
    let l = listing();
    let mut name = name_value();
    name.as_object_mut().unwrap().remove("info");
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            name,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    match v {
        Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
            assert!(reason.contains("info"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

/// A named change to a coin reply.
type CoinEdit = fn(&mut Value);

/// A FINALIZE covenant whose name or height item is missing or not the
/// encoding hsd writes says nothing about the listing: it could not be
/// checked. Only a well-formed item that differs fails it (the two tests
/// above).
#[tokio::test]
async fn lock_coin_covenant_without_readable_name_or_height_could_not_be_checked() {
    let cases: [(&str, CoinEdit); 4] = [
        ("height missing", |c| {
            c["covenant"]["items"] =
                Value::Array(c["covenant"]["items"].as_array().unwrap()[..1].to_vec())
        }),
        ("height not hex", |c| {
            c["covenant"]["items"][1] = "zz".into()
        }),
        ("height not 4 bytes", |c| {
            c["covenant"]["items"][1] = "0100".into()
        }),
        ("name missing", |c| {
            c["covenant"]["items"] =
                Value::Array(c["covenant"]["items"].as_array().unwrap()[..2].to_vec())
        }),
    ];
    for (case, edit) in cases {
        let l = listing();
        let mut c = coin_value();
        edit(&mut c);
        let v = verdict(&mock(Some(c), Some(late_mtp(&l)), tip_from_fixture()), &l).await;
        match v {
            Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
                assert!(reason.contains("covenant"), "{case}: {reason}")
            }
            other => panic!("{case}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn name_owned_by_other_output_index_fails_verification() {
    let l = listing();
    let mut n = name_value();
    n["info"]["owner"]["index"] = 1.into();
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            n,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

#[tokio::test]
async fn no_current_step_is_not_buyable_yet() {
    let l = listing();
    let early_mtp = l.steps[0].lock_time.saturating_sub(3_600);
    let v = verdict(
        &mock(Some(coin_value()), Some(early_mtp), tip_from_fixture()),
        &l,
    )
    .await;
    match v {
        Verdict::Hidden(Hidden::NotYetValid {
            first_valid_in_secs,
        }) => {
            // lockTime 1_783_696_480 is valid once MTP passes it rounded down
            // to 512 s (1_783_696_384), i.e. from 1_783_696_385; the MTP here
            // is 1_783_692_880.
            assert_eq!(l.steps[0].lock_time, 1_783_696_480, "fixture");
            assert_eq!(first_valid_in_secs, 3_505)
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn missing_mtp_could_not_check() {
    let l = listing();
    let v = verdict(&mock(Some(coin_value()), None, tip_from_fixture()), &l).await;
    assert!(
        matches!(&v, Verdict::Hidden(Hidden::CouldNotCheck { reason }) if reason.contains("median time")),
        "{v:?}"
    );
}

#[tokio::test]
async fn unreachable_node_could_not_check() {
    let l = listing();
    let v = verdict(&MockNodeRpc::new(), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::CouldNotCheck { .. })),
        "{v:?}"
    );
}

#[tokio::test]
async fn name_about_to_expire_is_hidden() {
    let l = listing();
    let end = 337_627 + 105_120;
    // Inside the finalize margin (1 + lockup 288 + one day 144).
    let tip = (end - 400) as i64;
    let v = verdict(&mock(Some(coin_value()), Some(late_mtp(&l)), tip), &l).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::ExpiresBeforeFinalize)),
        "{v:?}"
    );
}

#[tokio::test]
async fn name_expiring_within_a_month_is_buyable_with_warning() {
    let l = listing();
    let end = 337_627 + 105_120;
    let tip = (end - 144 * 10) as i64;
    let v = verdict(&mock(Some(coin_value()), Some(late_mtp(&l)), tip), &l).await;
    match v {
        Verdict::Buyable(b) => assert!(b.warn_expiry),
        other => panic!("{other:?}"),
    }
}

#[test]
fn expiring_name_blocked_and_warned_mainnet() {
    let p = Network::Main.name_params();
    let tip = 100_000;
    let (ok, _) = verify::finalize_deadline_ok(tip + 1 + 288 + 144, tip, &p);
    assert!(!ok, "expiring exactly at the margin is refused");
    let (ok, warn) = verify::finalize_deadline_ok(tip + 1 + 288 + 144 + 1, tip, &p);
    assert!(ok && warn);
    let (ok, warn) = verify::finalize_deadline_ok(tip + 100_000, tip, &p);
    assert!(ok && !warn);
}

/// The warning lasts 30 days past the margin and ends exactly there.
#[test]
fn expiry_warning_ends_thirty_days_past_the_margin() {
    let p = Network::Main.name_params();
    let tip = 100_000;
    let last_warned = tip + 1 + 288 + 144 + 30 * 144;
    assert_eq!(
        verify::finalize_deadline_ok(last_warned, tip, &p),
        (true, true)
    );
    assert_eq!(
        verify::finalize_deadline_ok(last_warned + 1, tip, &p),
        (true, false)
    );
}

#[test]
fn expiring_name_regtest_margin_is_relative() {
    let p = Network::Regtest.name_params();
    let tip = 1_000;
    assert!(!verify::finalize_deadline_ok(tip + 1 + 10 + 10, tip, &p).0);
    assert!(verify::finalize_deadline_ok(tip + 1 + 10 + 10 + 1, tip, &p).0);
}

#[test]
fn claimed_name_inside_claim_period_does_not_expire_early() {
    // Regtest: renewal window 5000, claim period 250 000 (hsd networks.js).
    let p = Network::Regtest.name_params();
    assert_eq!(p.expiry_end(100, true), 250_000);
    assert_eq!(p.expiry_end(300_000, true), 305_000);
    assert_eq!(p.expiry_end(100, false), 5_100);
}

// --- regtest listings signed in the tests -----------------------------------

use crate::tests::shakedex_cmd_tests::{
    regtest_listing_with, RegtestListing, REGTEST_MTP, REGTEST_TIP,
};

fn regtest_mock(r: &RegtestListing, coin: Value) -> MockNodeRpc {
    mock_with(
        Some(coin),
        r.name_info.clone(),
        Some(REGTEST_MTP),
        REGTEST_TIP,
    )
}

async fn regtest_verdict(r: &RegtestListing, coin: Value) -> Verdict {
    let l = ListingFile::parse(&r.json, Network::Regtest).unwrap();
    verify::verify_listing(&regtest_mock(r, coin), Network::Regtest, &l).await
}

#[tokio::test]
async fn zero_value_lock_coin_is_buyable() {
    // A name won with a single bid pays nothing at reveal: its coin is worth 0.
    let r = regtest_listing_with(0, &[(5_000_000, REGTEST_MTP - 100_000)]);
    match regtest_verdict(&r, r.coin.clone()).await {
        Verdict::Buyable(b) => assert_eq!(b.lock_value, 0),
        other => panic!("{other:?}"),
    }
}

/// R12: the next step is the earliest step not valid yet that is cheaper
/// than the current one; a dearer future step is never "next".
#[tokio::test]
async fn next_step_is_the_earliest_cheaper_step_not_valid_yet() {
    let r = regtest_listing_with(
        0,
        &[
            (9_000_000, REGTEST_MTP - 100_000),
            (5_000_000, REGTEST_MTP + 2_000),
            (3_000_000, REGTEST_MTP + 5_000),
            (10_000_000, REGTEST_MTP + 1_000),
        ],
    );
    match regtest_verdict(&r, r.coin.clone()).await {
        Verdict::Buyable(b) => assert_eq!((b.current_step, b.next_step), (0, Some(1))),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn negative_value_lock_coin_fails_verification() {
    let r = regtest_listing_with(0, &[(5_000_000, REGTEST_MTP - 100_000)]);
    let mut c = r.coin.clone();
    c["value"] = (-1).into();
    let v = regtest_verdict(&r, c).await;
    assert!(
        matches!(&v, Verdict::Hidden(Hidden::FailedVerification { reason }) if reason.contains("negative")),
        "{v:?}"
    );
}

#[tokio::test]
async fn lock_coin_at_another_witness_version_fails_verification() {
    // The lock's 32-byte program under witness version 1: same program,
    // not the lock's address.
    use bech32::{segwit, Hrp};
    let r = regtest_listing_with(1_000_000, &[(5_000_000, REGTEST_MTP - 100_000)]);
    let l = ListingFile::parse(&r.json, Network::Regtest).unwrap();
    let program = crate::noncustodial::shakedex::script::lock_program(&l.public_key);
    let hrp = Hrp::parse(Network::Regtest.address_hrp()).unwrap();
    let v1 = segwit::encode(hrp, segwit::VERSION_1, &program).unwrap();
    let mut c = r.coin.clone();
    c["address"] = v1.into();
    let v = regtest_verdict(&r, c).await;
    assert!(
        matches!(v, Verdict::Hidden(Hidden::FailedVerification { .. })),
        "{v:?}"
    );
}

// --- transfer_commits_to ------------------------------------------------------

/// A coin with covenant `kind` whose items 2–3 name `version` and `hash`.
fn covenant_coin(kind: u8, version: u8, hash: [u8; 20]) -> NodeCoin {
    serde_json::from_value(serde_json::json!({
        "hash": "33".repeat(32),
        "index": 0,
        "value": 0,
        "address": "hs1qlock",
        "covenant": {
            "type": kind,
            "items": ["aa".repeat(32), "00000000", hex::encode([version]), hex::encode(hash)],
        },
    }))
    .unwrap()
}

#[test]
fn a_transfer_commits_only_to_the_address_in_its_covenant() {
    use crate::noncustodial::address;
    use crate::noncustodial::shakedex::purchase::transfer_commits_to;
    use crate::noncustodial::sync::{COV_FINALIZE, COV_TRANSFER};
    let ours = address::encode_p2wpkh(Network::Main, &[7; 20]).unwrap();
    let theirs = address::encode_p2wpkh(Network::Main, &[8; 20]).unwrap();
    let transfer = covenant_coin(COV_TRANSFER, 0, [7; 20]);
    assert!(transfer_commits_to(&transfer, Network::Main, &ours).unwrap());
    assert!(!transfer_commits_to(&transfer, Network::Main, &theirs).unwrap());
    // Same hash under another witness version is another address.
    assert!(!transfer_commits_to(
        &covenant_coin(COV_TRANSFER, 1, [7; 20]),
        Network::Main,
        &ours
    )
    .unwrap());
    // The coin is our purchase's own TRANSFER output, so any other covenant
    // type is not hsd's answer: an error, never "commits elsewhere".
    for kind in [COV_FINALIZE, 0] {
        assert!(
            transfer_commits_to(&covenant_coin(kind, 0, [7; 20]), Network::Main, &ours).is_err()
        );
    }
    // A destination that does not decode is our own fault, not a verdict.
    assert!(transfer_commits_to(&transfer, Network::Main, "not an address").is_err());
}

/// A coin whose covenant is missing, or a TRANSFER without its address
/// items, proves nothing: an error, never "commits elsewhere".
#[test]
fn a_transfer_without_readable_covenant_is_an_error_not_a_no() {
    use crate::noncustodial::address;
    use crate::noncustodial::shakedex::purchase::transfer_commits_to;
    let ours = address::encode_p2wpkh(Network::Main, &[7; 20]).unwrap();
    let mut no_covenant = covenant_coin(crate::noncustodial::sync::COV_TRANSFER, 0, [7; 20]);
    no_covenant.covenant = None;
    assert!(transfer_commits_to(&no_covenant, Network::Main, &ours).is_err());
    let mut short = covenant_coin(crate::noncustodial::sync::COV_TRANSFER, 0, [7; 20]);
    short.covenant.as_mut().unwrap().items.truncate(3);
    assert!(transfer_commits_to(&short, Network::Main, &ours).is_err());
}

/// Without the name's renewal height its expiry is unknown: the listing could
/// not be checked. It is not "expires before finalize", which would be a
/// guess presented as a fact.
#[tokio::test]
async fn name_without_renewal_height_could_not_be_checked() {
    let l = listing();
    let mut name = name_value();
    name["info"].as_object_mut().unwrap().remove("renewal");
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            name,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    match v {
        Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
            assert!(reason.contains("renewal"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

/// Without the name's height the lock coin cannot be told from a leftover of
/// an earlier registration: the listing could not be checked. It is not "the
/// name has changed hands", which would be a guess presented as a fact.
#[tokio::test]
async fn name_without_height_could_not_be_checked() {
    let l = listing();
    let mut name = name_value();
    name["info"].as_object_mut().unwrap().remove("height");
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            name,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    match v {
        Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
            assert!(reason.contains("height"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

/// Without `claimed` the name's expiry is unknown (a claimed name expires
/// later): the listing could not be checked, rather than hidden as "expires
/// before finalize" on a guess.
#[tokio::test]
async fn name_without_claimed_could_not_be_checked() {
    let l = listing();
    let mut name = name_value();
    name["info"].as_object_mut().unwrap().remove("claimed");
    let v = verdict(
        &mock_with(
            Some(coin_value()),
            name,
            Some(late_mtp(&l)),
            tip_from_fixture(),
        ),
        &l,
    )
    .await;
    match v {
        Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
            assert!(reason.contains("claimed"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

/// A tip that is not a block height (negative, or past u32) is not read as
/// block 0, which would move every deadline the verdict checks.
#[tokio::test]
async fn unreadable_tip_could_not_be_checked() {
    let l = listing();
    for tip in [-1, i64::from(u32::MAX) + 1] {
        let v = verdict(&mock(Some(coin_value()), Some(late_mtp(&l)), tip), &l).await;
        match v {
            Verdict::Hidden(Hidden::CouldNotCheck { reason }) => {
                assert!(reason.contains("height"), "{tip}: {reason}")
            }
            other => panic!("{tip}: {other:?}"),
        }
    }
}
