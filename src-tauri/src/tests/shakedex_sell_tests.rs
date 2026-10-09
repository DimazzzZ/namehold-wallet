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
        lock_finalize_draft_id: None,
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
        abort_txid: None,
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

/// R19: the lock TRANSFER is output 0 of its transaction, so only a cancel
/// spending `(lock_transfer_txid, 0)` is the listing's abort; one spending
/// another output of the same transaction links nothing.
#[test]
fn a_cancel_of_another_output_of_the_lock_tx_links_nothing() {
    let conn = store_conn();
    let lock_txid = "ab".repeat(32);
    let mut l = listing("l1", "dexsale", ListingState::Locking);
    l.lock_transfer_txid = Some(lock_txid.clone());
    queries::insert_shakedex_listing(&conn, &l).unwrap();

    let n = queries::link_shakedex_listing_abort(
        &conn,
        STORE_PROFILE,
        "dexsale",
        &lock_txid,
        1,
        "c1",
        &"c1".repeat(32),
    )
    .unwrap();
    assert_eq!(n, 0, "output 1 is not the lock TRANSFER");
    let got = queries::get_shakedex_listing(&conn, "l1").unwrap().unwrap();
    assert_eq!((got.abort_draft_id, got.abort_txid), (None, None));

    let n = queries::link_shakedex_listing_abort(
        &conn,
        STORE_PROFILE,
        "dexsale",
        &lock_txid,
        0,
        "c0",
        &"c0".repeat(32),
    )
    .unwrap();
    assert_eq!(n, 1, "output 0 is");
}

/// The SQL that picks the listings a Cancel transfer aborts is generated from
/// `ListingState::CANCEL_ABORTABLE`, so the two cannot drift apart.
#[test]
fn cancel_abortable_sql_lists_the_rust_states() {
    assert_eq!(
        ListingState::cancel_abortable_sql(),
        "('locking', 'ready_to_finalize')"
    );
    for state in ListingState::ALL {
        assert_eq!(
            state.aborts_by_cancel_transfer(),
            ListingState::cancel_abortable_sql().contains(&format!("'{}'", state.as_str())),
            "{state:?}"
        );
    }
}

/// The draft-status sets the listings read: a lock draft holds its name
/// while it is unsent or may have reached the chain, and may still land
/// until it is mined; `dropped` and `failed` do neither.
#[test]
fn draft_status_sets_partition_the_statuses() {
    for (status, unsent, holds, may_land) in [
        ("draft", true, true, true),
        ("signed", true, true, true),
        ("broadcast_pending", false, true, true),
        ("broadcasted", false, true, true),
        ("confirmed", false, true, false),
        ("dropped", false, false, false),
        ("failed", false, false, false),
    ] {
        assert_eq!(queries::never_sent(status), unsent, "{status}");
        assert_eq!(queries::lock_draft_holds_name(status), holds, "{status}");
        assert_eq!(
            queries::lock_draft_may_still_land(status),
            may_land,
            "{status}"
        );
        assert_eq!(queries::draft_may_still_land(status), may_land, "{status}");
        assert!(
            !(unsent && queries::may_have_reached_chain(status)),
            "{status}"
        );
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

/// A `dropped` or `failed` draft was broadcast (eviction grace, a timed-out
/// broadcast): its TRANSFER may still be mined, so deleting the draft is no
/// evidence the name never left, and the listing stays.
#[test]
fn deleting_a_broadcast_then_dropped_or_failed_draft_keeps_the_locking_listing() {
    for status in ["dropped", "failed"] {
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
        conn.execute(
            "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = 'lockdraft'",
            params![status],
        )
        .unwrap();
        let mut l = listing("l1", "maybe", ListingState::Locking);
        l.lock_transfer_draft_id = Some("lockdraft".into());
        queries::insert_shakedex_listing(&conn, &l).unwrap();
        queries::delete_tx_draft(&conn, "lockdraft").unwrap();
        assert!(
            queries::get_shakedex_listing(&conn, "l1")
                .unwrap()
                .is_some(),
            "{status}"
        );
    }
}

#[test]
fn deleting_a_signed_lock_draft_deletes_its_listing() {
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
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = 'signed' WHERE id = 'lockdraft'",
        [],
    )
    .unwrap();
    let mut l = listing("l1", "unsent", ListingState::Locking);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "lockdraft").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_none());
}

#[test]
fn deleting_another_draft_keeps_a_locking_listing() {
    let conn = store_conn();
    for id in ["lockdraft", "other"] {
        queries::insert_tx_draft(&conn, id, STORE_PROFILE, "shakedex_lock", "", "{}", "{}")
            .unwrap();
    }
    let mut l = listing("l1", "mine", ListingState::Locking);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "other").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_some());
}

fn insert_draft(conn: &Connection, id: &str, action: &str, status: &str) {
    queries::insert_tx_draft(conn, id, STORE_PROFILE, action, "00", "{}", "{}").unwrap();
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = ?2",
        params![status, id],
    )
    .unwrap();
}

fn finalizing(conn: &Connection, id: &str, draft_id: &str) -> usize {
    let tx = conn.unchecked_transaction().unwrap();
    let n = queries::mark_listing_finalizing_in_tx(
        &tx,
        id,
        &queries::FinalizingListing {
            finalize_draft_id: draft_id,
            lock_txid: &"f1".repeat(32),
            lock_vout: 0,
            steps_json: r#"[{"price":5000000,"lockTime":1,"signature":"ab"}]"#,
            listing_file_json: "{}",
            expires_at: 1_731_536_000,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    n
}

/// R19: only a listing ReadyToFinalize becomes Finalizing, and it then
/// carries its FINALIZE draft, the lock outpoint (output 0 of the FINALIZE),
/// the steps, the listing file and its expiry.
#[test]
fn finalizing_is_written_only_from_ready_to_finalize() {
    for state in ListingState::ALL {
        let conn = store_conn();
        queries::insert_shakedex_listing(&conn, &listing("l1", "dexsale", state)).unwrap();
        insert_draft(&conn, "fin", sell::LOCK_FINALIZE_ACTION, "signed");
        let n = finalizing(&conn, "l1", "fin");
        let l = queries::get_shakedex_listing(&conn, "l1").unwrap().unwrap();
        if state == ListingState::ReadyToFinalize {
            assert_eq!(n, 1);
            assert_eq!(l.state, ListingState::Finalizing);
            assert!(
                !l.state.aborts_by_cancel_transfer(),
                "out of CANCEL_ABORTABLE"
            );
            assert_eq!(l.lock_finalize_draft_id.as_deref(), Some("fin"));
            assert_eq!(
                (l.lock_txid.as_deref(), l.lock_vout),
                (Some("f1".repeat(32).as_str()), Some(0))
            );
            assert!(l.steps_json.contains("5000000"));
            assert_eq!(l.listing_file_json.as_deref(), Some("{}"));
            assert_eq!(l.expires_at, Some(1_731_536_000));
        } else {
            assert_eq!(n, 0, "{state:?}");
            assert_eq!(l.state, state);
            assert_eq!(l.lock_finalize_draft_id, None);
            assert_eq!(l.lock_txid, None);
            assert_eq!(l.steps_json, "[]");
        }
    }
}

/// Deviation 3: an unsent FINALIZE (draft or signed) deleted takes its lock
/// outpoint, steps and file with it; the listing is ReadyToFinalize again.
#[test]
fn deleting_an_unsent_finalize_draft_returns_the_listing_to_ready() {
    for status in ["draft", "signed"] {
        let conn = store_conn();
        queries::insert_shakedex_listing(
            &conn,
            &listing("l1", "dexsale", ListingState::ReadyToFinalize),
        )
        .unwrap();
        insert_draft(&conn, "fin", sell::LOCK_FINALIZE_ACTION, status);
        assert_eq!(finalizing(&conn, "l1", "fin"), 1);
        // Another listing with its own FINALIZE draft is out of scope.
        queries::insert_shakedex_listing(
            &conn,
            &listing("l2", "other", ListingState::ReadyToFinalize),
        )
        .unwrap();
        insert_draft(&conn, "fin2", sell::LOCK_FINALIZE_ACTION, "signed");
        assert_eq!(finalizing(&conn, "l2", "fin2"), 1);
        queries::delete_tx_draft(&conn, "fin").unwrap();
        let o = queries::get_shakedex_listing(&conn, "l2").unwrap().unwrap();
        assert_eq!(o.state, ListingState::Finalizing, "{status}");
        assert!(o.lock_txid.is_some() && o.steps_json.contains("5000000"));
        assert_eq!(o.lock_finalize_draft_id.as_deref(), Some("fin2"));
        let l = queries::get_shakedex_listing(&conn, "l1").unwrap().unwrap();
        assert_eq!(l.state, ListingState::ReadyToFinalize, "{status}");
        assert_eq!(
            (l.lock_txid, l.lock_vout, l.lock_finalize_draft_id),
            (None, None, None)
        );
        assert_eq!(l.steps_json, "[]");
        assert_eq!((l.listing_file_json, l.expires_at), (None, None));
    }
}

/// A FINALIZE that was sent (`failed`, `dropped`) may be deleted, but its
/// listing keeps its outpoint and steps: whether it landed is the chain's to
/// say (Step 6).
#[test]
fn deleting_a_failed_finalize_draft_keeps_its_steps_until_the_chain_says() {
    for status in ["failed", "dropped"] {
        let conn = store_conn();
        queries::insert_shakedex_listing(
            &conn,
            &listing("l1", "dexsale", ListingState::ReadyToFinalize),
        )
        .unwrap();
        insert_draft(&conn, "fin", sell::LOCK_FINALIZE_ACTION, "signed");
        assert_eq!(finalizing(&conn, "l1", "fin"), 1);
        conn.execute(
            "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = 'fin'",
            [status],
        )
        .unwrap();
        queries::delete_tx_draft(&conn, "fin").unwrap();
        let l = queries::get_shakedex_listing(&conn, "l1").unwrap().unwrap();
        assert_eq!(l.state, ListingState::Finalizing, "{status}");
        assert!(l.lock_txid.is_some());
        assert!(l.steps_json.contains("5000000"));
    }
}

/// Every other transition writes only from its expected previous state and
/// reports 0 rows otherwise.
#[test]
fn listing_transitions_write_only_from_their_previous_state() {
    type Transition = fn(&Connection, &str) -> Result<usize, crate::error::AppError>;
    let cases: [(&str, Transition, ListingState, ListingState); 5] = [
        (
            "ready",
            queries::mark_listing_ready,
            ListingState::Locking,
            ListingState::ReadyToFinalize,
        ),
        (
            "again",
            queries::mark_listing_locking_again,
            ListingState::ReadyToFinalize,
            ListingState::Locking,
        ),
        (
            "listed",
            queries::mark_listing_listed,
            ListingState::Finalizing,
            ListingState::Listed,
        ),
        (
            "fin again",
            queries::mark_listing_finalizing_again,
            ListingState::Listed,
            ListingState::Finalizing,
        ),
        (
            "revert",
            queries::revert_listing_to_ready,
            ListingState::Finalizing,
            ListingState::ReadyToFinalize,
        ),
    ];
    for (label, f, from, to) in cases {
        for state in ListingState::ALL {
            let conn = store_conn();
            queries::insert_shakedex_listing(&conn, &listing("l1", "dexsale", state)).unwrap();
            let n = f(&conn, "l1").unwrap();
            let l = queries::get_shakedex_listing(&conn, "l1").unwrap().unwrap();
            if state == from {
                assert_eq!((n, l.state), (1, to), "{label}");
            } else {
                assert_eq!((n, l.state), (0, state), "{label} from {state:?}");
            }
        }
    }
}

/// A FINALIZE into our lock from another device: only a listing whose abort
/// is still the Cancel transfer adopts the coin, as Restored.
#[test]
fn finalized_elsewhere_is_adopted_only_before_the_lock() {
    for state in ListingState::ALL {
        let conn = store_conn();
        let mut l = listing("l1", "dexsale", state);
        l.abort_draft_id = Some("cancel".into());
        l.abort_txid = Some("cd".repeat(32));
        queries::insert_shakedex_listing(&conn, &l).unwrap();
        let n = queries::adopt_lock_finalized_elsewhere(&conn, "l1", &"ab".repeat(32), 0).unwrap();
        let l = queries::get_shakedex_listing(&conn, "l1").unwrap().unwrap();
        if state.aborts_by_cancel_transfer() {
            assert_eq!((n, l.state), (1, ListingState::Restored));
            assert_eq!(l.lock_txid, Some("ab".repeat(32)));
            assert_eq!(l.lock_vout, Some(0));
            assert_eq!((l.abort_draft_id, l.abort_txid), (None, None));
        } else {
            assert_eq!((n, l.state), (0, state), "{state:?}");
            assert_eq!(l.abort_draft_id.as_deref(), Some("cancel"));
            assert_eq!(l.lock_txid, None);
        }
    }
}

/// The Finalizing/Listed source lists exactly those two states of a profile.
#[test]
fn finalizing_and_listed_listings_are_listed() {
    let conn = store_conn();
    for (id, name, state) in [
        ("l1", "a", ListingState::Finalizing),
        ("l2", "b", ListingState::Listed),
        ("l3", "c", ListingState::ReadyToFinalize),
        ("l4", "d", ListingState::Sold),
    ] {
        queries::insert_shakedex_listing(&conn, &listing(id, name, state)).unwrap();
    }
    let ids: Vec<String> = queries::list_shakedex_listings_finalizing(&conn, STORE_PROFILE)
        .unwrap()
        .into_iter()
        .map(|l| l.id)
        .collect();
    assert_eq!(ids, ["l1", "l2"]);
    assert!(queries::list_shakedex_listings_finalizing(&conn, "other")
        .unwrap()
        .is_empty());
}

/// The deadline scan's source: ReadyToFinalize listings only.
#[test]
fn listings_ready_to_finalize_are_listed_for_the_reminder() {
    let conn = store_conn();
    for (id, name, state) in [
        ("l1", "ready", ListingState::ReadyToFinalize),
        ("l2", "locking", ListingState::Locking),
        ("l3", "fin", ListingState::Finalizing),
    ] {
        let mut l = listing(id, name, state);
        l.lock_transfer_txid = Some(format!("{id}-tx"));
        queries::insert_shakedex_listing(&conn, &l).unwrap();
    }
    assert_eq!(
        queries::list_listings_ready_to_finalize(&conn).unwrap(),
        vec![(
            STORE_PROFILE.to_string(),
            "ready".to_string(),
            "l1-tx".to_string()
        )]
    );
}

// --- the lock command -------------------------------------------------------

use mockito::{Mock, ServerGuard};
use serde_json::{json, Value};
use tauri::Manager;

use crate::commands::shakedex::{
    build_lock_draft_inner, shakedex_build_lock_draft, ExpiryNotice, LockDraftInput,
};
use crate::noncustodial::actions::DraftPlan;
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::{self, ExtendedPrivKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::shakedex::lock_key::derive_lock_key;
use crate::noncustodial::shakedex::sell::{self, LOCK_ACTION, LOCK_COSTS};
use crate::noncustodial::sync::{COV_REGISTER, COV_REVEAL, COV_TRANSFER};
use crate::noncustodial::types::TxSummary;
use crate::tests::shakedex_cmd_tests::{
    app_with, err_text, mock_blockchain_info, mock_coin, mock_name_info, rpc_ok, seed, seeded, set,
    with_db, PROFILE,
};
use crate::AppState;

type App = tauri::App<tauri::test::MockRuntime>;

const NAME: &str = "dexsale";
const OWNER_TXID: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const NAME_HEIGHT: u32 = 50;
const NAME_VALUE: u64 = 1_000_000;
/// Regtest: renewal 1000 + window 5000.
const RENEWAL: u64 = 1_000;
const REGTEST_END: i64 = 6_000;
/// Far from expiry on regtest (end - tip = 4000 >= 1800).
const QUIET_TIP: i64 = 2_000;

fn net_of(network: &str) -> Network {
    derivation::network_from_profile(network).unwrap()
}

fn master() -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed()).unwrap()
}

/// Our receive address 0/0, where `seeded` puts the funding coin.
fn addr00(net: Network) -> (String, String) {
    let (_sk, pk, addr) = hd::derive_address(net, &seed(), 0, 0, 0).unwrap();
    (
        addr,
        hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap()),
    )
}

