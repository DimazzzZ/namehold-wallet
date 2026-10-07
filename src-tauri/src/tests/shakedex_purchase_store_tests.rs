//! Storage for Shakedex purchases: the table and its queries, the reserved
//! destination address, and the held-back change of an unconfirmed purchase.

use rusqlite::{params, Connection};

use crate::db::{self, queries};
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::send::load_spendable_coins;

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
const PROFILE: &str = "life1";
const COIN_TXID: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const PURCHASE_TXID: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const LOCK_TXID: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn seed() -> [u8; 64] {
    hd::seed_from_mnemonic(MNEMONIC, "").unwrap()
}

fn account_xpub() -> ExtendedPubKey {
    let path = hd::bip44_path(Network::Main, 0, 0, 0);
    let master = ExtendedPrivKey::from_seed(&seed()).unwrap();
    let account = master.derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account)
}

/// Receive address, script and pubkey hex for leaf 0/0.
fn leaf00() -> (String, String, String) {
    let (_sk, pk, addr) = hd::derive_address(Network::Main, &seed(), 0, 0, 0).unwrap();
    let spk = hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

/// Migrated in-memory DB with a profile, derived leaf 0/0 and one liquid coin.
fn seeded_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();

    let (addr, spk, pubkey) = leaf00();
    queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Life",
        "mnemonic_hot",
        "mainnet",
        &account_xpub().to_base58check(Network::Main),
        0,
        false,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, ?3, ?4)",
        params![PROFILE, addr, spk, pubkey],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, 5000000, 0, 'liquid_hns', NULL)",
        params![COIN_TXID, PROFILE, addr, spk],
    )
    .unwrap();
    conn
}

