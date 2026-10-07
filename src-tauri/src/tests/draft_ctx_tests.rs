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
