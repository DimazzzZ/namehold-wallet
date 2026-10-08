//! Selling through Shakedex, day 0 (T2): the listings table, the lock
//! TRANSFER draft and its gates (R16, R18, R19, R21, R29, R31), and the abort
//! through the existing Cancel transfer.

use rusqlite::{params, Connection};

use crate::db::queries::{ListingMode, ListingState, ShakedexListing};
use crate::db::{self, queries};

const STORE_PROFILE: &str = "sell1";

fn store_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    queries::insert_wallet_profile(
        &conn,
        STORE_PROFILE,
        "Sell",
        "mnemonic_hot",
        "regtest",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    conn
}

fn listing(id: &str, name: &str, state: ListingState) -> ShakedexListing {
    ShakedexListing {
        id: id.into(),
        wallet_profile_id: STORE_PROFILE.into(),
        name: name.into(),
        mode: ListingMode::BuyNow,
        state,
        lock_pubkey_hex: "02".repeat(33),
        lock_transfer_draft_id: None,
        lock_transfer_txid: None,
        lock_txid: None,
        lock_vout: None,
        payment_address: Some("rs1qpay".into()),
        cancel_address: Some("rs1qcancel".into()),
        cancel_child_index: Some(2),
        steps_json: "[]".into(),
        listing_file_json: None,
        publish: false,
        market_status: None,
        market_retry_at: None,
        expires_at: None,
        abort_draft_id: None,
        sold_txid: None,
        cancel_txid: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn every_listing_state_round_trips_through_the_table() {
    let conn = store_conn();
    for (i, state) in ListingState::ALL.into_iter().enumerate() {
        // One open listing per name: a name per row.
        let l = listing(&format!("l{i}"), &format!("name{i}"), state);
        queries::insert_shakedex_listing(&conn, &l).unwrap();
        let got = queries::get_shakedex_listing(&conn, &l.id)
            .unwrap()
            .unwrap();
        assert_eq!(got.state, state);
        assert_eq!(got.mode, ListingMode::BuyNow);
        assert_eq!(got.cancel_child_index, Some(2));
    }
    let mut l = listing("ra", "auction", ListingState::Locking);
    l.mode = ListingMode::ReverseAuction;
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    assert_eq!(
        queries::get_shakedex_listing(&conn, "ra")
            .unwrap()
            .unwrap()
            .mode,
        ListingMode::ReverseAuction
    );
}

#[test]
fn an_unknown_listing_state_is_refused_when_read() {
    assert!("owned".parse::<ListingState>().is_err());
    assert_eq!(
        "cancel_awaiting_finalize".parse::<ListingState>().unwrap(),
        ListingState::CancelAwaitingFinalize
    );
    let conn = store_conn();
    let err = conn.execute(
        "INSERT INTO shakedex_listings (id, wallet_profile_id, name, mode, state, lock_pubkey_hex)
         VALUES ('x', ?1, 'n', 'buy_now', 'owned', '02')",
        params![STORE_PROFILE],
    );
    assert!(err.is_err(), "the CHECK refuses a spelling the enum lacks");
}

/// The CHECK lists exactly the spellings `as_str` writes: each one goes in
/// through plain SQL, and a spelling no variant has stays out.
#[test]
fn the_state_and_mode_checks_list_exactly_the_enum_spellings() {
    let conn = store_conn();
    let insert = |id: &str, mode: &str, state: &str| {
        conn.execute(
            "INSERT INTO shakedex_listings (id, wallet_profile_id, name, mode, state, lock_pubkey_hex)
             VALUES (?1, ?2, ?1, ?3, ?4, '02')",
            params![id, STORE_PROFILE, mode, state],
        )
    };
    for state in ListingState::ALL {
        insert(state.as_str(), ListingMode::BuyNow.as_str(), state.as_str())
            .unwrap_or_else(|e| panic!("{state:?} spelling '{}' refused: {e}", state.as_str()));
    }
    for mode in [ListingMode::BuyNow, ListingMode::ReverseAuction] {
        insert(&format!("m-{}", mode.as_str()), mode.as_str(), "locking")
            .unwrap_or_else(|e| panic!("{mode:?} spelling '{}' refused: {e}", mode.as_str()));
        assert_eq!(mode.as_str().parse::<ListingMode>().unwrap(), mode);
    }
    assert!(insert("bad-state", "buy_now", "Locking").is_err());
    assert!(insert("bad-state2", "buy_now", "locked").is_err());
    assert!(insert("bad-mode", "buyNow", "locking").is_err());
    assert!("buyNow".parse::<ListingMode>().is_err());
    // Every variant has a distinct spelling and parses back to itself.
    for state in ListingState::ALL {
        assert_eq!(state.as_str().parse::<ListingState>().unwrap(), state);
    }
}

/// The table's "one open listing per name" index, `is_terminal` and
/// `open_shakedex_listing_for_name` agree on which states are open.
#[test]
fn terminal_states_agree_with_the_open_listing_index() {
    for (i, first) in ListingState::ALL.into_iter().enumerate() {
        let conn = store_conn();
        queries::insert_shakedex_listing(&conn, &listing("a", "dup", first)).unwrap();
        let open = queries::open_shakedex_listing_for_name(&conn, STORE_PROFILE, "dup").unwrap();
        assert_eq!(open.is_some(), !first.is_terminal(), "{first:?}");
        let second =
            queries::insert_shakedex_listing(&conn, &listing("b", "dup", ListingState::Locking));
        assert_eq!(second.is_ok(), first.is_terminal(), "{i}: {first:?}");
    }
}

#[test]
fn a_lock_outpoint_is_tracked_once() {
    let conn = store_conn();
    let mut a = listing("a", "relist", ListingState::Cancelled);
    a.lock_txid = Some("aa".repeat(32));
    a.lock_vout = Some(0);
    queries::insert_shakedex_listing(&conn, &a).unwrap();
    let mut b = listing("b", "relist", ListingState::Listed);
    b.lock_txid = a.lock_txid.clone();
    b.lock_vout = Some(0);
    assert!(queries::insert_shakedex_listing(&conn, &b).is_err());
    b.lock_vout = Some(1);
    queries::insert_shakedex_listing(&conn, &b).unwrap();
}

#[test]
fn deleting_an_unsent_lock_draft_deletes_its_listing() {
    let conn = store_conn();
    queries::insert_tx_draft(
        &conn,
        "lockdraft",
        STORE_PROFILE,
        "shakedex_lock",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    let mut l = listing("l1", "gone", ListingState::Locking);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "lockdraft").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_none());
}

#[test]
fn deleting_a_draft_keeps_a_listing_past_locking() {
    let conn = store_conn();
    queries::insert_tx_draft(
        &conn,
        "lockdraft",
        STORE_PROFILE,
        "shakedex_lock",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    let mut l = listing("l1", "kept", ListingState::ReadyToFinalize);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "lockdraft").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_some());
}
