//! R17: lock keys come from the seed (ADR 0004).

use rusqlite::{params, Connection};
use secp256k1::{PublicKey, SECP256K1};
use serde_json::Value;

use crate::db::queries::get_profile_addresses;
use crate::error::AppError;
use crate::noncustodial::derivation::{self, BRANCH_CHANGE, BRANCH_RECEIVE};
use crate::noncustodial::hd::{ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::noncustodial::send::load_spendable_coins;
use crate::noncustodial::shakedex::lock_key::{
    derive_lock_key, lock_key_index, lock_key_path, LOCK_BRANCH,
};
use crate::noncustodial::shakedex::script;
use crate::tests::hsd_parity_tests::master_from_known_mnemonic;

const VECTORS: &str = include_str!("../../tests/vectors/vectors.json");

/// hsd's path notation: `m/44'/5353'/0'/2'/1433607229'`.
fn path_string(path: &[u32; 5]) -> String {
    let mut s = String::from("m");
    for &i in path {
        if i >= HARDENED_OFFSET {
            s.push_str(&format!("/{}'", i - HARDENED_OFFSET));
        } else {
            s.push_str(&format!("/{i}"));
        }
    }
    s
}

fn network_named(name: &str) -> Network {
    match name {
        "main" => Network::Main,
        "regtest" => Network::Regtest,
        other => panic!("vector network {other}"),
    }
}

#[test]
fn golden_path() {
    let master = master_from_known_mnemonic();

    // R17's table, copied from the spec: the source of truth.
    assert_eq!(lock_key_index("dexreviews").unwrap(), 1_433_607_229);
    for (network, path, address) in [
        (
            Network::Main,
            "m/44'/5353'/0'/2'/1433607229'",
            "hs1qkq6yytu0c4puw0xlrdwedylkk5a04yrgecgz9wgyu2sua3mzh4jqt74rdj",
        ),
        (
            Network::Regtest,
            "m/44'/5355'/0'/2'/1433607229'",
            "rs1qr3mxw2n44nxqf43jqr69d42nmypn84jgzxtxeafpxn4nq58tylzsrz9ec5",
        ),
    ] {
        let p = lock_key_path(network.coin_type(), 0, "dexreviews").unwrap();
        assert_eq!(path_string(&p), path, "{network:?}");
        let key = derive_lock_key(&master, network, 0, "dexreviews").unwrap();
        assert_eq!(key.address, address, "{network:?}");
    }

    // hsd's own derivation, for R17's name and for a name whose hash has the
    // top bit set (only the 31-bit mask keeps that one's index hardened-able).
    let v: Value = serde_json::from_str(VECTORS).unwrap();
    let entries = v["shakedex"]["lockPath"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["nameHashPrefix"].as_u64().unwrap() >= 0x8000_0000),
        "the vectors must include a name with the top bit set"
    );
    for e in entries {
        let name = e["name"].as_str().unwrap();
        let network = network_named(e["network"].as_str().unwrap());
        assert_eq!(
            lock_key_index(name).unwrap() as u64,
            e["index"].as_u64().unwrap(),
            "{name}"
        );
        let p = lock_key_path(network.coin_type(), 0, name).unwrap();
        assert_eq!(path_string(&p), e["path"].as_str().unwrap(), "{name}");
        let key = derive_lock_key(&master, network, 0, name).unwrap();
        assert_eq!(hex::encode(key.pubkey), e["lockPub"].as_str().unwrap());
        assert_eq!(key.address, e["lockAddress"].as_str().unwrap());
        assert_eq!(key.script, script::lock_script(&key.pubkey));
        assert_eq!(key.program, script::lock_program(&key.pubkey));
        assert_eq!(
            PublicKey::from_secret_key(SECP256K1, &key.secret).serialize(),
            key.pubkey,
            "the secret is the public key's"
        );
    }
}

#[test]
fn every_level_is_hardened() {
    let long = "a".repeat(63);
    for name in ["dexreviews", "namehold", "a", long.as_str()] {
        for account in [0, 1, HARDENED_OFFSET - 1] {
            for network in [Network::Main, Network::Regtest] {
                let p = lock_key_path(network.coin_type(), account, name).unwrap();
                assert!(
                    p.iter().all(|&i| i >= HARDENED_OFFSET),
                    "{name} account {account}: {}",
                    path_string(&p)
                );
                assert_eq!(p[0], HARDENED_OFFSET + 44);
                assert_eq!(p[1], HARDENED_OFFSET + network.coin_type());
                assert_eq!(p[2], HARDENED_OFFSET + account);
                assert_eq!(p[3], HARDENED_OFFSET + LOCK_BRANCH);
            }
        }
    }
    // An account that cannot be hardened is refused, not wrapped or clamped.
    for account in [HARDENED_OFFSET, u32::MAX] {
        assert!(matches!(
            lock_key_path(Network::Main.coin_type(), account, "dexreviews"),
            Err(AppError::InvalidInput(_))
        ));
    }
}

