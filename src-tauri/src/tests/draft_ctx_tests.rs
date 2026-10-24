//! `draft_ctx::active_profile`: the active wallet profile every draft and
//! browse context starts from.

use rusqlite::Connection;

use crate::commands::draft_ctx::active_profile;
use crate::db::{self, queries};
use crate::error::AppError;

fn conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

#[test]
fn no_active_profile_is_refused() {
    let err = active_profile(&conn()).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m.contains("no active wallet profile")));
}

#[test]
fn an_active_id_without_its_profile_is_not_found() {
    let conn = conn();
    queries::set_active_profile(&conn, "gone").unwrap();
    let err = active_profile(&conn).unwrap_err();
    assert!(matches!(err, AppError::NotFound(m) if m.contains("gone")));
}

#[test]
fn the_active_profile_is_returned() {
    let conn = conn();
    queries::insert_wallet_profile(
        &conn,
        "w1",
        "Main",
        "ledger_hardware",
        "regtest",
        "xpub",
        0,
        false,
    )
    .unwrap();
    queries::set_active_profile(&conn, "w1").unwrap();
    let p = active_profile(&conn).unwrap();
    assert_eq!((p.id.as_str(), p.kind.as_str()), ("w1", "ledger_hardware"));
}

// --- fetch_name_state_strict ---------------------------------------------------

use crate::commands::draft_ctx::fetch_name_state_strict;
use crate::tests::mock_node_rpc::MockNodeRpc;
use serde_json::{json, Value};

fn info() -> Value {
    json!({"info": {
        "name": "dexreviews",
        "height": 150_000,
        "renewal": 160_000,
        "renewals": 2,
        "claimed": 0,
        "weak": false,
        "value": 1_000,
        "state": "closed",
    }})
}

async fn strict(reply: Value) -> Result<crate::commands::draft_ctx::NameState, AppError> {
    fetch_name_state_strict(&MockNodeRpc::new().with_name_info(reply), "dexreviews").await
}

/// The FINALIZE covenant is built from these fields: they are read exactly
/// as the node reports them.
#[tokio::test]
async fn strict_name_state_reads_the_covenant_fields() {
    let ns = strict(info()).await.unwrap();
    assert_eq!(
        (ns.height, ns.renewals, ns.claimed, ns.weak),
        (150_000, 2, 0, false)
    );
}

/// A field the covenant needs that the node leaves out, or reports out of
/// range, is refused rather than defaulted: a defaulted covenant is rejected
/// by the node only after the user has confirmed and signed it.
#[tokio::test]
async fn strict_name_state_refuses_a_missing_or_out_of_range_field() {
    for (field, value) in [
        ("height", None),
        ("renewals", None),
        ("claimed", None),
        ("weak", None),
        ("height", Some(json!(1u64 << 32))),
        ("height", Some(json!(-1))),
        ("weak", Some(json!("no"))),
    ] {
        let mut r = info();
        match value.clone() {
            None => {
                r["info"].as_object_mut().unwrap().remove(field);
            }
            Some(v) => r["info"][field] = v,
        }
        let err = strict(r).await.unwrap_err();
        assert!(err.to_string().contains(field), "{field} {value:?}: {err}");
    }
}

/// A reply without `info` is not hsd saying the name has no state (hsd
/// always sends the key): it is an RPC error, not an invalid input.
#[tokio::test]
async fn strict_name_state_without_info_key_is_an_rpc_error() {
    let err = strict(json!({})).await.unwrap_err();
    assert!(matches!(err, AppError::Rpc(_)), "{err:?}");
    let err = strict(json!({"info": null})).await.unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)), "{err:?}");
}

// --- consensus guards: RENEW, FINALIZE, REDEEM -------------------------------

use crate::commands::draft_ctx::{
    ensure_finalize_matured, ensure_renew_not_premature, exclude_owner_reveal,
};
use crate::db::queries::NameCoin;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::BlockchainInfo;
use crate::tests::mock_node_rpc::RpcCall;

const OWNER_TXID: &str = "80cfddda07cf7baedbafc078fd2a013deb2a323db389898da771f2d1041fcfdd";

