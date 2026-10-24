//! Address allocation outside `derivation.rs`'s own unit tests: reservation
//! (R21).

use crate::db::queries::list_receive_addresses;
use crate::error::AppError;
use crate::noncustodial::derivation::{
    adopt_receive_address, derive_one, next_unused_receive_address, reserve_receive_address,
    BRANCH_RECEIVE,
};
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;
use crate::tests::command_helpers::{insert_liquid_coin, mnemonic_profile_db};

fn receive(xpub: &ExtendedPubKey, index: u32) -> String {
    derive_one(Network::Main, xpub, BRANCH_RECEIVE, index)
        .unwrap()
        .address
}

#[test]
fn reserved_addresses_are_not_reissued() {
    let (conn, xpub) = mnemonic_profile_db(Network::Main);
    // Index 0 already received coins.
    let r0 = receive(&xpub, 0);
    insert_liquid_coin(&conn, 0xaa, &r0, "00", 1000);

    // Two reservations give two addresses, next to the used range.
    let a = reserve_receive_address(&conn, "p1").unwrap();
    let b = reserve_receive_address(&conn, "p1").unwrap();
    assert_ne!(a.address, b.address);
    assert_eq!(a.address, receive(&xpub, 1));
    assert_eq!(b.address, receive(&xpub, 2));
    assert_eq!((a.branch, a.child_index), (BRANCH_RECEIVE, 1));
    assert_eq!((b.branch, b.child_index), (BRANCH_RECEIVE, 2));

    // Ordinary allocation skips both, and the receive list shows them used.
    let next = next_unused_receive_address(&conn, "p1", 0, Network::Main, &xpub).unwrap();
    assert_eq!(next.child_index, 3);
    let rows = list_receive_addresses(&conn, "p1", 0).unwrap();
    for addr in [&a.address, &b.address] {
        assert!(rows.iter().any(|r| &r.address == addr && r.used), "{addr}");
    }

    // Inside a caller's transaction (T2 reserves while it records a listing).
    let tx = conn.unchecked_transaction().unwrap();
    let c = reserve_receive_address(&tx, "p1").unwrap();
    tx.commit().unwrap();
    assert_eq!(c.address, receive(&xpub, 3));
    assert_eq!((c.branch, c.child_index), (BRANCH_RECEIVE, 3));
}

/// Make every "mark used" fail, as a write error would.
fn refuse_marking_used(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TRIGGER no_mark_used BEFORE UPDATE OF used ON derived_addresses
         BEGIN SELECT RAISE(ABORT, 'mark used refused'); END;",
    )
    .unwrap();
}

fn stored_addresses(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM derived_addresses", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_reservation_for_a_missing_profile_fails_and_leaves_no_open_savepoint() {
    let (conn, _xpub) = mnemonic_profile_db(Network::Main);
    match reserve_receive_address(&conn, "nope") {
        Err(AppError::InvalidInput(m)) => assert_eq!(m, "wallet profile not found"),
        other => panic!("expected not found, got {other:?}"),
    }
    assert!(conn.is_autocommit(), "the savepoint was released");
}

#[test]
fn a_failed_reservation_rolls_back_the_address_it_derived() {
    let (conn, _xpub) = mnemonic_profile_db(Network::Main);
    let before = stored_addresses(&conn);
    refuse_marking_used(&conn);
    let err = reserve_receive_address(&conn, "p1").unwrap_err();
    assert!(err.to_string().contains("mark used refused"), "{err}");
    assert_eq!(stored_addresses(&conn), before, "nothing persisted");
    assert!(conn.is_autocommit());
}

#[test]
fn a_failed_adoption_rolls_back_the_addresses_it_derived() {
    let (conn, xpub) = mnemonic_profile_db(Network::Main);
    let before = stored_addresses(&conn);
    refuse_marking_used(&conn);
    let err = adopt_receive_address(&conn, "p1", &receive(&xpub, 30), 40).unwrap_err();
    assert!(err.to_string().contains("mark used refused"), "{err}");
    assert_eq!(stored_addresses(&conn), before, "nothing persisted");
    assert!(conn.is_autocommit());
}