#[test]
fn lock_key_is_not_derivable_from_the_account_xpub() {
    let master = master_from_known_mnemonic();
    let path = lock_key_path(Network::Main.coin_type(), 0, "dexreviews").unwrap();
    let account = master.derive_path(&path[..3]).unwrap();
    let xpub = ExtendedPubKey::from_priv(&account);

    // Every step below the account node is hardened: an xpub cannot take it.
    assert!(matches!(
        xpub.derive_path(&path[3..]),
        Err(AppError::InvalidInput(_))
    ));
    assert!(matches!(
        xpub.derive_child(path[3]),
        Err(AppError::InvalidInput(_))
    ));

    // The non-hardened twin of the branch is a different key.
    let key = derive_lock_key(&master, Network::Main, 0, "dexreviews").unwrap();
    let twin = xpub
        .derive_path(&[LOCK_BRANCH, path[4] - HARDENED_OFFSET])
        .unwrap();
    assert_ne!(twin.compressed_pubkey(), key.pubkey);
}

#[test]
fn lock_key_of_an_invalid_name_is_refused() {
    let master = master_from_known_mnemonic();
    for name in ["DexReviews", "", "-dex", &"a".repeat(64)] {
        assert!(
            matches!(lock_key_index(name), Err(AppError::InvalidInput(_))),
            "{name:?}"
        );
        assert!(
            derive_lock_key(&master, Network::Main, 0, name).is_err(),
            "{name:?}"
        );
    }
}

/// A regtest mnemonic profile `p1` with its account xpub from the test phrase
/// and the usual 20-address receive and change windows.
fn regtest_profile_with_addresses() -> (Connection, ExtendedPubKey) {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    let master = master_from_known_mnemonic();
    let account = master
        .derive_path(&[
            HARDENED_OFFSET + 44,
            HARDENED_OFFSET + Network::Regtest.coin_type(),
            HARDENED_OFFSET,
        ])
        .unwrap();
    let xpub = ExtendedPubKey::from_priv(&account);
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
         VALUES ('p1', 'Seller', 'mnemonic_hot', 'regtest', ?1)",
        params![xpub.to_base58check(Network::Regtest)],
    )
    .unwrap();
    for branch in [BRANCH_RECEIVE, BRANCH_CHANGE] {
        derivation::ensure_addresses(&conn, "p1", 0, Network::Regtest, &xpub, branch, 20).unwrap();
    }
    (conn, xpub)
}

fn insert_liquid_coin(
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

#[test]
fn coins_at_a_lock_address_are_not_spendable() {
    let (conn, xpub) = regtest_profile_with_addresses();
    let master = master_from_known_mnemonic();
    let lock = derive_lock_key(&master, Network::Regtest, 0, "dexreviews").unwrap();

    // Sync never asks the node for the lock address's coins: it is not one of
    // the profile's derived addresses.
    let synced = get_profile_addresses(&conn, "p1").unwrap();
    assert_eq!(synced.len(), 40);
    assert!(!synced.contains(&lock.address));

    // Even a plain coin at the lock address recorded as liquid HNS (the worst
    // case) is not offered by coin selection; one at a receive address is.
    let receive0 = derivation::derive_one(Network::Regtest, &xpub, BRANCH_RECEIVE, 0).unwrap();
    insert_liquid_coin(
        &conn,
        0xaa,
        &receive0.address,
        &receive0.script_pubkey_hex,
        100_000,
    );
    insert_liquid_coin(
        &conn,
        0xbb,
        &lock.address,
        &format!("0020{}", hex::encode(lock.program)),
        900_000,
    );
    let coins = load_spendable_coins(&conn, "p1", None, Network::Regtest).unwrap();
    assert_eq!(coins.len(), 1, "only the receive-address coin");
    assert_eq!(coins[0].txid, hex::encode([0xaa; 32]));
    assert_eq!(coins[0].value, 100_000);
}