/// A regtest `getnameinfo` reply, copied from the live regtest node for a
/// registered name with a transfer pending: renewed at 9735, transferred at
/// 9838, owner coin `OWNER_TXID:0`.
fn regtest_name_info() -> Value {
    json!({
        "start": {"reserved": false, "week": 17, "start": 34},
        "info": {
            "name": "shkreg2205014174",
            "nameHash": "03a3cdeb7bfad6b0b1a2dd6ac670d5a00445955122a750335a2e8772e24e8d59",
            "state": "CLOSED",
            "height": 9701,
            "renewal": 9735,
            "owner": {"hash": OWNER_TXID, "index": 0},
            "value": 500000,
            "highest": 1000000,
            "data": "000601117368616b656465782d636c692d73656c6c",
            "transfer": 9838,
            "revoked": 0,
            "claimed": 0,
            "renewals": 1,
            "registered": true,
            "expired": false,
            "weak": false,
            "stats": {
                "renewalPeriodStart": 9735,
                "renewalPeriodEnd": 14735,
                "blocksUntilExpire": 839,
                "daysUntilExpire": 5.83,
                "transferLockupStart": 9838,
                "transferLockupEnd": 9848,
                "blocksUntilValidFinalize": -4048,
                "hoursUntilValidFinalize": -674.67
            }
        }
    })
}

fn node_at(tip: i64, reply: Value) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_name_info(reply)
        .with_blockchain_info(BlockchainInfo {
            blocks: tip,
            ..Default::default()
        })
}

/// hsd refuses a RENEW while `height < renewal + treeInterval` (regtest 5),
/// judged at `tip + 1`: renewed at 9735, the first block it may be renewed in
/// is 9740, built on tip 9739.
#[tokio::test]
async fn renew_before_a_tree_interval_since_the_last_renewal_is_refused() {
    let node = node_at(9738, regtest_name_info());
    let err = ensure_renew_not_premature(&node, Network::Regtest, "shkreg2205014174")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("renewed in 1 block")),
        "{err:?}"
    );
    assert!(node
        .calls()
        .contains(&RpcCall::NameInfo("shkreg2205014174".into())));

    let node = node_at(9739, regtest_name_info());
    ensure_renew_not_premature(&node, Network::Regtest, "shkreg2205014174")
        .await
        .expect("renewable at tip + 1 = renewal + tree interval");
}

/// Without the renewal height there is no verdict, and no renewal on a guess.
#[tokio::test]
async fn renew_guard_without_the_renewal_height_is_an_rpc_error() {
    let mut reply = regtest_name_info();
    reply["info"].as_object_mut().unwrap().remove("renewal");
    let err = ensure_renew_not_premature(&node_at(20_000, reply), Network::Regtest, "x")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::Rpc(m) if m.contains("renewal")),
        "{err:?}"
    );
}

/// hsd refuses a FINALIZE while `height < transfer + transferLockup` (regtest
/// 10): transferred at 9838, finalizable in block 9848, built on tip 9847.
#[tokio::test]
async fn finalize_before_the_transfer_lockup_is_over_is_refused() {
    let node = node_at(9846, regtest_name_info());
    let err = ensure_finalize_matured(&node, Network::Regtest, "shkreg2205014174")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("still locked for 1 more block")),
        "{err:?}"
    );

    let node = node_at(9847, regtest_name_info());
    ensure_finalize_matured(&node, Network::Regtest, "shkreg2205014174")
        .await
        .expect("finalizable once the lockup is over");
}

/// `transfer: 0` is hsd saying no transfer is pending: a FINALIZE of it is
/// refused by the node (it asserts `ns.transfer !== 0`).
#[tokio::test]
async fn finalize_of_a_name_with_no_transfer_is_refused() {
    let mut reply = regtest_name_info();
    reply["info"]["transfer"] = json!(0);
    let err = ensure_finalize_matured(&node_at(20_000, reply), Network::Regtest, "x")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("no transfer")),
        "{err:?}"
    );
}

fn reveal(txid: &str, vout: u32) -> NameCoin {
    NameCoin {
        txid: txid.into(),
        vout,
        value: 1_000_000,
        address: "rs1qexample".into(),
        branch: 0,
        child_index: 0,
        covenant_type: crate::noncustodial::sync::COV_REVEAL as i64,
        covenant_json: None,
        name_height: Some(9701),
    }
}