fn covenant_json(cov_type: u8, items: &[String]) -> String {
    json!({ "type": cov_type, "action": "", "items": items }).to_string()
}

/// The name's owner coin, `cov_type` (REGISTER unless a test says
/// otherwise), at our address 0/0, and the tracked row pointing at it.
fn seed_owner_coin(conn: &Connection, net: Network, txid: &str, cov_type: u8) {
    let (addr, spk) = addr00(net);
    let nh = hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap());
    let items = [nh.clone(), hex::encode(NAME_HEIGHT.to_le_bytes())];
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex, value_doos,
             covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, 'name_control', NULL)",
        params![
            txid,
            PROFILE,
            addr,
            spk,
            NAME_VALUE as i64,
            i64::from(cov_type),
            covenant_json(cov_type, &items)
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
         VALUES (?1, ?2, ?3, 'CLOSED', ?4, 0, ?5, ?6)",
        params![PROFILE, NAME, nh, txid, addr, i64::from(NAME_HEIGHT)],
    )
    .unwrap();
}

fn name_info(renewal: u64, claimed: u64, owner_txid: &str) -> Value {
    json!({
        "info": {
            "name": NAME, "state": "CLOSED", "height": NAME_HEIGHT, "renewal": renewal,
            "renewals": 0, "claimed": claimed, "weak": false, "transfer": 0, "revoked": 0,
            "owner": { "hash": owner_txid, "index": 0 }, "value": NAME_VALUE
        },
        "start": null
    })
}

fn unlock(app: &App, net: Network) {
    *app.state::<AppState>().signer.lock().unwrap() = Some(SignerSession::unlock(
        PROFILE.into(),
        net,
        master(),
        600_000,
    ));
}

/// A profile of `kind` on `network` owning NAME as a coin of `cov_type`,
/// unlocked, and a node at `tip` answering `info` for the name.
async fn fixture_with(
    network: &str,
    kind: &str,
    tip: i64,
    cov_type: u8,
    info: Value,
) -> (ServerGuard, Vec<Mock>, App) {
    let mut node = mockito::Server::new_async().await;
    let mocks = vec![
        mock_blockchain_info(&mut node, tip, Some(1_700_000_000)).await,
        mock_name_info(&mut node, info).await,
    ];
    let conn = seeded(network, kind, &node.url());
    seed_owner_coin(&conn, net_of(network), OWNER_TXID, cov_type);
    let app = app_with(conn);
    unlock(&app, net_of(network));
    (node, mocks, app)
}

/// A profile of `kind` on `network` owning NAME, unlocked, and a node at `tip`.
async fn lock_fixture(network: &str, kind: &str, tip: i64) -> (ServerGuard, Vec<Mock>, App) {
    fixture_with(
        network,
        kind,
        tip,
        COV_REGISTER,
        name_info(RENEWAL, 0, OWNER_TXID),
    )
    .await
}

async fn build(
    app: &App,
) -> Result<crate::noncustodial::types::TxDraftSummary, crate::error::AppError> {
    shakedex_build_lock_draft(app.state(), NAME.into(), ListingMode::BuyNow, false, None).await
}

fn count(app: &App, table: &str) -> i64 {
    with_db(app, |c| {
        c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
}

fn open_listing(app: &App) -> Option<ShakedexListing> {
    with_db(app, |c| {
        queries::open_shakedex_listing_for_name(c, PROFILE, NAME).unwrap()
    })
}

/// Nothing written: no draft, no listing, no reserved address.
fn assert_nothing_written(app: &App, addresses_before: i64) {
    assert_eq!(count(app, "wallet_tx_drafts"), 0);
    assert_eq!(count(app, "shakedex_listings"), 0);
    assert_eq!(count(app, "derived_addresses"), addresses_before);
}

/// R19 day 0 and R18: the draft spends our owner coin into a TRANSFER that
/// stays at our address and commits to SHA3-256 of the lock script of the
/// key derived from the seed for this name and account; the listing is
/// Locking with that public key and this draft.
#[tokio::test]
async fn lock_draft_commits_to_the_derived_lock_address() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.expect("lock builds");
    assert_eq!(draft.action, LOCK_ACTION);

    let key = derive_lock_key(&master(), Network::Regtest, 0, NAME).unwrap();
    let row = with_db(&app, |c| {
        queries::get_tx_draft(c, &draft.id).unwrap().unwrap()
    });
    let plan: DraftPlan = serde_json::from_str(&row.signing_inputs_json).unwrap();
    assert_eq!(
        (plan.inputs[0].txid.as_str(), plan.inputs[0].vout),
        (OWNER_TXID, 0)
    );
    let out = &plan.outputs[0];
    assert_eq!(out.covenant_type, COV_TRANSFER);
    assert_eq!(
        out.address,
        addr00(Network::Regtest).0,
        "stays home until finalized"
    );
    assert_eq!(out.value, NAME_VALUE);
    let nh = hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap());
    assert_eq!(
        out.covenant_items_hex,
        vec![
            nh,
            hex::encode(NAME_HEIGHT.to_le_bytes()),
            "00".into(),
            hex::encode(key.program)
        ]
    );
    assert_eq!(draft.summary["recipientAddress"], key.address);
    assert_eq!(draft.summary["name"], NAME);

    let l = open_listing(&app).expect("a listing");
    assert_eq!(l.state, ListingState::Locking);
    assert_eq!(l.mode, ListingMode::BuyNow);
    assert_eq!(l.lock_pubkey_hex, hex::encode(key.pubkey));
    assert_eq!(l.lock_transfer_draft_id.as_deref(), Some(draft.id.as_str()));
    assert_eq!(
        l.lock_transfer_txid,
        row.summary_json
            .parse::<Value>()
            .ok()
            .and_then(|s| s["txid"].as_str().map(str::to_owned))
    );
    assert!(l.lock_transfer_txid.is_some());
    assert!(!l.publish);
    assert_eq!(
        (l.lock_txid, l.lock_vout),
        (None, None),
        "set by Finalize & sign (T3)"
    );
}

/// R18: a key that fails its self-check never reaches a draft, a listing or
/// a reserved address.
#[tokio::test]
async fn refuses_lock_on_self_check_failure() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let ctx = crate::commands::draft_ctx::load_ctx(&app.state()).unwrap();
    let mut key = derive_lock_key(&master(), Network::Regtest, 0, NAME).unwrap();
    let other = derive_lock_key(&master(), Network::Regtest, 0, "othername").unwrap();
    key.address = other.address.clone();
    let addresses_before = count(&app, "derived_addresses");
    let err = with_db(&app, |c| {
        let owner = queries::get_name_coin(c, PROFILE, NAME).unwrap().unwrap();
        build_lock_draft_inner(
            c,
            &LockDraftInput {
                ctx: &ctx,
                key: &key,
                name: NAME,
                mode: ListingMode::BuyNow,
                publish: false,
                owner: &owner,
                name_height: NAME_HEIGHT,
                notice: ExpiryNotice::Ok,
                rate: 10,
            },
        )
        .unwrap_err()
    });
    assert!(err_text(err).contains("self-check"));
    assert_nothing_written(&app, addresses_before);
}

/// R6/R29: no lock on a node that cannot send.
#[tokio::test]
async fn lock_refused_without_write_capability() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    with_db(&app, |c| set(c, "chain_source", "explorer"));
    let err = build(&app).await.unwrap_err();
    assert!(err_text(err).contains(crate::commands::shakedex::NEEDS_SENDING_NODE));
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R16/R29: Ledger, watch-only and extended-private-key profiles cannot
/// lock, with the sentence the UI shows.
#[tokio::test]
async fn lock_refused_for_ledger_and_watch_only() {
    let sentence = crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY;
    for kind in ["ledger_hardware", "xpriv_hot", "watch_only_xpub"] {
        let (_n, _m, app) = lock_fixture("regtest", kind, QUIET_TIP).await;
        if kind == "watch_only_xpub" {
            with_db(&app, |c| {
                c.execute(
                    "UPDATE wallet_profiles SET watch_only = 1 WHERE id = ?1",
                    params![PROFILE],
                )
                .unwrap();
            });
        }
        let err = err_text(build(&app).await.unwrap_err());
        assert!(err.contains(sentence), "{kind}: {err}");
        assert_eq!(count(&app, "shakedex_listings"), 0, "{kind}");
    }
}

/// R15/R29: a new listing on mainnet needs the experimental flag.
#[tokio::test]
async fn mainnet_lock_needs_experimental_flag() {
    // Mainnet: renewal 1000 + 105 120; tip 2000 is far from expiry.
    let (_node, _m, app) = lock_fixture("mainnet", "mnemonic_hot", 2_000).await;
    let err = build(&app).await.unwrap_err();
    assert!(err_text(err).contains(crate::noncustodial::shakedex::MAINNET_SELLING_EXPERIMENTAL));
    assert_eq!(count(&app, "shakedex_listings"), 0);
    with_db(&app, |c| set(c, "shakedex_experimental", "true"));
    let draft = build(&app).await.expect("flag set: the lock builds");
    assert_eq!(draft.action, LOCK_ACTION);
}

#[tokio::test]
async fn regtest_lock_ignores_flag() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    with_db(&app, |c| set(c, "shakedex_experimental", "false"));
    let draft = build(&app).await.expect("regtest needs no flag");
    assert_eq!(draft.action, LOCK_ACTION);
}