fn purchase(id: &str, state: &str, destination: &str) -> queries::ShakedexPurchase {
    queries::ShakedexPurchase {
        id: id.to_string(),
        wallet_profile_id: PROFILE.to_string(),
        name: "nameholdtest".to_string(),
        listing_json: "{\"listing\":true}".to_string(),
        lock_txid: LOCK_TXID.to_string(),
        lock_vout: 0,
        price_doos: 1_000_000,
        purchase_draft_id: format!("draft-{id}"),
        purchase_txid: PURCHASE_TXID.to_string(),
        destination_address: destination.to_string(),
        state: state.parse().unwrap(),
        purchase_height: None,
        blocks_remaining: None,
        missing_since_height: None,
        rebroadcast_count: 0,
        lost_reason: None,
        finalize_draft_id: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn migration_creates_table_and_round_trips_a_purchase() {
    let conn = seeded_conn();
    let p = purchase("p1", "pending_send", "hs1qdest");
    queries::insert_shakedex_purchase(&conn, &p).unwrap();

    let got = queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .expect("inserted purchase is found");
    assert_eq!(got.name, p.name);
    assert_eq!(got.listing_json, p.listing_json);
    assert_eq!(got.lock_txid, LOCK_TXID);
    assert_eq!(got.lock_vout, 0);
    assert_eq!(got.price_doos, 1_000_000);
    assert_eq!(got.purchase_draft_id, "draft-p1");
    assert_eq!(got.purchase_txid, PURCHASE_TXID);
    assert_eq!(got.destination_address, "hs1qdest");
    assert_eq!(got.state, crate::db::queries::PurchaseState::PendingSend);
    assert_eq!(got.rebroadcast_count, 0);
    assert_eq!(got.purchase_height, None);
    assert_eq!(got.finalize_draft_id, None);
    assert!(!got.created_at.is_empty());
    assert!(queries::get_shakedex_purchase(&conn, "nope")
        .unwrap()
        .is_none());

    queries::update_shakedex_purchase_state(
        &conn,
        "p1",
        &queries::PurchaseProgress {
            state: queries::PurchaseState::AwaitingFinalize,
            purchase_height: Some(120),
            blocks_remaining: Some(7),
            missing_since_height: None,
            rebroadcast_count: 2,
            lost_reason: None,
        },
    )
    .unwrap();
    let got = queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .unwrap();
    assert_eq!(
        got.state,
        crate::db::queries::PurchaseState::AwaitingFinalize
    );
    assert_eq!(got.purchase_height, Some(120));
    assert_eq!(got.blocks_remaining, Some(7));
    assert_eq!(got.rebroadcast_count, 2);
    assert_eq!(
        queries::list_open_shakedex_purchases(&conn, PROFILE)
            .unwrap()
            .len(),
        1,
        "awaiting_finalize is still open"
    );

    queries::set_shakedex_purchase_finalize_draft(&conn, "p1", "fin-draft").unwrap();
    let got = queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .unwrap();
    assert_eq!(got.finalize_draft_id.as_deref(), Some("fin-draft"));

    queries::update_shakedex_purchase_state(
        &conn,
        "p1",
        &queries::PurchaseProgress {
            state: queries::PurchaseState::Owned,
            purchase_height: Some(120),
            blocks_remaining: None,
            missing_since_height: None,
            rebroadcast_count: 2,
            lost_reason: None,
        },
    )
    .unwrap();
    assert!(queries::list_open_shakedex_purchases(&conn, PROFILE)
        .unwrap()
        .is_empty());

    // A lost purchase records its reason and is not open either.
    queries::insert_shakedex_purchase(&conn, &purchase("p2", "unconfirmed", "hs1qdest2")).unwrap();
    queries::update_shakedex_purchase_state(
        &conn,
        "p2",
        &queries::PurchaseProgress {
            state: queries::PurchaseState::Lost,
            purchase_height: None,
            blocks_remaining: None,
            missing_since_height: Some(130),
            rebroadcast_count: 1,
            lost_reason: Some("lock spent elsewhere".to_owned()),
        },
    )
    .unwrap();
    let lost = queries::get_shakedex_purchase(&conn, "p2")
        .unwrap()
        .unwrap();
    assert_eq!(lost.lost_reason.as_deref(), Some("lock spent elsewhere"));
    assert_eq!(lost.missing_since_height, Some(130));
    assert!(queries::list_open_shakedex_purchases(&conn, PROFILE)
        .unwrap()
        .is_empty());

    queries::delete_shakedex_purchase(&conn, "p1").unwrap();
    assert!(queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .is_none());
}

#[test]
fn second_open_purchase_of_same_lock_is_refused_by_the_db() {
    let conn = seeded_conn();
    queries::insert_shakedex_purchase(&conn, &purchase("p1", "unconfirmed", "hs1qa")).unwrap();
    assert!(
        queries::insert_shakedex_purchase(&conn, &purchase("p2", "unconfirmed", "hs1qb")).is_err(),
        "a second open purchase of the same lock outpoint must fail"
    );
    assert!(
        queries::insert_shakedex_purchase(&conn, &purchase("p3", "pending_send", "hs1qc")).is_err(),
        "pending_send counts as open too"
    );

    // Once the first is no longer pending/unconfirmed the lock may be bought again.
    queries::update_shakedex_purchase_state(
        &conn,
        "p1",
        &queries::PurchaseProgress {
            state: queries::PurchaseState::Lost,
            purchase_height: None,
            blocks_remaining: None,
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: Some("x".to_owned()),
        },
    )
    .unwrap();
    queries::insert_shakedex_purchase(&conn, &purchase("p4", "unconfirmed", "hs1qd")).unwrap();
}

#[test]
fn destination_address_counts_as_used() {
    let conn = seeded_conn();
    let xpub = account_xpub();

    let first =
        derivation::next_unused_receive_address(&conn, PROFILE, 0, Network::Main, &xpub).unwrap();
    // Leaf 0/0 holds the seeded coin, so the first unused one is already past it.
    let again =
        derivation::next_unused_receive_address(&conn, PROFILE, 0, Network::Main, &xpub).unwrap();
    assert_eq!(first.address, again.address, "unused address is stable");

    queries::insert_shakedex_purchase(&conn, &purchase("p1", "pending_send", &first.address))
        .unwrap();
    let next =
        derivation::next_unused_receive_address(&conn, PROFILE, 0, Network::Main, &xpub).unwrap();
    assert_ne!(
        next.address, first.address,
        "a purchase's destination must never be handed out again"
    );
}

#[test]
fn change_of_unconfirmed_purchase_is_not_spendable() {
    let conn = seeded_conn();
    let (addr, spk, _pk) = leaf00();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 1, ?2, ?3, ?4, 3000000, 0, 'liquid_hns', NULL)",
        params![PURCHASE_TXID, PROFILE, addr, spk],
    )
    .unwrap();

    let txids = |conn: &Connection| -> Vec<String> {
        load_spendable_coins(conn, PROFILE, None, Network::Main)
            .unwrap()
            .into_iter()
            .map(|c| c.txid)
            .collect()
    };

    assert!(txids(&conn).contains(&PURCHASE_TXID.to_string()));

    queries::insert_shakedex_purchase(&conn, &purchase("p1", "unconfirmed", "hs1qdest")).unwrap();
    let held = txids(&conn);
    assert!(
        !held.contains(&PURCHASE_TXID.to_string()),
        "change of an unconfirmed purchase is held back: {held:?}"
    );
    assert!(
        held.contains(&COIN_TXID.to_string()),
        "unrelated coins stay spendable: {held:?}"
    );

    queries::update_shakedex_purchase_state(
        &conn,
        "p1",
        &queries::PurchaseProgress {
            state: queries::PurchaseState::AwaitingFinalize,
            purchase_height: Some(10),
            blocks_remaining: None,
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: None,
        },
    )
    .unwrap();
    assert!(txids(&conn).contains(&PURCHASE_TXID.to_string()));
}

