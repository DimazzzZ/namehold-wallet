use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::commands::secure_wallet::{gap_limit, provision_addresses};
use crate::noncustodial::hd::{ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::tests::hsd_parity_tests::master_from_known_mnemonic;

pub fn create_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .unwrap();
    let sql = include_str!("../../../src-tauri/src/sql/001_initial.sql");
    conn.execute_batch(sql).unwrap();
    let sql2 = include_str!("../../../src-tauri/src/sql/002_hsd_prefix.sql");
    conn.execute_batch(sql2).unwrap();
    let sql3 = include_str!("../../../src-tauri/src/sql/003_provider_modes.sql");
    conn.execute_batch(sql3).unwrap();
    conn
}

pub fn create_test_state() -> crate::AppState {
    let conn = create_test_db();
    crate::AppState {
        db: Mutex::new(conn),
        signer: Mutex::new(None),
        secure_prompts: Mutex::new(std::collections::HashMap::new()),
        hsd_child: Mutex::new(None),
        node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
        sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::commands::sync::SyncStatus::default(),
        )),
    }
}

/// Set one per-profile node-config override (ADR-001), upserting.
///
/// Four test files and the watched-name daemon's own test module each carried
/// their own copy of this. Four were byte-identical; the fifth used
/// `INSERT OR REPLACE`, which differs on an existing row — it rewrites the
/// whole row rather than the value, so a future column would silently be
/// reset. One copy, and the shapes cannot drift again.
pub fn set_profile_override(conn: &Connection, profile_id: &str, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO profile_settings (profile_id, key, value) VALUES (?1, ?2, ?3)
         ON CONFLICT(profile_id, key) DO UPDATE SET value = excluded.value",
        rusqlite::params![profile_id, key, value],
    )
    .unwrap();
}

/// A mnemonic profile `p1` on `network`, with the account 0 xpub of the test
/// phrase and the receive and change windows a new wallet gets.
pub fn mnemonic_profile_db(network: Network) -> (Connection, ExtendedPubKey) {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    let account = master_from_known_mnemonic()
        .derive_path(&[
            HARDENED_OFFSET + 44,
            HARDENED_OFFSET + network.coin_type(),
            HARDENED_OFFSET,
        ])
        .unwrap();
    let xpub = ExtendedPubKey::from_priv(&account);
    let xpub_str = xpub.to_base58check(network);
    let network_name = match network {
        Network::Main => "mainnet",
        Network::Regtest => "regtest",
        other => panic!("no profile network name for {other:?}"),
    };
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
         VALUES ('p1', 'Seller', 'mnemonic_hot', ?1, ?2)",
        params![network_name, xpub_str],
    )
    .unwrap();
    provision_addresses(&conn, "p1", network, &xpub_str, gap_limit(&HashMap::new())).unwrap();
    (conn, xpub)
}

/// Make `address` one of `profile_id`'s derived receive addresses, as sync
/// would have, so a coin recorded there is counted and selectable. Keeps an
/// address that is already derived as it is.
pub fn own_address(conn: &Connection, profile_id: &str, address: &str) {
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         SELECT ?1, 0, 0,
                (SELECT COALESCE(MAX(child_index) + 1, 0) FROM derived_addresses
                 WHERE wallet_profile_id = ?1 AND account_index = 0 AND branch = 0),
                ?2, '00', '00'
         WHERE NOT EXISTS (SELECT 1 FROM derived_addresses
                           WHERE wallet_profile_id = ?1 AND address = ?2)",
        params![profile_id, address],
    )
    .unwrap();
}

/// An unspent liquid HNS coin of `p1` at `address`, vout 0, whose txid is
/// `txid_byte` repeated.
pub fn insert_liquid_coin(
    conn: &Connection,
    txid_byte: u8,
    address: &str,
    script_hex: &str,
    value: u64,
) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class)
         VALUES (?1, 0, 'p1', ?2, ?3, ?4, 0, 'liquid_hns')",
        params![
            hex::encode([txid_byte; 32]),
            address,
            script_hex,
            value as i64
        ],
    )
    .unwrap();
}