/// R23/R29: off mainnet a listing is shared as a file only.
#[tokio::test]
async fn regtest_lock_cannot_publish_to_the_market() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let err = shakedex_build_lock_draft(app.state(), NAME.into(), ListingMode::BuyNow, true, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains(crate::commands::shakedex::MARKET_MAINNET_ONLY));
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R18: Lock derives the lock key, so it needs the unlocked signer: not a
/// locked session, not another profile's, not none.
#[tokio::test]
async fn lock_needs_the_unlocked_signer() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    app.state::<AppState>()
        .signer
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lock();
    assert!(matches!(
        build(&app).await.unwrap_err(),
        crate::error::AppError::WalletLocked
    ));
    *app.state::<AppState>().signer.lock().unwrap() = Some(SignerSession::unlock(
        "another".into(),
        Network::Regtest,
        master(),
        600_000,
    ));
    assert!(err_text(build(&app).await.unwrap_err()).contains("different wallet profile"));
    *app.state::<AppState>().signer.lock().unwrap() = None;
    assert!(matches!(
        build(&app).await.unwrap_err(),
        crate::error::AppError::WalletLocked
    ));
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R31 at Lock, regtest: expiry 6000; refused at tip 5979 (6000 <= 5979 +
/// 1 + 10 + 10), built at tip 5978, with nothing written by the refusal.
#[tokio::test]
async fn lock_refused_when_the_name_would_expire_during_the_lockup() {
    let (mut node, mut mocks, app) =
        lock_fixture("regtest", "mnemonic_hot", REGTEST_END - 21).await;
    let addresses_before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(
        err.contains("before its transfer into the lock could be finalized"),
        "R31 refusal: {err}"
    );
    assert!(err.contains(&REGTEST_END.to_string()), "{err}");
    assert_nothing_written(&app, addresses_before);

    mocks.remove(0).remove_async().await;
    mocks.push(mock_blockchain_info(&mut node, REGTEST_END - 22, Some(1_700_000_000)).await);
    build(&app)
        .await
        .expect("one block earlier the lock builds");
}

/// R31's warning: below 180 R9 days (1800 regtest blocks) from the tip the
/// draft carries it; at 1800 it does not. LOCK_COSTS is always there.
#[tokio::test]
async fn lock_warns_below_six_months() {
    let (mut node, mut mocks, app) =
        lock_fixture("regtest", "mnemonic_hot", REGTEST_END - 1_799).await;
    let draft = build(&app).await.expect("builds with the warning");
    let warnings: Vec<String> = serde_json::from_value(draft.summary["warnings"].clone()).unwrap();
    assert_eq!(
        warnings,
        vec![LOCK_COSTS.to_string(), sell::near_expiry_warning(1_799)]
    );
    // The unsent draft goes, and its listing with it; the owner coin is free.
    with_db(&app, |c| queries::delete_tx_draft(c, &draft.id).unwrap());
    assert_eq!(count(&app, "shakedex_listings"), 0);

    mocks.remove(0).remove_async().await;
    mocks.push(mock_blockchain_info(&mut node, REGTEST_END - 1_800, Some(1_700_000_000)).await);
    let draft = build(&app).await.expect("builds without it");
    let warnings: Vec<String> = serde_json::from_value(draft.summary["warnings"].clone()).unwrap();
    assert_eq!(warnings, vec![LOCK_COSTS.to_string()]);
}

/// R27 and R31: the sentences say what the spec says; the warning carries
/// the block count it is given.
#[test]
fn lock_sentences_say_what_locking_costs() {
    for phrase in ["cannot be changed", "cannot be renewed", "renews the name"] {
        assert!(LOCK_COSTS.contains(phrase), "{phrase}");
    }
    let w = sell::near_expiry_warning(1_234);
    assert!(w.contains("expires in 1234 blocks"), "{w}");
    assert!(w.contains("renews the name"), "{w}");
}

/// The lock draft's summary reads back as the plain `TxSummary` the secure
/// window and the frontend's `TxDraftSummary.summary` expect, plus `name`.
#[tokio::test]
async fn lock_summary_reads_as_a_tx_summary() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.unwrap();
    let row = with_db(&app, |c| {
        queries::get_tx_draft(c, &draft.id).unwrap().unwrap()
    });
    let s: TxSummary = serde_json::from_str(&row.summary_json).unwrap();
    assert_eq!(s.action, LOCK_ACTION);
    assert_eq!(s.warnings, vec![LOCK_COSTS.to_string()]);
    assert_eq!(
        s.send_total_doos, NAME_VALUE as i64,
        "the name's own output"
    );
    assert!(s.fee_doos > 0);
    assert!(s.txid.is_some());
    assert_eq!(open_listing(&app).unwrap().lock_transfer_txid, s.txid);
    assert_eq!(draft.summary["name"], NAME);
}

/// R21: the listing pays a fresh receive address and cancels to another,
/// both marked used when the listing is created, so the next allocation
/// returns a third; the cancel address's receive-branch index is stored.
#[tokio::test]
async fn lock_reserves_payment_and_cancel_addresses() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    build(&app).await.expect("lock builds");
    let l = open_listing(&app).unwrap();
    let pay = l.payment_address.clone().expect("payment address");
    let cancel = l.cancel_address.clone().expect("cancel address");
    assert_ne!(pay, cancel);
    with_db(&app, |c| {
        let rows = queries::list_receive_addresses(c, PROFILE, 0).unwrap();
        for a in [&pay, &cancel] {
            assert!(
                rows.iter().any(|r| &r.address == a && r.used),
                "{a} reserved and used"
            );
        }
        let cancel_row = rows.iter().find(|r| r.address == cancel).unwrap();
        assert_eq!(l.cancel_child_index, Some(i64::from(cancel_row.index)));
        assert_ne!(
            cancel_row.index, 0,
            "not the address the owner coin sits at"
        );
        let next = derivation::reserve_receive_address(c, PROFILE).unwrap();
        assert!(next.address != pay && next.address != cancel);
    });
}

/// hsd lets a TRANSFER coin go only to UPDATE, RENEW, FINALIZE or REVOKE:
/// a name already in a transfer is not locked.
#[tokio::test]
async fn lock_refused_while_a_transfer_is_pending() {
    let (_node, _m, app) = fixture_with(
        "regtest",
        "mnemonic_hot",
        QUIET_TIP,
        COV_TRANSFER,
        name_info(RENEWAL, 0, OWNER_TXID),
    )
    .await;
    let before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(
        err.contains("a transfer of 'dexsale' is pending: cancel or finalize it"),
        "{err}"
    );
    assert_nothing_written(&app, before);
}

/// Only an owner coin hsd lets go to a TRANSFER (REGISTER, UPDATE, RENEW,
/// FINALIZE) is locked; a name the wallet does not hold is not either.
#[tokio::test]
async fn lock_refused_for_a_name_not_ours_or_not_registered() {
    let (_node, _m, app) = fixture_with(
        "regtest",
        "mnemonic_hot",
        QUIET_TIP,
        COV_REVEAL,
        name_info(RENEWAL, 0, OWNER_TXID),
    )
    .await;
    let before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("is not registered to this wallet"), "{err}");
    assert_nothing_written(&app, before);

    with_db(&app, |c| {
        c.execute("DELETE FROM tracked_name_states", []).unwrap();
    });
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("wallet does not hold"), "{err}");
    assert_nothing_written(&app, before);
}

#[tokio::test]
async fn second_lock_of_a_name_is_refused() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    build(&app).await.expect("first lock builds");
    let before = count(&app, "derived_addresses");
    assert!(err_text(build(&app).await.unwrap_err()).contains("already locked for sale"));
    assert_eq!(count(&app, "shakedex_listings"), 1);
    assert_eq!(count(&app, "wallet_tx_drafts"), 1);
    assert_eq!(count(&app, "derived_addresses"), before);
}

/// Fail closed: R31 reads the tip, the renewal height and the claimed count
/// from the node; a reply without one of them refuses the lock ("could not
/// check"), and a name with no state is refused as such.
#[tokio::test]
async fn lock_refused_when_the_node_cannot_say_when_the_name_expires() {
    let mut no_renewal = name_info(RENEWAL, 0, OWNER_TXID);
    no_renewal["info"]
        .as_object_mut()
        .unwrap()
        .remove("renewal");
    let mut no_claimed = name_info(RENEWAL, 0, OWNER_TXID);
    no_claimed["info"]
        .as_object_mut()
        .unwrap()
        .remove("claimed");
    for (what, info) in [("renewal", no_renewal), ("claimed", no_claimed)] {
        let (_n, _m, app) =
            fixture_with("regtest", "mnemonic_hot", QUIET_TIP, COV_REGISTER, info).await;
        let before = count(&app, "derived_addresses");
        let err = err_text(build(&app).await.unwrap_err());
        assert!(err.contains("could not check"), "{what}: {err}");
        assert_nothing_written(&app, before);
    }

    let (_n, _m, app) = fixture_with(
        "regtest",
        "mnemonic_hot",
        QUIET_TIP,
        COV_REGISTER,
        json!({ "info": null, "start": null }),
    )
    .await;
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("no on-chain state or has expired"), "{err}");
    assert_eq!(count(&app, "shakedex_listings"), 0);

    // A getblockchaininfo reply without the tip.
    let mut node = mockito::Server::new_async().await;
    let _b = node
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            json!({ "method": "getblockchaininfo" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!({ "chain": "regtest", "headers": QUIET_TIP })))
        .create_async()
        .await;
    let _n = mock_name_info(&mut node, name_info(RENEWAL, 0, OWNER_TXID)).await;
    let conn = seeded("regtest", "mnemonic_hot", &node.url());
    seed_owner_coin(&conn, Network::Regtest, OWNER_TXID, COV_REGISTER);
    let app = app_with(conn);
    unlock(&app, Network::Regtest);
    let before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("could not check"), "no tip: {err}");
    assert_nothing_written(&app, before);
}

/// R27 in the secure window: the lock draft's confirmation carries what
/// locking costs as a Warning row.
#[tokio::test]
async fn lock_confirmation_shows_what_locking_costs() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.unwrap();
    let row = with_db(&app, |c| {
        queries::get_tx_draft(c, &draft.id).unwrap().unwrap()
    });
    let details = crate::commands::tx::confirm_details_for_draft(&row).unwrap();
    assert!(
        details["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["label"] == "Warning" && r["value"] == LOCK_COSTS),
        "{details}"
    );
}

// --- the abort ------------------------------------------------------------

use crate::commands::names::build_cancel_draft;
use crate::noncustodial::rpc::NodeCoin;
use crate::shakedex_jobs::refresh_listings_before_lock_with_client;
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};

/// Lock NAME, then make the wallet's records what a sync leaves after the
/// lock TRANSFER is mined: our TRANSFER coin at 0/0 is the owner coin.
/// Returns the lock txid.
async fn locked_on_chain(app: &App) -> String {
    locked_on_chain_on(app, Network::Regtest).await
}

/// [`locked_on_chain`] for a profile on `net`.
async fn locked_on_chain_on(app: &App, net: Network) -> String {
    let draft = build(app).await.expect("lock builds");
    let txid = draft.summary["txid"].as_str().unwrap().to_string();
    with_db(app, |c| {
        queries::update_tx_draft_status(c, &draft.id, "broadcasted", None, Some(&txid)).unwrap();
        queries::update_tx_draft_confirmation(c, &draft.id, QUIET_TIP - 100, None).unwrap();
        // The funding coin the lock reserved stands in for its change, so
        // the cancel has a coin to pay its fee from.
        queries::release_reserved_utxos_for_draft(c, &draft.id).unwrap();
        c.execute(
            "UPDATE tracked_utxos SET spent_by_txid = ?1 WHERE txid = ?2",
            params![txid, OWNER_TXID],
        )
        .unwrap();
        c.execute(
            "DELETE FROM tracked_name_states WHERE name = ?1",
            params![NAME],
        )
        .unwrap();
        seed_owner_coin(c, net, &txid, COV_TRANSFER);
    });
    txid
}

/// A coin as hsd's `GET /coin/:hash/:index` sends it: `height` is the block
/// it was mined in, -1 in the mempool, absent when a test leaves it out.
fn node_coin(txid: &str, vout: u32, height: Option<i64>) -> NodeCoin {
    let mut coin = json!({ "hash": txid, "index": vout, "value": NAME_VALUE });
    if let Some(h) = height {
        coin["height"] = h.into();
    }
    serde_json::from_value(coin).unwrap()
}

/// A node whose `GET /coin` knows the cancel's UPDATE (output 0) at
/// `cancel_height` and the lock TRANSFER coin when `transfer_unspent`;
/// `None` for a coin hsd answers 404 for (spent, or never existed).
fn chain(
    cancel_txid: &str,
    cancel_height: Option<Option<i64>>,
    lock_txid: &str,
    transfer_unspent: bool,
) -> MockNodeRpc {
    let (cancel_txid, lock_txid) = (cancel_txid.to_string(), lock_txid.to_string());
    MockNodeRpc::new().with_get_coin(move |txid, vout| {
        Ok(if txid == cancel_txid && vout == 0 {
            cancel_height.map(|h| node_coin(txid, vout, h))
        } else if txid == lock_txid && vout == 0 && transfer_unspent {
            Some(node_coin(txid, vout, Some(QUIET_TIP - 100)))
        } else {
            None
        })
    })
}

