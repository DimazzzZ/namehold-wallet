//! Tests for `crate::commands::active_profile` — the one place that turns the
//! active wallet profile into a `Network` (defaulting to mainnet).

use crate::commands::active_profile::{
    active_profile_network_from_conn, active_profile_network_opt_from_conn,
};
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

/// The optional form keeps "no profile" distinguishable from "mainnet". Callers
/// for whom defaulting would be an action rather than a label — `start_hsd`,
/// which would otherwise begin a full mainnet chain sync — use this one.
#[test]
fn the_optional_form_reports_no_profile_as_none() {
    let conn = db();
    assert_eq!(active_profile_network_opt_from_conn(&conn), None);
    assert_eq!(
        active_profile_network_from_conn(&conn),
        Network::Main,
        "the defaulting form is unchanged for callers that only label"
    );
}

#[test]
fn the_optional_form_reads_the_active_profile_network() {
    let conn = db();
    seed_profile(&conn, "p1", "regtest");
    set_active_profile(&conn, "p1").unwrap();
    assert_eq!(
        active_profile_network_opt_from_conn(&conn),
        Some(Network::Regtest)
    );
}

// --- profile_network_opt_from_conn: a named profile, unknown means unknown ---

#[test]
fn a_named_profiles_network_is_read_without_the_active_profile() {
    // The sync steps work on a profile id they were handed, which need not be
    // the active one.
    let conn = db();
    seed_profile(&conn, "p-regtest", "regtest");
    seed_profile(&conn, "p-main", "mainnet");
    set_active_profile(&conn, "p-main").unwrap();
    assert_eq!(
        crate::commands::active_profile::profile_network_opt_from_conn(&conn, "p-regtest"),
        Some(Network::Regtest)
    );
}

#[test]
fn a_missing_profile_reads_as_unknown_not_mainnet() {
    let conn = db();
    assert_eq!(
        crate::commands::active_profile::profile_network_opt_from_conn(&conn, "nobody"),
        None
    );
}

#[test]
fn an_unparseable_network_string_reads_as_unknown_not_mainnet() {
    let conn = db();
    // Same smuggling as the fallback test above: the schema's CHECK keeps
    // unknown networks out, so this branch is defensive (an older DB, a future
    // network name) rather than a state the app can produce.
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    seed_profile(&conn, "p1", "weirdnet");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .unwrap();
    assert_eq!(
        crate::commands::active_profile::profile_network_opt_from_conn(&conn, "p1"),
        None,
        "unknown must not resolve to mainnet for a step that acts on the answer"
    );
}