/// hsd refuses to REDEEM the name's owner coin (`bad-redeem-owner`): the
/// winning reveal is dropped, a losing one on another outpoint of the same
/// transaction is kept.
#[tokio::test]
async fn redeem_never_spends_the_owner_coin_the_node_reports() {
    let node = node_at(20_000, regtest_name_info());
    let kept = exclude_owner_reveal(
        &node,
        "shkreg2205014174",
        vec![reveal(OWNER_TXID, 0), reveal(OWNER_TXID, 1)],
    )
    .await
    .unwrap();
    assert_eq!(
        kept.iter()
            .map(|c| (c.txid.as_str(), c.vout))
            .collect::<Vec<_>>(),
        vec![(OWNER_TXID, 1)]
    );
}

/// A wallet whose only reveal won has nothing to redeem: refused, rather than
/// a transaction the node rejects.
#[tokio::test]
async fn redeem_of_nothing_but_the_winning_reveal_is_refused() {
    let node = node_at(20_000, regtest_name_info());
    let err = exclude_owner_reveal(&node, "shkreg2205014174", vec![reveal(OWNER_TXID, 0)])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("won the auction")),
        "{err:?}"
    );
}

/// Without the node's owner there is no telling the winner from a loser.
#[tokio::test]
async fn redeem_guard_without_the_owner_is_an_rpc_error() {
    let mut reply = regtest_name_info();
    reply["info"].as_object_mut().unwrap().remove("owner");
    let err = exclude_owner_reveal(&node_at(20_000, reply), "x", vec![reveal(OWNER_TXID, 1)])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::Rpc(m) if m.contains("owner")),
        "{err:?}"
    );
}

/// No coins, no question for the node: the caller says there is nothing to
/// redeem.
#[tokio::test]
async fn redeem_guard_with_no_coins_asks_nothing() {
    let node = MockNodeRpc::new();
    assert!(exclude_owner_reveal(&node, "x", vec![])
        .await
        .unwrap()
        .is_empty());
    assert!(node.calls().is_empty());
}

// --- REDEEM: the window hsd takes one in -------------------------------------

use crate::commands::draft_ctx::{ensure_redeem_window_open, redeem_window, RedeemWindow};

/// [`regtest_name_info`] after the owner revoked the name in block 9850:
/// `getnameinfo` reports it REVOKED, with the stats hsd writes for that state
/// (`namestate.js` `toStats`: the revoke period ends `auctionMaturity` blocks
/// after the revoke, regtest 65), at a tip `blocks_until_reopen` blocks
/// before that end.
fn revoked_name_info(blocks_until_reopen: i64) -> Value {
    let mut reply = regtest_name_info();
    let info = reply["info"].as_object_mut().unwrap();
    info.insert("state".into(), json!("REVOKED"));
    info.insert("revoked".into(), json!(9850));
    info.insert("transfer".into(), json!(0));
    info.insert(
        "stats".into(),
        json!({
            "revokePeriodStart": 9850,
            "revokePeriodEnd": 9915,
            "blocksUntilReopen": blocks_until_reopen,
            "hoursUntilReopen": blocks_until_reopen as f64 / 6.0,
        }),
    );
    reply
}

/// [`regtest_name_info`] at a tip `blocks_until_expire` blocks before its
/// renewal window ends.
fn closed_name_info(blocks_until_expire: i64) -> Value {
    let mut reply = regtest_name_info();
    reply["info"]["stats"]["blocksUntilExpire"] = json!(blocks_until_expire);
    reply
}

/// hsd lets a losing bidder redeem "even in a revoked state" (`chain.js`,
/// REDEEM: only `state < CLOSED` is `bad-redeem-state`, and REVOKED sorts
/// after CLOSED), until the name expires. A transaction is judged at
/// `tip + 1`, and the name expires there once `blocksUntilReopen` (REVOKED)
/// or `blocksUntilExpire` (CLOSED) is down to 1: `maybeExpire` then resets
/// the name to a new auction height, and the REDEEM no longer matches it.
#[test]
fn the_redeem_window_is_read_off_the_state_and_its_stats() {
    let window = |reply: Value| {
        redeem_window(
            reply["info"]["state"].as_str().unwrap(),
            reply["info"].get("stats"),
        )
    };
    assert_eq!(window(revoked_name_info(30)), RedeemWindow::Open);
    assert_eq!(window(revoked_name_info(2)), RedeemWindow::Open);
    assert_eq!(window(revoked_name_info(1)), RedeemWindow::ExpiresNextBlock);
    assert_eq!(window(closed_name_info(2)), RedeemWindow::Open);
    assert_eq!(window(closed_name_info(1)), RedeemWindow::ExpiresNextBlock);
    for state in ["OPENING", "BIDDING", "REVEAL"] {
        assert_eq!(
            redeem_window(state, None),
            RedeemWindow::NotClosed,
            "{state}"
        );
    }
    let mut no_stats = revoked_name_info(30);
    no_stats["info"].as_object_mut().unwrap().remove("stats");
    assert_eq!(window(no_stats), RedeemWindow::Unknown);
}