/// Run the abort job on the app's database, as the sync step does. The
/// connection is taken out of the app for the call, so no lock is held
/// across an await.
async fn run_abort_job(app: &App, rpc: &MockNodeRpc) {
    let conn = std::mem::replace(
        &mut *app.state::<AppState>().db.lock().unwrap(),
        Connection::open_in_memory().unwrap(),
    );
    let res = refresh_listings_before_lock_with_client(&conn, rpc, PROFILE).await;
    *app.state::<AppState>().db.lock().unwrap() = conn;
    res.expect("abort job runs");
    assert_eq!(
        rpc.count_matching(|c| matches!(c, RpcCall::SendRawTransaction(_))),
        0,
        "the job sends nothing (it runs in the daemon too)"
    );
}

fn listing_state(app: &App, id: &str) -> ListingState {
    with_db(app, |c| {
        queries::get_shakedex_listing(c, id).unwrap().unwrap().state
    })
}

/// R19: Cancel transfer on a name still Locking is the abort. Building or
/// sending it changes nothing (hsd answers a refused send with its txid);
/// once its UPDATE is a mined coin the listing is Aborted, and a reorg that
/// takes it out (the UPDATE back in the mempool, or the lock TRANSFER
/// unspent again) makes the listing Locking again.
#[tokio::test]
async fn cancel_transfer_aborts_the_listing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    assert_eq!(
        open_listing(&app).unwrap().lock_transfer_txid.as_deref(),
        Some(lock_txid.as_str())
    );

    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .expect("cancel builds");
    let l = open_listing(&app).unwrap();
    assert_eq!(l.abort_draft_id.as_deref(), Some(cancel.id.as_str()));
    assert_eq!(l.state, ListingState::Locking, "built, not sent");
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    assert_eq!(
        l.abort_txid.as_deref(),
        Some(ctxid.as_str()),
        "the cancel's txid"
    );
    with_db(&app, |c| {
        queries::update_tx_draft_status(c, &cancel.id, "broadcasted", None, Some(&ctxid)).unwrap();
    });

    // Sent, nothing on chain yet: the lock TRANSFER is still unspent.
    run_abort_job(&app, &chain(&ctxid, None, &lock_txid, true)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "not taken"
    );
    run_abort_job(&app, &chain(&ctxid, Some(Some(-1)), &lock_txid, false)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "in the mempool"
    );

    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(listing_state(&app, &l.id), ListingState::Aborted);
    assert!(open_listing(&app).is_none(), "the name is free again");

    run_abort_job(&app, &chain(&ctxid, Some(Some(-1)), &lock_txid, false)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "a reorg took the cancel back to the mempool"
    );

    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Aborted,
        "mined again"
    );
    run_abort_job(&app, &chain(&ctxid, None, &lock_txid, true)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "a reorg took the cancel out and the lock TRANSFER is unspent again"
    );
}

/// A final verdict needs the node's word: a coin without its height, the
/// cancel's UPDATE gone with the lock TRANSFER spent (mined and spent since,
/// or another spend), or no answer at all leave the listing as it is.
#[tokio::test]
async fn abort_needs_the_nodes_word() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    let id = open_listing(&app).unwrap().id;

    for state in [ListingState::Locking, ListingState::Aborted] {
        with_db(&app, |c| {
            c.execute(
                "UPDATE shakedex_listings SET state = ?1 WHERE id = ?2",
                params![state, id],
            )
            .unwrap();
        });
        for (what, rpc) in [
            ("no height", chain(&ctxid, Some(None), &lock_txid, true)),
            ("both spent", chain(&ctxid, None, &lock_txid, false)),
            ("no answer", MockNodeRpc::new()),
        ] {
            run_abort_job(&app, &rpc).await;
            assert_eq!(listing_state(&app, &id), state, "{what}");
        }
    }
}

/// Only a cancel of this listing's own lock TRANSFER aborts it.
#[tokio::test]
async fn cancel_of_another_transfer_leaves_the_listing_alone() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock = build(&app).await.expect("lock builds"); // never sent
                                                        // A TRANSFER of the name that is not the lock's (another device).
    with_db(&app, |c| {
        queries::release_reserved_utxos_for_draft(c, &lock.id).unwrap();
        c.execute(
            "UPDATE tracked_utxos SET spent_by_txid = 'x' WHERE txid = ?1",
            params![OWNER_TXID],
        )
        .unwrap();
        c.execute(
            "DELETE FROM tracked_name_states WHERE name = ?1",
            params![NAME],
        )
        .unwrap();
        seed_owner_coin(c, Network::Regtest, &"77".repeat(32), COV_TRANSFER);
    });
    build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .expect("cancel builds");
    assert_eq!(open_listing(&app).unwrap().abort_draft_id, None);
}

/// Deleting a Cancel transfer that was never sent (`draft`, `signed`) clears
/// its link: it aborts nothing. A `dropped` or `failed` one was broadcast and
/// may still be mined, so its listing keeps the link.
#[tokio::test]
async fn deleting_an_unsent_cancel_unlinks_it() {
    for (status, keeps_link) in [
        ("draft", false),
        ("signed", false),
        ("dropped", true),
        ("failed", true),
    ] {
        let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
        locked_on_chain(&app).await;
        let cancel = build_cancel_draft(app.state(), NAME.into(), None)
            .await
            .unwrap();
        assert_eq!(
            open_listing(&app).unwrap().abort_draft_id.as_deref(),
            Some(cancel.id.as_str())
        );
        with_db(&app, |c| {
            if status != "draft" {
                queries::update_tx_draft_status(c, &cancel.id, status, None, None).unwrap();
            }
            queries::delete_tx_draft(c, &cancel.id).unwrap();
        });
        let after = open_listing(&app).unwrap();
        let expected = keeps_link.then(|| cancel.id.clone());
        assert_eq!(after.abort_draft_id, expected, "{status}");
        let expected_txid =
            keeps_link.then(|| cancel.summary["txid"].as_str().unwrap().to_string());
        assert_eq!(after.abort_txid, expected_txid, "{status}");
    }
}

/// A cancel that was broadcast, then dropped and its draft deleted, may
/// still be mined (another node held it): the listing keeps the cancel's
/// txid, so the job still finds the mined UPDATE and the listing is Aborted.
#[tokio::test]
async fn deleted_dropped_cancel_mined_later_still_aborts() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    let id = open_listing(&app).unwrap().id;
    with_db(&app, |c| {
        queries::update_tx_draft_status(c, &cancel.id, "dropped", None, Some(&ctxid)).unwrap();
        queries::delete_tx_draft(c, &cancel.id).unwrap();
        assert!(queries::get_tx_draft(c, &cancel.id).unwrap().is_none());
    });

    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted);
}

/// A reorg that takes the abort out after the name was locked again: the new
/// listing is the one open (`idx_shakedex_listings_open_name` allows one per
/// name), so the old listing stays Aborted and the job carries on. The new
/// lock TRANSFER spends the cancel's UPDATE, so it is mined only if the
/// cancel is.
#[tokio::test]
async fn reorged_abort_leaves_a_newer_listing_open() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    let old = open_listing(&app).unwrap().id;
    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(listing_state(&app, &old), ListingState::Aborted);

    let mut newer = listing("newer", NAME, ListingState::Locking);
    newer.wallet_profile_id = PROFILE.into();
    with_db(&app, |c| {
        queries::insert_shakedex_listing(c, &newer).unwrap()
    });

    run_abort_job(&app, &chain(&ctxid, Some(Some(-1)), &lock_txid, false)).await;
    assert_eq!(listing_state(&app, &old), ListingState::Aborted);
    assert_eq!(open_listing(&app).unwrap().id, "newer");
    // Refused by the guard, not by the index: no error for the job to log.
    with_db(&app, |c| {
        assert_eq!(queries::unabort_shakedex_listing(c, &old).unwrap(), 0);
    });
}

/// The abort is a sync step: both the app's sync and the daemon's run it
/// against an authoritative node, and neither sends anything (SECURITY.md,
/// "Daemon is read-only").
#[tokio::test]
async fn sync_aborts_the_listing_in_the_app_and_the_daemon() {
    use crate::commands::sync::{run_sync_steps, SyncCaller, SyncStatus};

    for caller in [SyncCaller::Daemon, SyncCaller::App] {
        let (_lock_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
        locked_on_chain(&app).await;
        let cancel = build_cancel_draft(app.state(), NAME.into(), None)
            .await
            .unwrap();
        let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
        let id = open_listing(&app).unwrap().id;

        let mut node = mockito::Server::new_async().await;
        let path = std::env::temp_dir().join(format!(
            "namehold_sell_abort_{}_{caller:?}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let db_path = path.to_str().unwrap().to_string();
        with_db(&app, |c| {
            queries::set_setting(c, "node_rpc_url", &node.url()).unwrap();
            c.execute("VACUUM INTO ?1", params![db_path]).unwrap();
        });
        let send = node
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("sendrawtransaction".into()))
            .expect(0)
            .create_async()
            .await;
        let _info = node
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(
                json!({ "method": "getblockchaininfo" }),
            ))
            .with_header("content-type", "application/json")
            .with_body(rpc_ok(json!({
                "chain": "regtest", "blocks": QUIET_TIP, "headers": QUIET_TIP,
                "verificationprogress": 1.0, "mediantime": 1_700_000_000u64
            })))
            .create_async()
            .await;
        let _mined = node
            .mock("GET", format!("/coin/{ctxid}/0").as_str())
            .with_header("content-type", "application/json")
            .with_body(
                json!({ "hash": ctxid, "index": 0, "value": NAME_VALUE, "height": QUIET_TIP })
                    .to_string(),
            )
            .create_async()
            .await;

        match caller {
            SyncCaller::Daemon => crate::daemon::sync_profile(&db_path, PROFILE).await,
            SyncCaller::App => {
                let status = std::sync::Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
                run_sync_steps(&status, &db_path, PROFILE, SyncCaller::App).await;
            }
        }

        send.assert_async().await;
        let conn = Connection::open(&path).unwrap();
        let got = queries::get_shakedex_listing(&conn, &id).unwrap().unwrap();
        assert_eq!(got.state, ListingState::Aborted, "{caller:?}");
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}

// --- the lock resolved from chain facts -------------------------------------

/// hsd's `getnameinfo` for NAME with its owner outpoint and `revoked` height
/// (`NameState.getJSON`; a REVOKE sets `revoked` and leaves `owner` as it
/// was, `chain.js`).
fn info_with(owner_txid: &str, owner_index: u32, revoked: u64) -> Value {
    let mut v = name_info(RENEWAL, 0, owner_txid);
    v["info"]["owner"]["index"] = owner_index.into();
    v["info"]["revoked"] = revoked.into();
    v
}

/// A node answering `info` for NAME, and `GET /coin` with a mined coin for
/// each outpoint in `coins` and hsd's empty 404 for any other.
fn facts(info: Value, coins: &[(&str, u32)]) -> MockNodeRpc {
    let coins: Vec<(String, u32)> = coins.iter().map(|(t, v)| (t.to_string(), *v)).collect();
    MockNodeRpc::new()
        .with_name_info(info)
        .with_get_coin(move |txid, vout| {
            Ok(coins
                .iter()
                .any(|(t, v)| t == txid && *v == vout)
                .then(|| node_coin(txid, vout, Some(QUIET_TIP - 100))))
        })
}

fn set_state(app: &App, id: &str, state: ListingState) {
    with_db(app, |c| {
        c.execute(
            "UPDATE shakedex_listings SET state = ?1 WHERE id = ?2",
            params![state, id],
        )
        .unwrap();
    });
}

/// R19 from chain facts: a Cancel transfer this wallet never linked (sent
/// from another device) is mined: the owner is its UPDATE, the lock TRANSFER
/// coin is gone and the lock draft is confirmed, so the listing is Aborted.
/// While the lock TRANSFER is still the owner and a coin it stays Locking.
#[tokio::test]
async fn external_cancel_aborts_the_listing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let id = open_listing(&app).unwrap().id;
    assert_eq!(open_listing(&app).unwrap().abort_txid, None);

    run_abort_job(
        &app,
        &facts(info_with(&lock_txid, 0, 0), &[(&lock_txid, 0)]),
    )
    .await;
    assert_eq!(listing_state(&app, &id), ListingState::Locking);

    let other_cancel = "ee".repeat(32);
    run_abort_job(
        &app,
        &facts(info_with(&other_cancel, 0, 0), &[(&other_cancel, 0)]),
    )
    .await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted);
}

