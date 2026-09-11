//! Tests for `crate::commands::active_profile` — the one place that turns the
//! active wallet profile into a `Network` (defaulting to mainnet).

use crate::commands::active_profile::active_profile_network_from_conn;
use crate::db::queries::{insert_wallet_profile, set_active_profile};
use crate::noncustodial::network::Network;
use rusqlite::Connection;

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    conn
}

fn seed_profile(conn: &Connection, id: &str, network: &str) {
    insert_wallet_profile(
        conn,
        id,
        "Primary",
        "mnemonic_hot",
        network,
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
}

#[test]
fn defaults_to_mainnet_without_an_active_profile() {
    let conn = db();
    assert_eq!(active_profile_network_from_conn(&conn), Network::Main);
}

#[test]
fn reads_the_active_profile_network() {
    let conn = db();
    seed_profile(&conn, "p1", "regtest");
    set_active_profile(&conn, "p1").unwrap();
    assert_eq!(active_profile_network_from_conn(&conn), Network::Regtest);
}

#[test]
fn accepts_the_mainnet_spelling_used_by_the_profile_schema() {
    let conn = db();
    seed_profile(&conn, "p1", "mainnet");
    set_active_profile(&conn, "p1").unwrap();
    assert_eq!(active_profile_network_from_conn(&conn), Network::Main);
}

#[test]
fn falls_back_to_mainnet_for_an_unparseable_network_string() {
    let conn = db();
    // The schema's CHECK constraint keeps unknown networks out of the table, so
    // this row has to be smuggled past it: the fallback under test is defensive
    // (an older DB, a future network name), not something the app can produce.
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    seed_profile(&conn, "p1", "weirdnet");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .unwrap();
    set_active_profile(&conn, "p1").unwrap();
    // Same fallback the two deleted copies had: unknown → Main.
    assert_eq!(active_profile_network_from_conn(&conn), Network::Main);
}