/// The builder asks the node the same question: a REDEEM of a revoked name is
/// built, one the node would refuse in the next block is not, and a reply
/// that does not say when the name expires gives no verdict.
#[tokio::test]
async fn the_redeem_guard_takes_a_revoked_name_until_it_expires_with_the_next_block() {
    ensure_redeem_window_open(&node_at(9885, revoked_name_info(30)), "shkreg2205014174")
        .await
        .expect("a revoked name is redeemable");
    ensure_redeem_window_open(&node_at(9913, revoked_name_info(2)), "shkreg2205014174")
        .await
        .expect("still redeemable in the next block");
    for reply in [revoked_name_info(1), closed_name_info(1)] {
        let err = ensure_redeem_window_open(&node_at(9914, reply), "shkreg2205014174")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::InvalidInput(m) if m.contains("next block")),
            "{err:?}"
        );
    }
    let mut bidding = regtest_name_info();
    bidding["info"]["state"] = json!("BIDDING");
    let err = ensure_redeem_window_open(&node_at(9705, bidding), "x")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("not closed")),
        "{err:?}"
    );
    let mut no_stats = revoked_name_info(30);
    no_stats["info"].as_object_mut().unwrap().remove("stats");
    let err = ensure_redeem_window_open(&node_at(9885, no_stats), "x")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::Rpc(m) if m.contains("expires")),
        "{err:?}"
    );
}

// --- node_name_info: the reply every consensus guard reads ---------------------

/// hsd always sends `info`: a reply without the key is not its answer and is
/// an RPC error, while `info: null` is hsd saying the name has no state. Every
/// guard that reads the name's info tells the two apart the same way.
#[tokio::test]
async fn consensus_guards_tell_a_missing_info_key_from_a_name_with_no_state() {
    let coins = || vec![reveal(OWNER_TXID, 1)];
    for (reply, rpc) in [(json!({}), true), (json!({"info": null}), false)] {
        let node = node_at(20_000, reply.clone());
        let errs = [
            ensure_renew_not_premature(&node, Network::Regtest, "gone")
                .await
                .unwrap_err(),
            ensure_finalize_matured(&node, Network::Regtest, "gone")
                .await
                .unwrap_err(),
            ensure_redeem_window_open(&node, "gone").await.unwrap_err(),
            exclude_owner_reveal(&node, "gone", coins())
                .await
                .unwrap_err(),
        ];
        for err in errs {
            if rpc {
                assert!(
                    matches!(&err, AppError::Rpc(m) if m.contains("did not report the name's info for 'gone'")),
                    "{reply}: {err:?}"
                );
            } else {
                assert!(
                    matches!(&err, AppError::InvalidInput(m) if m.contains("'gone' has no on-chain state")),
                    "{reply}: {err:?}"
                );
            }
        }
    }
}

// --- recheck_lock_expiry ------------------------------------------------------

use crate::commands::draft_ctx::{recheck_lock_expiry, LockSend};

/// A lock draft sent without the node's tip cannot be judged again: the send
/// is refused with an RPC error that names the name, never let through.
#[tokio::test]
async fn lock_recheck_without_the_node_tip_is_an_rpc_error() {
    let node = MockNodeRpc::new()
        .with_name_info(regtest_name_info())
        .with_blockchain_info_err("connection refused");
    for send in [LockSend::Transfer, LockSend::Finalize] {
        let err = recheck_lock_expiry(&node, Network::Regtest, "shkreg2205014174", send)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::Rpc(m)
                if m.contains("node did not report its tip")
                    && m.contains("'shkreg2205014174'")
                    && m.contains("connection refused")),
            "{send:?}: {err:?}"
        );
    }
}