/// A REVOKE of the name leaves hsd's `owner` at the coin it spent (the lock
/// TRANSFER) and sets `revoked`: with the lock coin spent, the listing is
/// Aborted, and a revoked owner never makes it Locking again.
#[tokio::test]
async fn revoked_name_aborts_the_listing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let id = open_listing(&app).unwrap().id;
    let revoked = facts(info_with(&lock_txid, 0, QUIET_TIP as u64), &[]);
    run_abort_job(&app, &revoked).await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted);
    run_abort_job(&app, &revoked).await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted, "stays");
}

/// A lock TRANSFER that never landed: its draft dropped, failed or deleted,
/// its coin hsd's 404 and the owner still the coin it would have spent. The
/// listing is Aborted, and the name can be locked again at once.
#[tokio::test]
async fn dead_lock_draft_aborts_and_the_name_can_be_locked_again() {
    for end in ["dropped", "failed", "deleted"] {
        let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
        let lock = build(&app).await.expect("lock builds");
        let l = open_listing(&app).unwrap();
        with_db(&app, |c| {
            let status = if end == "deleted" { "dropped" } else { end };
            queries::update_tx_draft_status(c, &lock.id, status, None, None).unwrap();
            queries::release_reserved_utxos_for_draft(c, &lock.id).unwrap();
            if end == "deleted" {
                queries::delete_tx_draft(c, &lock.id).unwrap();
            }
        });
        run_abort_job(
            &app,
            &facts(info_with(OWNER_TXID, 0, 0), &[(OWNER_TXID, 0)]),
        )
        .await;
        assert_eq!(listing_state(&app, &l.id), ListingState::Aborted, "{end}");
        build(&app).await.expect("a new lock of the name");
        assert_ne!(open_listing(&app).unwrap().id, l.id, "{end}");
    }
}

/// A Cancel transfer replaced before it was sent may still be the one that is
/// mined (another device sent it): the listing links the newer cancel, whose
/// UPDATE never appears, and is Aborted from the chain facts all the same.
#[tokio::test]
async fn replaced_cancel_aborts_the_listing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    locked_on_chain(&app).await;
    let a = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let a_txid = a.summary["txid"].as_str().unwrap().to_string();
    with_db(&app, |c| queries::delete_tx_draft(c, &a.id).unwrap());
    // Another fee rate, so another transaction.
    let b = build_cancel_draft(app.state(), NAME.into(), Some(7_777))
        .await
        .unwrap();
    let l = open_listing(&app).unwrap();
    assert_eq!(l.abort_draft_id.as_deref(), Some(b.id.as_str()));
    assert_ne!(l.abort_txid.as_deref(), Some(a_txid.as_str()));

    run_abort_job(&app, &facts(info_with(&a_txid, 0, 0), &[(&a_txid, 0)])).await;
    assert_eq!(listing_state(&app, &l.id), ListingState::Aborted);
}

/// A lock draft that may still land (unsent, sent, or sent with no word
/// back) proves nothing by a missing coin: the listing stays Locking.
#[tokio::test]
async fn lock_draft_in_flight_without_its_coin_changes_nothing() {
    for status in ["draft", "signed", "broadcast_pending", "broadcasted"] {
        let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
        let lock = build(&app).await.expect("lock builds");
        let id = open_listing(&app).unwrap().id;
        if status != "draft" {
            with_db(&app, |c| {
                queries::update_tx_draft_status(c, &lock.id, status, None, None).unwrap();
            });
        }
        run_abort_job(
            &app,
            &facts(info_with(OWNER_TXID, 0, 0), &[(OWNER_TXID, 0)]),
        )
        .await;
        assert_eq!(listing_state(&app, &id), ListingState::Locking, "{status}");
    }
}

/// hsd says the name has no live state (`info: null`, expired): the listing
/// is Expired. An Aborted listing stays Aborted.
#[tokio::test]
async fn name_without_live_state_expires_the_listing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let id = open_listing(&app).unwrap().id;
    let gone = facts(json!({ "info": null, "start": null }), &[(&lock_txid, 0)]);
    set_state(&app, &id, ListingState::Aborted);
    run_abort_job(&app, &gone).await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted);
    with_db(&app, |c| {
        assert_eq!(
            queries::expire_shakedex_listing(c, &id).unwrap(),
            0,
            "only before the lock"
        );
    });
    set_state(&app, &id, ListingState::Locking);
    run_abort_job(&app, &gone).await;
    assert_eq!(listing_state(&app, &id), ListingState::Expired);
}

/// An Aborted listing is Locking again once its lock TRANSFER is a coin
/// again, or is the owner of a name that is not revoked.
#[tokio::test]
async fn aborted_listing_relocks_when_the_lock_transfer_is_back() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let id = open_listing(&app).unwrap().id;
    let other = "ee".repeat(32);
    for (what, rpc) in [
        (
            "a coin again",
            facts(info_with(&other, 0, 0), &[(&lock_txid, 0)]),
        ),
        ("the owner again", facts(info_with(&lock_txid, 0, 0), &[])),
    ] {
        set_state(&app, &id, ListingState::Aborted);
        run_abort_job(&app, &rpc).await;
        assert_eq!(listing_state(&app, &id), ListingState::Locking, "{what}");
    }
    // The lock TRANSFER is output 0: another output of its transaction is
    // not it.
    set_state(&app, &id, ListingState::Aborted);
    run_abort_job(&app, &facts(info_with(&lock_txid, 1, 0), &[])).await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted, "output 1");
}

/// Any reply that is not hsd's whole answer leaves the listing as it is: a
/// read error, a `getnameinfo` without `info`, `owner`, its `hash` or
/// `index`, or `revoked`, and a `GET /coin` that errs (as a 404 with a body
/// does, `rpc::hsd_not_found`).
#[tokio::test]
async fn missing_chain_facts_change_nothing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let id = open_listing(&app).unwrap().id;
    let other = "ee".repeat(32);
    // Facts that would abort a Locking listing, and relock an Aborted one.
    let abort = info_with(&other, 0, 0);
    let relock = info_with(&lock_txid, 0, 0);
    let strip = |mut v: Value, path: &[&str]| {
        let (last, parents) = path.split_last().unwrap();
        let mut at = &mut v;
        for p in parents {
            at = &mut at[*p];
        }
        at.as_object_mut().unwrap().remove(*last);
        v
    };
    for (state, base) in [
        (ListingState::Locking, abort),
        (ListingState::Aborted, relock),
    ] {
        let mut cases: Vec<(String, MockNodeRpc)> = [
            vec!["info"],
            vec!["info", "owner"],
            vec!["info", "owner", "hash"],
            vec!["info", "owner", "index"],
            vec!["info", "revoked"],
        ]
        .into_iter()
        .map(|path| (path.join("."), facts(strip(base.clone(), &path), &[])))
        .collect();
        cases.push((
            "no name info".into(),
            MockNodeRpc::new()
                .with_name_info_err("down")
                .with_get_coin(|_, _| Ok(None)),
        ));
        // The owner is another outpoint, so only the coin decides.
        cases.push((
            "coin lookup error".into(),
            MockNodeRpc::new()
                .with_name_info(info_with(&other, 0, 0))
                .with_get_coin(|_, _| {
                    Err(crate::error::AppError::Rpc(
                        "coin lookup got a 404 that is not hsd's (it has a body)".into(),
                    ))
                }),
        ));
        for (what, rpc) in cases {
            set_state(&app, &id, state);
            run_abort_job(&app, &rpc).await;
            assert_eq!(listing_state(&app, &id), state, "{state:?}: {what}");
        }
    }
}

// --- R31 again at broadcast ---------------------------------------------------

/// A regtest node at `tip` that answers NAME's `getnameinfo` and counts
/// `sendrawtransaction` (expected `sends` times).
async fn broadcast_node(tip: i64, sends: usize) -> (ServerGuard, Vec<Mock>) {
    let mut node = mockito::Server::new_async().await;
    let info = node
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            json!({ "method": "getblockchaininfo" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!({
            "chain": "regtest", "blocks": tip, "headers": tip,
            "verificationprogress": 1.0, "mediantime": 1_700_000_000u64
        })))
        .create_async()
        .await;
    let name = mock_name_info(&mut node, name_info(RENEWAL, 0, OWNER_TXID)).await;
    let send = node
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            json!({ "method": "sendrawtransaction" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!("ab".repeat(32))))
        .expect(sends)
        .create_async()
        .await;
    (node, vec![info, name, send])
}

/// Build the lock far from expiry and mark it signed; then point the profile
/// at `node`.
async fn signed_lock_sent_through(node: &ServerGuard) -> (App, String) {
    let (_build_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.expect("lock builds far from expiry");
    with_db(&app, |c| {
        queries::update_tx_draft_signed(c, &draft.id, "00", &draft.summary.to_string()).unwrap();
        set(c, "node_rpc_url", &node.url());
    });
    (app, draft.id)
}

/// R31 is judged again when the lock is sent: it may be broadcast days after
/// it was built. Refused with the build's own sentence, nothing reaches the
/// node, and the unsent draft goes with its listing, freeing the name. One
/// block earlier the lock is sent.
#[tokio::test]
async fn lock_broadcast_refused_when_the_name_would_expire_during_the_lockup() {
    let (node, m) = broadcast_node(REGTEST_END - 21, 0).await;
    let (app, id) = signed_lock_sent_through(&node).await;
    let err = err_text(
        crate::commands::tx::broadcast_tx_draft(app.state(), id.clone())
            .await
            .unwrap_err(),
    );
    assert!(
        err.contains("before its transfer into the lock could be finalized"),
        "R31 refusal: {err}"
    );
    m[2].assert_async().await;
    with_db(&app, |c| {
        assert!(
            queries::get_tx_draft(c, &id).unwrap().is_none(),
            "draft discarded"
        );
    });
    assert!(open_listing(&app).is_none(), "the name is free again");

    let (node, m) = broadcast_node(REGTEST_END - 22, 1).await;
    let (app, id) = signed_lock_sent_through(&node).await;
    crate::commands::tx::broadcast_tx_draft(app.state(), id)
        .await
        .expect("one block earlier the lock is sent");
    m[2].assert_async().await;
}

// --- Finalize & sign (T3) ---------------------------------------------------

use crate::commands::shakedex::{prepare_lock_finalize, PreparedLockFinalize, StepInput};

/// Mined at TRANSFER_HEIGHT; on regtest the lockup is over once tip + 1 >=
/// TRANSFER_HEIGHT + 10, i.e. from tip TRANSFER_HEIGHT + 9.
const TRANSFER_HEIGHT: i64 = QUIET_TIP - 100;
const SIGN_MTP: u64 = 1_700_100_000;
const RENEWAL_BLOCK: &str = "aa00000000000000000000000000000000000000000000000000000000000000";

fn ready_tip(net: Network) -> i64 {
    TRANSFER_HEIGHT + i64::from(net.name_params().transfer_lockup) - 1
}

fn price(p: &str) -> Vec<StepInput> {
    vec![StepInput { price: p.into() }]
}

/// The lock TRANSFER coin as `GET /coin` sends it: at our address 0/0,
/// committing to `program` (the lock's, unless a test says otherwise).
fn lock_transfer_coin(net: Network, txid: &str, program: &[u8; 32], height: i64) -> Value {
    let nh = hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap());
    json!({
        "hash": txid, "index": 0, "value": NAME_VALUE, "address": addr00(net).0,
        "height": height, "coinbase": false, "version": 0,
        "covenant": { "type": COV_TRANSFER, "action": "TRANSFER", "items": [
            nh, hex::encode(NAME_HEIGHT.to_le_bytes()), "00", hex::encode(program) ] }
    })
}

/// `getnameinfo` once the lock TRANSFER is the owner, mined at
/// TRANSFER_HEIGHT.
fn locked_name_info(renewal: u64, lock_txid: &str) -> Value {
    let mut v = name_info(renewal, 0, lock_txid);
    v["info"]["transfer"] = TRANSFER_HEIGHT.into();
    v
}

/// `getblockchaininfo` at `tip`, with the median time when given.
fn chain_info(net: Network, tip: i64, mtp: Option<u64>) -> Value {
    let chain = if net == Network::Main {
        "main"
    } else {
        "regtest"
    };
    let mut v = json!({ "chain": chain, "blocks": tip, "headers": tip });
    if let Some(m) = mtp {
        v["mediantime"] = m.into();
    }
    v
}

struct Ready {
    net: Network,
    node: ServerGuard,
    mocks: Vec<Mock>,
    app: App,
    listing_id: String,
    lock_txid: String,
}

impl Ready {
    /// Swap the node's answers: tip and MTP, `getnameinfo`, the lock TRANSFER
    /// coin (`None`: hsd's 404).
    async fn node(&mut self, tip: i64, mtp: Option<u64>, info: Value, coin: Option<Value>) {
        let chain = chain_info(self.net, tip, mtp);
        self.node_with(chain, info, coin).await;
    }

