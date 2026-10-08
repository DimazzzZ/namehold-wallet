//! Address allocation outside `derivation.rs`'s own unit tests: reservation
//! (R21).

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::commands::secure_wallet::{gap_limit, provision_addresses};
use crate::db::queries::{get_profile_addresses, list_receive_addresses};
use crate::noncustodial::derivation::{
    derive_one, next_unused_receive_address, reserve_receive_address, BRANCH_RECEIVE,
};
use crate::noncustodial::hd::{ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::tests::hsd_parity_tests::master_from_known_mnemonic;

/// A mainnet mnemonic profile `p1` on the test phrase's account 0 xpub, with
/// the receive and change windows a new wallet gets.
fn profile_db() -> (Connection, ExtendedPubKey) {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    let account = master_from_known_mnemonic()
        .derive_path(&[
            HARDENED_OFFSET + 44,
            HARDENED_OFFSET + Network::Main.coin_type(),
            HARDENED_OFFSET,
        ])
        .unwrap();
    let xpub = ExtendedPubKey::from_priv(&account);
    let xpub_str = xpub.to_base58check(Network::Main);
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
         VALUES ('p1', 'Seller', 'mnemonic_hot', 'mainnet', ?1)",
        params![xpub_str],
    )
    .unwrap();
    provision_addresses(
        &conn,
        "p1",
        Network::Main,
        &xpub_str,
        gap_limit(&HashMap::new()),
    )
    .unwrap();
    (conn, xpub)
}

fn receive(xpub: &ExtendedPubKey, index: u32) -> String {
    derive_one(Network::Main, xpub, BRANCH_RECEIVE, index)
        .unwrap()
        .address
}

#[test]
fn reserved_addresses_are_not_reissued() {
    let (conn, xpub) = profile_db();
    // Index 0 already received coins.
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class)
         VALUES ('aa', 0, 'p1', ?1, '00', 1000, 0, 'liquid_hns')",
        params![receive(&xpub, 0)],
    )
    .unwrap();

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

    // A restore provisions the gap-limit window from index 0: the reserved
    // addresses sit right after the used range, so the scan reaches them.
    let (restored, _) = profile_db();
    let scanned = get_profile_addresses(&restored, "p1").unwrap();
    assert!(scanned.contains(&a) && scanned.contains(&b));

    // Inside a caller's transaction (T2 reserves while it records a listing).
    let tx = conn.unchecked_transaction().unwrap();
    let c = reserve_receive_address(&tx, "p1").unwrap();
    tx.commit().unwrap();
    assert_eq!(c, receive(&xpub, 3));
}