#[test]
fn deleting_an_unsent_purchase_draft_deletes_its_pending_purchase() {
    let conn = seeded_conn();
    queries::insert_tx_draft(
        &conn,
        "draft-p1",
        PROFILE,
        "shakedex_purchase",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    queries::insert_shakedex_purchase(&conn, &purchase("p1", "pending_send", "hs1qa")).unwrap();
    queries::delete_tx_draft(&conn, "draft-p1").unwrap();
    assert!(queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .is_none());
}

#[test]
fn deleting_a_draft_keeps_a_purchase_that_left_pending_send() {
    // Only a never-sent purchase goes with its draft; a later state is the
    // chain's to decide.
    let conn = seeded_conn();
    queries::insert_tx_draft(
        &conn,
        "draft-p1",
        PROFILE,
        "shakedex_purchase",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    queries::insert_shakedex_purchase(&conn, &purchase("p1", "unconfirmed", "hs1qa")).unwrap();
    queries::delete_tx_draft(&conn, "draft-p1").unwrap();
    assert!(queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .is_some());
}

#[test]
fn deleting_a_finalize_draft_clears_the_link_on_its_purchase() {
    let conn = seeded_conn();
    queries::insert_shakedex_purchase(&conn, &purchase("p1", "awaiting_finalize", "hs1qa"))
        .unwrap();
    queries::insert_tx_draft(
        &conn,
        "fin",
        PROFILE,
        "shakedex_purchase_finalize",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    queries::set_shakedex_purchase_finalize_draft(&conn, "p1", "fin").unwrap();
    queries::delete_tx_draft(&conn, "fin").unwrap();
    let p = queries::get_shakedex_purchase(&conn, "p1")
        .unwrap()
        .unwrap();
    assert_eq!(p.state, crate::db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.finalize_draft_id, None);
}

#[test]
fn every_purchase_state_round_trips_through_the_table() {
    use queries::PurchaseState::*;
    let conn = seeded_conn();
    for (i, state) in [PendingSend, Unconfirmed, AwaitingFinalize, Owned, Lost]
        .into_iter()
        .enumerate()
    {
        let mut p = purchase(&format!("p{i}"), "pending_send", "hs1qdest");
        p.state = state;
        p.lock_vout = i as i64; // one open purchase per lock outpoint
        queries::insert_shakedex_purchase(&conn, &p).unwrap();
        let got = queries::get_shakedex_purchase(&conn, &p.id)
            .unwrap()
            .unwrap();
        assert_eq!(got.state, state);
    }
}

#[test]
fn an_unknown_purchase_state_is_refused_when_read() {
    assert!("sold".parse::<queries::PurchaseState>().is_err());
    assert_eq!(
        "awaiting_finalize"
            .parse::<queries::PurchaseState>()
            .unwrap(),
        queries::PurchaseState::AwaitingFinalize
    );
}

/// A purchase lost after it paid keeps a `confirmed` draft, so Activity
/// learns of the loss from the draft list, not from the draft's status.
#[test]
fn draft_list_carries_the_reason_a_purchase_was_lost() {
    let conn = seeded_conn();
    for id in ["p1", "p2"] {
        queries::insert_tx_draft(
            &conn,
            &format!("draft-{id}"),
            PROFILE,
            "shakedex_purchase",
            "",
            "{}",
            "{}",
        )
        .unwrap();
    }
    queries::insert_shakedex_purchase(&conn, &purchase("p1", "awaiting_finalize", "hs1qa"))
        .unwrap();
    let mut lost = purchase("p2", "awaiting_finalize", "hs1qb");
    lost.lock_txid = "77".repeat(32);
    queries::insert_shakedex_purchase(&conn, &lost).unwrap();
    queries::update_shakedex_purchase_state(
        &conn,
        "p2",
        &queries::PurchaseProgress {
            state: queries::PurchaseState::Lost,
            purchase_height: Some(120),
            blocks_remaining: None,
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: Some("the name expired before it was finalized".into()),
        },
    )
    .unwrap();

    let drafts = queries::list_tx_drafts(&conn, PROFILE).unwrap();
    let reason = |id: &str| {
        drafts
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .purchase_lost_reason
            .clone()
    };
    assert_eq!(reason("draft-p1"), None, "an open purchase is not lost");
    assert_eq!(
        reason("draft-p2").as_deref(),
        Some("the name expired before it was finalized")
    );
}