    /// [`Ready::node`] with `getblockchaininfo`'s whole reply.
    async fn node_with(&mut self, chain: Value, info: Value, coin: Option<Value>) {
        for m in self.mocks.drain(..) {
            m.remove_async().await;
        }
        for (method, result) in [
            ("getblockchaininfo", chain),
            ("getblockhash", json!(RENEWAL_BLOCK)),
        ] {
            let m = self
                .node
                .mock("POST", "/")
                .match_body(mockito::Matcher::PartialJson(json!({ "method": method })))
                .with_header("content-type", "application/json")
                .with_body(rpc_ok(result))
                .create_async()
                .await;
            self.mocks.push(m);
        }
        self.mocks.push(mock_name_info(&mut self.node, info).await);
        let path = format!("/coin/{}/0", self.lock_txid);
        self.mocks.push(match coin {
            Some(c) => mock_coin(&mut self.node, &self.lock_txid, c).await,
            None => {
                self.node
                    .mock("GET", path.as_str())
                    .with_status(404)
                    .create_async()
                    .await
            }
        });
    }

    fn key(&self) -> crate::noncustodial::shakedex::lock_key::LockKey {
        derive_lock_key(&master(), self.net, 0, NAME).unwrap()
    }

    /// The lock TRANSFER coin as the node reports it once mined.
    fn coin(&self) -> Value {
        lock_transfer_coin(
            self.net,
            &self.lock_txid,
            &self.key().program,
            TRANSFER_HEIGHT,
        )
    }

    fn info(&self) -> Value {
        locked_name_info(RENEWAL, &self.lock_txid)
    }

    fn listing(&self) -> ShakedexListing {
        with_db(&self.app, |c| {
            queries::get_shakedex_listing(c, &self.listing_id)
                .unwrap()
                .unwrap()
        })
    }
}

/// NAME locked on `network`, its lock TRANSFER mined at TRANSFER_HEIGHT and
/// the owner, the lockup over, the listing ReadyToFinalize; the node answers
/// as hsd would at the first tip the FINALIZE is valid.
async fn ready_fixture(network: &str) -> Ready {
    let net = net_of(network);
    let (node, mocks, app) = lock_fixture(network, "mnemonic_hot", QUIET_TIP).await;
    if net == Network::Main {
        // Locking a name on mainnet needs the flag (R15); the tests then
        // set it as they need.
        with_db(&app, |c| set(c, "shakedex_experimental", "true"));
    }
    let lock_txid = locked_on_chain_on(&app, net).await;
    let listing_id = open_listing(&app).unwrap().id;
    with_db(&app, |c| {
        c.execute(
            "UPDATE shakedex_listings SET state = 'ready_to_finalize' WHERE id = ?1",
            [&listing_id],
        )
        .unwrap();
    });
    let mut r = Ready {
        net,
        node,
        mocks,
        app,
        listing_id,
        lock_txid,
    };
    let (info, coin) = (r.info(), r.coin());
    r.node(ready_tip(net), Some(SIGN_MTP), info, Some(coin))
        .await;
    r
}

async fn prepare(
    r: &Ready,
    prices: &[StepInput],
) -> Result<PreparedLockFinalize, crate::error::AppError> {
    prepare_lock_finalize(&r.app.state(), &r.listing_id, prices, None).await
}

/// A refusal wrote nothing: the listing is still ReadyToFinalize and only
/// the lock draft exists.
fn assert_nothing_finalized(r: &Ready, drafts: i64) {
    let l = r.listing();
    assert_eq!(l.state, ListingState::ReadyToFinalize);
    assert_eq!(l.lock_finalize_draft_id, None);
    assert_eq!(l.steps_json, "[]");
    assert_eq!(count(&r.app, "wallet_tx_drafts"), drafts);
}

/// The fixture's node lets Finalize & sign through, and what it built is
/// the FINALIZE of our TRANSFER coin into the lock of the re-derived key.
#[tokio::test]
async fn prepare_lock_finalize_builds_the_finalize_into_the_lock() {
    let r = ready_fixture("regtest").await;
    let p = prepare(&r, &price("5")).await.expect("ready");
    let key = r.key();
    assert_eq!(p.key.pubkey, key.pubkey);
    assert_eq!(p.prices, vec![5_000_000]);
    assert_eq!(
        sell::lock_output(&p.plan.plan, &key.address).unwrap(),
        (0, NAME_VALUE)
    );
    assert_eq!(p.mtp, SIGN_MTP);
    assert_eq!(p.lock_time, sell::buy_now_lock_time(SIGN_MTP));
    assert_eq!(p.listing.id, r.listing_id);
    assert_eq!(p.payment_address, r.listing().payment_address.unwrap());
    let plan = &p.plan.plan;
    assert_eq!(plan.inputs[0].txid, r.lock_txid, "spends the lock TRANSFER");
    assert_eq!(plan.inputs[0].vout, 0);
    assert_eq!(plan.outputs[0].address, key.address, "into the lock");
    assert_eq!(plan.outputs[0].value, NAME_VALUE);
    assert_eq!(
        plan.outputs[0].covenant_type,
        crate::noncustodial::sync::COV_FINALIZE
    );
    assert_eq!(
        plan.outputs[0].covenant_items_hex[6], RENEWAL_BLOCK,
        "the renewal block the node reported"
    );
    assert_nothing_finalized(&r, 1);
}

/// R18 at Finalize & sign: the node's confirmed lock TRANSFER must commit to
/// SHA3-256 of `lock_script(derived pub)`. A TRANSFER committing to any
/// other program (another key's lock, another address) is refused before
/// anything is built, asked or written, also when the database's copy of
/// the public key says the other key (the key is re-derived, never read
/// back); so is one the node reports without a readable covenant (could not
/// check).
#[tokio::test]
async fn refuses_finalize_on_commitment_mismatch() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    let other = derive_lock_key(&master(), net, 0, "othername").unwrap();
    let lock_txid = r.lock_txid.clone();
    let other_coin = lock_transfer_coin(net, &lock_txid, &other.program, TRANSFER_HEIGHT);
    let info = r.info();
    r.node(
        ready_tip(net),
        Some(SIGN_MTP),
        info.clone(),
        Some(other_coin.clone()),
    )
    .await;
    let err = prepare(&r, &price("5")).await.err().expect("refused");
    assert!(err_text(err).contains(sell::LOCK_COMMITMENT_MISMATCH));

    // The stored public key says the other key too: still refused.
    let stored = r.listing().lock_pubkey_hex;
    with_db(&r.app, |c| {
        c.execute(
            "UPDATE shakedex_listings SET lock_pubkey_hex = ?1 WHERE id = ?2",
            params![hex::encode(other.pubkey), r.listing_id],
        )
        .unwrap();
    });
    let err = prepare(&r, &price("5")).await.err().expect("refused");
    assert!(err_text(err).contains(sell::LOCK_COMMITMENT_MISMATCH));
    with_db(&r.app, |c| {
        c.execute(
            "UPDATE shakedex_listings SET lock_pubkey_hex = ?1 WHERE id = ?2",
            params![stored, r.listing_id],
        )
        .unwrap();
    });

    let good = r.coin();
    let mut no_cov = good.clone();
    no_cov.as_object_mut().unwrap().remove("covenant");
    let mut short_items = good.clone();
    short_items["covenant"]["items"]
        .as_array_mut()
        .unwrap()
        .truncate(3);
    let mut not_transfer = good.clone();
    not_transfer["covenant"]["type"] = crate::noncustodial::sync::COV_UPDATE.into();
    let mut bad_height = good.clone();
    bad_height["covenant"]["items"][1] = json!("zz");
    for (case, coin) in [
        ("no covenant", no_cov),
        ("no address items", short_items),
        ("not a TRANSFER", not_transfer),
        ("unreadable name height", bad_height),
    ] {
        r.node(ready_tip(net), Some(SIGN_MTP), info.clone(), Some(coin))
            .await;
        let err = prepare(&r, &price("5")).await.err().expect(case);
        assert!(
            matches!(err, crate::error::AppError::Rpc(_)),
            "{case}: could not check: {err:?}"
        );
    }
    assert_nothing_finalized(&r, 1);
}

/// R19/R18: the name must still be held by our lock TRANSFER (the owner,
/// output 0, of an unrevoked name), and the TRANSFER must be mined and
/// unspent; each missing field is "could not check".
#[tokio::test]
async fn finalize_and_sign_refused_when_the_name_left_its_lock_transfer() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    let lock_txid = r.lock_txid.clone();
    let coin = r.coin();
    let mut moved = locked_name_info(RENEWAL, &"77".repeat(32));
    moved["info"]["transfer"] = 0.into();
    let mut other_output = r.info();
    other_output["info"]["owner"]["index"] = 1.into();
    let mut revoked = r.info();
    revoked["info"]["revoked"] = (QUIET_TIP - 1).into();
    let mut no_owner = r.info();
    no_owner["info"].as_object_mut().unwrap().remove("owner");
    let mut no_index = r.info();
    no_index["info"]["owner"]
        .as_object_mut()
        .unwrap()
        .remove("index");
    let mut no_revoked = r.info();
    no_revoked["info"]
        .as_object_mut()
        .unwrap()
        .remove("revoked");
    let mut no_info = r.info();
    no_info.as_object_mut().unwrap().remove("info");
    let mut no_height = coin.clone();
    no_height.as_object_mut().unwrap().remove("height");
    let mempool = lock_transfer_coin(net, &lock_txid, &r.key().program, -1);
    for (case, info, coin, needle) in [
        (
            "owner moved",
            moved,
            Some(coin.clone()),
            "no longer held by its lock transfer",
        ),
        (
            "another output owns it",
            other_output,
            Some(coin.clone()),
            "no longer held by its lock transfer",
        ),
        (
            "revoked",
            revoked,
            Some(coin.clone()),
            "no longer held by its lock transfer",
        ),
        ("no owner", no_owner, Some(coin.clone()), "could not check"),
        (
            "no owner index",
            no_index,
            Some(coin.clone()),
            "could not check",
        ),
        (
            "no revoked",
            no_revoked,
            Some(coin.clone()),
            "could not check",
        ),
        ("no info", no_info, Some(coin.clone()), "could not check"),
        ("coin spent", r.info(), None, "no longer unspent"),
        ("in the mempool", r.info(), Some(mempool), "not mined"),
        (
            "coin without a height",
            r.info(),
            Some(no_height),
            "could not check",
        ),
    ] {
        r.node(ready_tip(net), Some(SIGN_MTP), info, coin).await;
        let e = err_text(prepare(&r, &price("5")).await.err().expect(case));
        assert!(e.contains(needle), "{case}: {e}");
    }

    // The name expired and was registered again: the TRANSFER commits to
    // an older name height than the node's.
    let mut old_height = r.coin();
    old_height["covenant"]["items"][1] = json!(hex::encode((NAME_HEIGHT - 1).to_le_bytes()));
    r.node(ready_tip(net), Some(SIGN_MTP), r.info(), Some(old_height))
        .await;
    let e = err_text(prepare(&r, &price("5")).await.err().expect("re-registered"));
    assert!(e.contains("registered again"), "{e}");

    // The wallet's own records must hold the lock TRANSFER as the name's
    // coin (a sync behind the chain is refused, not guessed).
    r.node(ready_tip(net), Some(SIGN_MTP), r.info(), Some(r.coin()))
        .await;
    let row = with_db(&r.app, |c| {
        c.query_row(
            "SELECT owner_txid FROM tracked_name_states WHERE name = ?1",
            [NAME],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    });
    with_db(&r.app, |c| {
        c.execute(
            "UPDATE tracked_name_states SET owner_txid = ?1 WHERE name = ?2",
            params![OWNER_TXID, NAME],
        )
        .unwrap();
    });
    let e = err_text(prepare(&r, &price("5")).await.err().expect("wallet behind"));
    assert!(e.contains("has not seen the lock transfer"), "{e}");
    // Another TRANSFER coin of ours is the wallet's owner coin.
    with_db(&r.app, |c| {
        seed_owner_coin(c, net, &"88".repeat(32), COV_TRANSFER);
    });
    let e = err_text(prepare(&r, &price("5")).await.err().expect("other coin"));
    assert!(e.contains("has not seen the lock transfer"), "{e}");
    with_db(&r.app, |c| {
        c.execute(
            "UPDATE tracked_name_states SET owner_txid = ?1 WHERE name = ?2",
            params![row, NAME],
        )
        .unwrap();
    });

    // The node and the wallet disagree on the coin (value, address).
    let mut other_value = r.coin();
    other_value["value"] = (NAME_VALUE + 1).into();
    let mut no_address = r.coin();
    no_address.as_object_mut().unwrap().remove("address");
    for (case, coin) in [("value", other_value), ("address", no_address)] {
        r.node(ready_tip(net), Some(SIGN_MTP), r.info(), Some(coin))
            .await;
        let e = prepare(&r, &price("5")).await.err().expect(case);
        assert!(matches!(e, crate::error::AppError::Rpc(_)), "{case}: {e:?}");
    }

    // The tip the lockup is judged at must come from the node.
    let mut no_tip = chain_info(net, ready_tip(net), Some(SIGN_MTP));
    no_tip.as_object_mut().unwrap().remove("blocks");
    r.node_with(no_tip, r.info(), Some(r.coin())).await;
    let e = err_text(prepare(&r, &price("5")).await.err().expect("no tip"));
    assert!(e.contains("could not check"), "no tip: {e}");
    assert_nothing_finalized(&r, 1);
}

