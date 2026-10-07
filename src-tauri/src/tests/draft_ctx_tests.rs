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
