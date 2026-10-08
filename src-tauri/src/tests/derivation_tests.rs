//! Address allocation outside `derivation.rs`'s own unit tests: reservation
//! (R21).

use crate::db::queries::list_receive_addresses;
use crate::noncustodial::derivation::{
    derive_one, next_unused_receive_address, reserve_receive_address, BRANCH_RECEIVE,
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
    assert_ne!(a, b);
    assert_eq!(a, receive(&xpub, 1));
    assert_eq!(b, receive(&xpub, 2));

    // Ordinary allocation skips both, and the receive list shows them used.
    let next = next_unused_receive_address(&conn, "p1", 0, Network::Main, &xpub).unwrap();
    assert_eq!(next.child_index, 3);
    let rows = list_receive_addresses(&conn, "p1", 0).unwrap();
    for addr in [&a, &b] {
        assert!(rows.iter().any(|r| &r.address == addr && r.used), "{addr}");
    }

    // Inside a caller's transaction (T2 reserves while it records a listing).
    let tx = conn.unchecked_transaction().unwrap();
    let c = reserve_receive_address(&tx, "p1").unwrap();
    tx.commit().unwrap();
    assert_eq!(c, receive(&xpub, 3));
}