/// R19: Finalize & sign opens once the transfer lockup is over at the next
/// block (blocks_until_finalize == 0), and a listing still Locking is
/// refused whatever the node says.
#[tokio::test]
async fn finalize_and_sign_refused_before_the_lockup() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    r.node(ready_tip(net) - 1, Some(SIGN_MTP), r.info(), Some(r.coin()))
        .await;
    let e = err_text(
        prepare(&r, &price("5"))
            .await
            .err()
            .expect("one block early"),
    );
    assert!(e.contains("lockup ends in 1 block"), "{e}");
    r.node(ready_tip(net), Some(SIGN_MTP), r.info(), Some(r.coin()))
        .await;
    prepare(&r, &price("5")).await.expect("at the lockup's end");

    for (state, needle) in [
        ("locking", "after the transfer lockup"),
        ("finalizing", "already finalized into its lock"),
        ("listed", "already finalized into its lock"),
    ] {
        with_db(&r.app, |c| {
            c.execute(
                "UPDATE shakedex_listings SET state = ?1 WHERE id = ?2",
                params![state, r.listing_id],
            )
            .unwrap();
        });
        let e = err_text(prepare(&r, &price("5")).await.err().expect(state));
        assert!(e.contains(needle), "{state}: {e}");
    }
}

/// R31 at Finalize & sign, with 0 lockup left: refused while the expiry end
/// is at or below tip + 1 + day (regtest day = 10), built one block earlier.
/// The node's own renewal height decides, read from getnameinfo.
#[tokio::test]
async fn finalize_and_sign_refused_when_the_name_would_expire_first() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    let p = net.name_params();
    // A name close to its expiry: the lock TRANSFER mined a renewal window
    // after TRANSFER_HEIGHT, Finalize & sign at the first tip it is valid.
    let transfer_height = TRANSFER_HEIGHT + i64::from(p.renewal_window);
    let tip = transfer_height + i64::from(p.transfer_lockup) - 1;
    let mut coin = r.coin();
    coin["height"] = transfer_height.into();
    let lock_txid = r.lock_txid.clone();
    let info = |renewal: u64| {
        let mut v = locked_name_info(renewal, &lock_txid);
        v["info"]["transfer"] = transfer_height.into();
        v
    };
    // expiry end = renewal + window; refused when end <= tip + 1 + day.
    let refused_renewal =
        u64::try_from(tip + 1 + i64::from(p.margin_day()) - i64::from(p.renewal_window)).unwrap();
    r.node(
        tip,
        Some(SIGN_MTP),
        info(refused_renewal),
        Some(coin.clone()),
    )
    .await;
    let e = err_text(prepare(&r, &price("5")).await.err().expect("refused"));
    assert!(e.contains("too soon to finalize it into the lock"), "{e}");
    r.node(
        tip,
        Some(SIGN_MTP),
        info(refused_renewal + 1),
        Some(coin.clone()),
    )
    .await;
    prepare(&r, &price("5"))
        .await
        .expect("one block more is enough");

    for field in ["renewal", "claimed", "height", "renewals", "weak"] {
        let mut missing = info(refused_renewal + 1);
        missing["info"].as_object_mut().unwrap().remove(field);
        r.node(tip, Some(SIGN_MTP), missing, Some(coin.clone()))
            .await;
        let e = prepare(&r, &price("5")).await.err().expect(field);
        assert!(
            matches!(e, crate::error::AppError::Rpc(_)),
            "{field}: {e:?}"
        );
    }
    assert_nothing_finalized(&r, 1);
}

/// R19: prices a person can type are refused in the backend with the
/// reason; a Buy Now takes exactly one.
#[tokio::test]
async fn step_prices_out_of_range_are_refused() {
    let r = ready_fixture("regtest").await;
    for (text, why) in [
        ("0", "above 0"),
        ("0.0009", "dust"),
        ("2040000000.000001", "money supply"),
        ("1.0000001", "6 decimals"),
        ("five", "not a number"),
    ] {
        let e = err_text(prepare(&r, &price(text)).await.err().expect(text));
        assert!(e.contains(why), "{text}: {e}");
    }
    for prices in [
        vec![],
        vec![
            StepInput { price: "5".into() },
            StepInput { price: "4".into() },
        ],
    ] {
        let e = err_text(prepare(&r, &prices).await.err().expect("one price"));
        assert!(e.contains("exactly one price"), "{e}");
    }
    assert_eq!(
        prepare(&r, &price("2040000000"))
            .await
            .expect("the supply")
            .prices,
        vec![crate::noncustodial::shakedex::purchase::MAX_MONEY]
    );
    assert_nothing_finalized(&r, 1);
}

/// R15/R29: Finalize & sign acts on an existing listing, so the mainnet
/// experimental flag does not gate it.
#[tokio::test]
async fn finalize_and_sign_ignores_experimental_flag() {
    let r = ready_fixture("mainnet").await;
    with_db(&r.app, |c| set(c, "shakedex_experimental", "false"));
    prepare(&r, &price("5"))
        .await
        .expect("not gated by the flag");
}

/// The Cancel transfer linked to the listing spends the same TRANSFER coin
/// as the FINALIZE: while it may still be mined (unsent, or sent and not
/// given up) Finalize & sign is refused. A dropped, failed or deleted cancel
/// does not hold it back.
#[tokio::test]
async fn finalize_and_sign_refused_while_the_cancel_transfer_is_alive() {
    let r = ready_fixture("regtest").await;
    let link = |id: &str, status: &str| {
        let id = id.to_string();
        with_db(&r.app, |c| {
            queries::insert_tx_draft(c, &id, PROFILE, "cancel", "00", "{}", "{}").unwrap();
            c.execute(
                "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = ?2",
                params![status, id],
            )
            .unwrap();
            c.execute(
                "UPDATE shakedex_listings SET abort_draft_id = ?1, abort_txid = ?2 WHERE id = ?3",
                params![id, "c0".repeat(32), r.listing_id],
            )
            .unwrap();
        });
        id
    };
    for status in ["draft", "signed", "broadcast_pending", "broadcasted"] {
        link(&format!("cancel-{status}"), status);
        let e = err_text(prepare(&r, &price("5")).await.err().expect(status));
        assert!(e.contains(sell::CANCEL_TRANSFER_PENDING), "{status}: {e}");
    }
    for status in ["dropped", "failed"] {
        link(&format!("cancel-{status}"), status);
        prepare(&r, &price("5"))
            .await
            .unwrap_or_else(|e| panic!("{status}: {e:?}"));
    }
    let id = link("cancel-deleted", "draft");
    with_db(&r.app, |c| queries::delete_tx_draft(c, &id).unwrap());
    assert_eq!(r.listing().abort_draft_id, None, "deleted: unlinked");
    prepare(&r, &price("5")).await.expect("deleted cancel");
}

/// R16/R6: the Shakedex gates, and the unlocked signer.
#[tokio::test]
async fn finalize_and_sign_refused_for_ledger_and_watch_only() {
    let r = ready_fixture("regtest").await;
    for kind in ["ledger_hardware", "xpriv_hot"] {
        with_db(&r.app, |c| {
            c.execute(
                "UPDATE wallet_profiles SET kind = ?1 WHERE id = ?2",
                params![kind, PROFILE],
            )
            .unwrap();
        });
        let e = err_text(prepare(&r, &price("5")).await.err().expect(kind));
        assert!(
            e.contains(crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY),
            "{kind}: {e}"
        );
    }
}

#[tokio::test]
async fn finalize_and_sign_refused_without_write_capability() {
    let r = ready_fixture("regtest").await;
    with_db(&r.app, |c| set(c, "chain_source", "explorer"));
    let e = err_text(prepare(&r, &price("5")).await.err().expect("refused"));
    assert!(e.contains(crate::commands::shakedex::NEEDS_SENDING_NODE));
}

#[tokio::test]
async fn finalize_and_sign_needs_the_unlocked_signer() {
    let r = ready_fixture("regtest").await;
    *r.app.state::<AppState>().signer.lock().unwrap() = None;
    let e = prepare(&r, &price("5")).await.err().expect("locked");
    assert!(matches!(e, crate::error::AppError::WalletLocked), "{e:?}");
    for m in &r.mocks {
        assert!(
            !m.matched_async().await,
            "nothing is read before the signer"
        );
    }
}

/// A listing of another profile is not found, whatever its id.
#[tokio::test]
async fn finalize_and_sign_acts_only_on_the_active_profiles_listing() {
    let r = ready_fixture("regtest").await;
    let e = prepare_lock_finalize(&r.app.state(), "no-such-listing", &price("5"), None)
        .await
        .err()
        .expect("unknown");
    assert!(matches!(e, crate::error::AppError::NotFound(_)), "{e:?}");
    with_db(&r.app, |c| {
        queries::insert_wallet_profile(
            c,
            "other",
            "Other",
            "mnemonic_hot",
            "regtest",
            "xpubX",
            0,
            false,
        )
        .unwrap();
        c.execute(
            "UPDATE shakedex_listings SET wallet_profile_id = 'other' WHERE id = ?1",
            [&r.listing_id],
        )
        .unwrap();
    });
    let e = prepare(&r, &price("5")).await.err().expect("not ours");
    assert!(matches!(e, crate::error::AppError::NotFound(_)), "{e:?}");
}

/// R19: the Buy Now lock time comes from the node's median time; without it
/// nothing is signed.
#[tokio::test]
async fn finalize_and_sign_refused_without_the_median_time() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    r.node(ready_tip(net), None, r.info(), Some(r.coin())).await;
    let e = prepare(&r, &price("5")).await.err().expect("refused");
    assert!(
        matches!(&e, crate::error::AppError::Rpc(m) if m.contains("median time")),
        "{e:?}"
    );
}

/// Deviation 6: a reverse auction's schedule arrives with T8.
#[tokio::test]
async fn finalize_and_sign_refused_for_a_reverse_auction_until_t8() {
    let r = ready_fixture("regtest").await;
    with_db(&r.app, |c| {
        c.execute(
            "UPDATE shakedex_listings SET mode = 'reverse_auction' WHERE id = ?1",
            [&r.listing_id],
        )
        .unwrap();
    });
    let e = err_text(prepare(&r, &price("5")).await.err().expect("refused"));
    assert!(e.contains("reverse auctions are not supported yet"), "{e}");
}

// --- Finalize & sign: confirm, sign, store; export (T3) ---------------------

use crate::commands::secure_prompt::{push_test_answer, take_test_requests, SecurePromptResult};
use crate::commands::shakedex::{
    export_listing_file_from_conn, finalize_and_sign_confirmed, ListingSummary,
};
use crate::noncustodial::shakedex::listing_file::ListingFile;
use crate::noncustodial::shakedex::template::{
    encode_lock_time, is_valid_at, secs_until_valid, verify_step_signature, StepTemplate,
};
use crate::noncustodial::sync::COV_FINALIZE;

/// Clear the request record and queue the answer to the next prompt.
fn answer(confirmed: bool) {
    let _ = take_test_requests();
    push_test_answer(SecurePromptResult {
        value: None,
        confirmed,
    });
}

async fn finalize_and_sign(
    r: &Ready,
    prices: &[StepInput],
) -> Result<ListingSummary, crate::error::AppError> {
    finalize_and_sign_confirmed(&r.app.state(), r.app.handle(), &r.listing_id, prices, None).await
}

/// A node mock that fails the test if anything is sent.
async fn no_broadcast(r: &mut Ready) -> Mock {
    r.node
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            json!({ "method": "sendrawtransaction" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!("00")))
        .expect(0)
        .create_async()
        .await
}

