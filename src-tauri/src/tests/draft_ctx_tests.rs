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