/// R19 day 2 end to end: one confirmation, then the FINALIZE into the lock
/// is a signed draft spending our lock TRANSFER into a FINALIZE at the lock
/// address, the listing is Finalizing with the lock outpoint (the
/// FINALIZE's txid and the index of its FINALIZE output), and its one step
/// and its listing file are signed over that outpoint, with that output's
/// value, by the lock key, and verify. Nothing is sent.
#[tokio::test]
async fn lock_then_finalize_and_sign() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    let key = derive_lock_key(&master(), net, 0, NAME).unwrap();
    let sent = no_broadcast(&mut r).await;
    answer(true);
    let s = finalize_and_sign(&r, &price("5"))
        .await
        .expect("finalize & sign");
    assert_eq!(s.state, ListingState::Finalizing);
    assert_eq!(
        (s.id.as_str(), s.name.as_str()),
        (r.listing_id.as_str(), NAME)
    );

    let draft_id = s.finalize_draft_id.clone().expect("the FINALIZE draft");
    let row = with_db(&r.app, |c| {
        queries::get_tx_draft(c, &draft_id).unwrap().unwrap()
    });
    assert_eq!(row.action, sell::LOCK_FINALIZE_ACTION);
    assert_eq!(row.status, "signed", "sent next by broadcast_tx_draft");
    assert!(row.signed_tx_hex.is_some());
    sent.assert_async().await;
    let plan: DraftPlan = serde_json::from_str(&row.signing_inputs_json).unwrap();
    assert_eq!(
        (plan.inputs[0].txid.as_str(), plan.inputs[0].vout),
        (r.lock_txid.as_str(), 0)
    );
    let lock_outputs: Vec<usize> = plan
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.covenant_type == COV_FINALIZE)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(lock_outputs.len(), 1, "one FINALIZE output");
    let lock_vout = lock_outputs[0];
    let lock_out = &plan.outputs[lock_vout];
    assert_eq!(lock_out.address, key.address);
    assert_eq!(lock_out.value, NAME_VALUE);
    let finalize_txid: String = serde_json::from_str::<Value>(&row.summary_json).unwrap()["txid"]
        .as_str()
        .unwrap()
        .into();

    let l = r.listing();
    assert_eq!(l.lock_finalize_draft_id.as_deref(), Some(draft_id.as_str()));
    assert_eq!(
        (l.lock_txid.as_deref(), l.lock_vout),
        (Some(finalize_txid.as_str()), Some(lock_vout as i64)),
        "the lock outpoint is the FINALIZE output of the plan"
    );
    assert_eq!(
        (s.lock_txid.clone(), s.lock_vout),
        (l.lock_txid.clone(), l.lock_vout)
    );
    assert_eq!(s.steps.len(), 1);
    let step = &s.steps[0];
    assert_eq!(step.price, 5_000_000);
    let mut outpoint = [0u8; 32];
    hex::decode_to_slice(&finalize_txid, &mut outpoint).unwrap();
    let sig: [u8; 65] = hex::decode(&step.signature).unwrap().try_into().unwrap();
    let pubkey: [u8; 33] = hex::decode(&l.lock_pubkey_hex).unwrap().try_into().unwrap();
    assert_eq!(pubkey, key.pubkey, "the stored pubkey is the lock key's");
    verify_step_signature(
        &StepTemplate {
            lock_outpoint: (outpoint, lock_vout as u32),
            lock_value: lock_out.value,
            lock_pubkey: &pubkey,
            payment: crate::noncustodial::tx::output_address_from_string(
                net,
                l.payment_address.as_deref().unwrap(),
            )
            .unwrap(),
            price: 5_000_000,
            lock_time_secs: step.lock_time,
        },
        &sig,
    )
    .expect("the step verifies over the FINALIZE's lock outpoint");

    let file =
        ListingFile::parse(l.listing_file_json.as_deref().expect("file saved"), net).unwrap();
    assert_eq!(
        (file.lock_txid, file.lock_vout),
        (outpoint, lock_vout as u32)
    );
    assert_eq!(file.public_key, key.pubkey);
    assert_eq!(file.payment_addr, l.payment_address.clone().unwrap());
    assert_eq!(file.steps.len(), 1);
    assert_eq!(file.steps[0].signature, sig);
    assert_eq!(
        (file.steps[0].price, file.steps[0].lock_time),
        (step.price, step.lock_time)
    );
    assert_eq!(
        file.expires_at,
        Some(SIGN_MTP + sell::LISTING_LIFETIME_SECS)
    );
    assert_eq!(
        l.expires_at,
        Some((SIGN_MTP + sell::LISTING_LIFETIME_SECS) as i64)
    );
    assert_eq!(s.expires_at, l.expires_at);
    let stored: Vec<sell::StoredStep> = serde_json::from_str(&l.steps_json).unwrap();
    assert_eq!(stored, s.steps);

    // Once is enough: the listing is no longer ReadyToFinalize.
    answer(true);
    let e = err_text(
        finalize_and_sign(&r, &price("5"))
            .await
            .expect_err("second run"),
    );
    assert!(e.contains("already finalized into its lock"), "{e}");
}

/// R19: a Buy Now is valid in the next block at the MTP it was signed at,
/// also at an MTP on a 512-second boundary, where a lock time equal to the
/// MTP would not be.
#[tokio::test]
async fn buy_now_valid_immediately() {
    let r = ready_fixture("regtest").await;
    answer(true);
    let s = finalize_and_sign(&r, &price("5")).await.unwrap();
    let lt = s.steps[0].lock_time;
    assert!(is_valid_at(encode_lock_time(lt).unwrap(), SIGN_MTP));
    assert_eq!(secs_until_valid(lt, SIGN_MTP), 0);

    let mut r = ready_fixture("regtest").await;
    let aligned = SIGN_MTP / 512 * 512;
    assert!(!is_valid_at(encode_lock_time(aligned).unwrap(), aligned));
    let (info, coin) = (r.info(), r.coin());
    r.node(ready_tip(r.net), Some(aligned), info, Some(coin))
        .await;
    answer(true);
    let s = finalize_and_sign(&r, &price("5")).await.unwrap();
    let lt = s.steps[0].lock_time;
    assert!(is_valid_at(encode_lock_time(lt).unwrap(), aligned));
    assert_eq!(secs_until_valid(lt, aligned), 0);
}

/// R19 (review focus 2): the lock time is the MTP at signing minus 512 s,
/// not anything from the day the name was locked; the app may have been
/// closed through the lockup. The listing's expiry counts from that same
/// MTP.
#[tokio::test]
async fn buy_now_lock_time_is_taken_at_signing() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    let days_later = SIGN_MTP + 9 * 86_400;
    let (info, coin) = (r.info(), r.coin());
    r.node(ready_tip(net) + 1_000, Some(days_later), info, Some(coin))
        .await;
    answer(true);
    let s = finalize_and_sign(&r, &price("5")).await.unwrap();
    assert_eq!(s.steps[0].lock_time, days_later - 512);
    assert_eq!(s.steps[0].lock_time, sell::buy_now_lock_time(days_later));
    assert_eq!(
        s.expires_at,
        Some((days_later + sell::LISTING_LIFETIME_SECS) as i64)
    );
}

/// R19: without the node's median time nothing is asked, signed or written.
#[tokio::test]
async fn finalize_and_sign_without_the_median_time_asks_and_writes_nothing() {
    let mut r = ready_fixture("regtest").await;
    let net = Network::Regtest;
    let (info, coin) = (r.info(), r.coin());
    r.node(ready_tip(net), None, info, Some(coin)).await;
    answer(true);
    let e = finalize_and_sign(&r, &price("5"))
        .await
        .expect_err("refused");
    assert!(
        matches!(&e, crate::error::AppError::Rpc(m) if m.contains("median time")),
        "{e:?}"
    );
    assert!(take_test_requests().is_empty(), "nothing asked");
    assert_nothing_finalized(&r, 1);
}

/// R20: the confirmation is its own prompt, not the transaction one, and
/// lists the FINALIZE's fee, every step with its price and when it is
/// valid, and the permanence warning — before anything is signed.
#[tokio::test]
async fn confirmation_lists_every_step() {
    let r = ready_fixture("regtest").await;
    answer(true);
    let s = finalize_and_sign(&r, &price("5")).await.unwrap();
    let reqs = take_test_requests();
    assert_eq!(
        reqs.len(),
        1,
        "one confirmation for the FINALIZE and every step"
    );
    let req = &reqs[0];
    assert_eq!(req.mode, "confirm");
    assert_eq!(
        req.title,
        crate::commands::shakedex::FINALIZE_AND_SIGN_TITLE
    );
    assert_ne!(req.title, "Confirm transaction");
    let rows = req.details.as_ref().unwrap()["rows"]
        .as_array()
        .unwrap()
        .clone();
    let value = |label: &str| {
        rows.iter()
            .find(|r| r["label"] == label)
            .map(|r| r["value"].as_str().unwrap().to_string())
    };
    let fee = with_db(&r.app, |c| {
        let d = queries::get_tx_draft(c, s.finalize_draft_id.as_deref().unwrap())
            .unwrap()
            .unwrap();
        serde_json::from_str::<Value>(&d.summary_json).unwrap()["feeDoos"]
            .as_i64()
            .unwrap()
    });
    assert!(fee > 0);
    assert_eq!(
        value("Network fee (finalize into the lock)"),
        Some(crate::noncustodial::types::doos_to_hns_string(fee))
    );
    assert_eq!(
        value("Price step 1").as_deref(),
        Some("5.000000 HNS, valid at once")
    );
    assert_eq!(
        value("Paid to"),
        r.listing().payment_address,
        "the reserved payment address"
    );
    assert_eq!(value("Lock address"), Some(r.key().address));
    assert_eq!(
        value("Warning").as_deref(),
        Some(sell::STEP_SIGNATURE_PERMANENCE)
    );
}

/// R20: cancelling the confirmation signs and writes nothing.
#[tokio::test]
async fn cancelled_confirmation_writes_nothing() {
    let r = ready_fixture("regtest").await;
    answer(false);
    let e = finalize_and_sign(&r, &price("5"))
        .await
        .expect_err("rejected");
    assert!(matches!(e, crate::error::AppError::UserRejected), "{e:?}");
    assert_eq!(take_test_requests().len(), 1, "it was asked");
    let l = r.listing();
    assert_eq!(l.state, ListingState::ReadyToFinalize);
    assert_eq!((l.steps_json.as_str(), l.listing_file_json), ("[]", None));
    assert_eq!((l.lock_txid, l.lock_vout, l.expires_at), (None, None, None));
    assert_eq!(count(&r.app, "wallet_tx_drafts"), 1, "only the lock draft");
}

/// R19, coordinator (a): the FINALIZE draft and the Finalizing listing
/// commit together. When the listing cannot move (it changed while the
/// prompt was open), the draft is rolled back with it.
#[tokio::test]
async fn finalize_and_sign_writes_all_or_nothing() {
    let r = ready_fixture("regtest").await;
    // The listing leaves ReadyToFinalize as soon as the FINALIZE draft is
    // inserted, inside the same transaction.
    with_db(&r.app, |c| {
        c.execute_batch(
            "CREATE TRIGGER move_away AFTER INSERT ON wallet_tx_drafts
             BEGIN UPDATE shakedex_listings SET state = 'locking'; END;",
        )
        .unwrap();
    });
    answer(true);
    let e = err_text(
        finalize_and_sign(&r, &price("5"))
            .await
            .expect_err("refused"),
    );
    assert!(e.contains("this listing changed meanwhile"), "{e}");
    with_db(&r.app, |c| {
        c.execute_batch("DROP TRIGGER move_away").unwrap();
    });
    assert_nothing_finalized(&r, 1);
    assert_eq!(r.listing().listing_file_json, None);
}

/// R27: from Finalizing on, Cancel transfer is no longer the abort and is
/// refused.
#[tokio::test]
async fn cancel_transfer_refused_once_finalize_and_sign_ran() {
    let r = ready_fixture("regtest").await;
    answer(true);
    finalize_and_sign(&r, &price("5")).await.unwrap();
    let e = err_text(
        build_cancel_draft(r.app.state(), NAME.into(), None)
            .await
            .expect_err("refused"),
    );
    assert!(
        e.contains(crate::noncustodial::shakedex::NAME_LOCKED_FOR_SALE),
        "{e}"
    );
}

/// R23, deviation 3: the saved file leaves the wallet only once its
/// FINALIZE is mined (Listed and after); before that it would carry steps
/// over a coin that may never exist.
#[tokio::test]
async fn listing_file_is_exported_only_once_the_finalize_is_mined() {
    let r = ready_fixture("regtest").await;
    let export = || {
        with_db(&r.app, |c| {
            export_listing_file_from_conn(c, PROFILE, &r.listing_id)
        })
    };
    let set_state = |s: ListingState| {
        with_db(&r.app, |c| {
            c.execute(
                "UPDATE shakedex_listings SET state = ?1 WHERE id = ?2",
                params![s.as_str(), r.listing_id],
            )
            .unwrap();
        })
    };
    let refused = |what: &str| {
        let e = err_text(export().expect_err(what));
        assert!(
            e.contains("once the finalize into the lock is mined"),
            "{what}: {e}"
        );
    };
    refused("ReadyToFinalize");
    answer(true);
    finalize_and_sign(&r, &price("5")).await.unwrap();
    refused("Finalizing");
    set_state(ListingState::Locking);
    refused("Locking");
    set_state(ListingState::Listed);
    let file = export().expect("Listed");
    let saved = r.listing().listing_file_json;
    assert_eq!(Some(file), saved);
    let e = with_db(&r.app, |c| {
        export_listing_file_from_conn(c, "another-profile", &r.listing_id)
    })
    .expect_err("not ours");
    assert!(matches!(e, crate::error::AppError::NotFound(_)), "{e:?}");
}
