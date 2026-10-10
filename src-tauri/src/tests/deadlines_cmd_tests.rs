//! Command-level tests for `scan_deadline_notifications` (I1, Task 4): the IO
//! shell around the pure `commands::deadlines::scan_deadlines` core — config
//! loading, deadline collection (reveal from `bid_commitments`, renewal via
//! `compute_renewals`), and dedup-state persistence in the `settings` table.
//!
//! The actual OS notification dispatch is compiled out under `#[cfg(test)]`
//! (see `commands::deadlines::send_os_notification`) — there is no OS
//! notification center in CI — so these tests assert on `ScanOutcome`
//! (which deadlines were newly notified) and the persisted state, not on any
//! real system notification.

use crate::commands::deadlines::{scan_deadline_notifications, scan_deadline_notifications_on_day};
use crate::db;
use crate::tests::names_cmd_tests::{create_full_test_state, insert_valid_profile, mock_app_with};
use tauri::Manager;

const BLOCKS_PER_DAY: i64 = 144;
const RENEWAL_WINDOW: i64 = 105_120; // mainnet

fn enable_notifications(
    conn: &rusqlite::Connection,
    reveal_lead_blocks: &str,
    renewal_lead_days: &str,
) {
    db::queries::set_setting(conn, "deadline_notify_enabled", "true").unwrap();
    db::queries::set_setting(
        conn,
        "deadline_notify_reveal_lead_blocks",
        reveal_lead_blocks,
    )
    .unwrap();
    db::queries::set_setting(conn, "deadline_notify_renewal_lead_days", renewal_lead_days).unwrap();
}

fn seed_current_height(conn: &rusqlite::Connection, profile_id: &str, height: i64) {
    conn.execute(
        "UPDATE wallet_profiles SET last_synced_height = ?1 WHERE id = ?2",
        rusqlite::params![height, profile_id],
    )
    .unwrap();
}

fn seed_pending_bid(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    reveal_end_height: i64,
) {
    db::queries::insert_bid_commitment(
        conn,
        profile_id,
        name,
        "aabb",
        "rs1qaddr",
        0,
        0,
        1000,
        2000,
        &"11".repeat(32),
        &"22".repeat(32),
    )
    .unwrap();
    db::queries::set_auction_heights(conn, profile_id, &"22".repeat(32), 0, reveal_end_height)
        .unwrap();
}

fn seed_owned_name_near_renewal(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    renewal_height: i64,
) {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
             height, renewal_height)
         VALUES (?1, ?2, 'aa', 'CLOSED', 'deadbeef', 0, 100, ?3)",
        rusqlite::params![profile_id, name, renewal_height],
    )
    .unwrap();
    // Seed a matching unspent `name_control` UTXO so the name passes the
    // ownership gate in `read_owned_names_explorer`.
    conn.execute(
        "INSERT OR IGNORE INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES ('deadbeef', 0, ?1, 'rs1qtest', '00', 1000, 6, NULL, 'name_control', NULL)",
        rusqlite::params![profile_id],
    )
    .unwrap();
}

#[tokio::test]
async fn disabled_by_default_notifies_nothing_even_with_imminent_deadlines() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        seed_current_height(&conn, &id, 1_000);
        // Reveal window closes in 10 blocks — well within any reasonable lead.
        seed_pending_bid(&conn, &id, "imminent", 1_010);
        id
    };
    let _ = &profile_id;

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .expect("scan should succeed even when disabled");

    assert!(!outcome.enabled);
    assert!(
        outcome.notified.is_empty(),
        "must not notify while the feature is off by default"
    );
}

#[tokio::test]
async fn notifies_for_imminent_reveal_window_and_dedups_next_scan() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_current_height(&conn, &id, 1_000);
        // 50 blocks remaining: inside the 144-block lead.
        seed_pending_bid(&conn, &id, "closingsoon", 1_050);
        id
    };

    let app = mock_app_with(state);
    let first = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .expect("first scan should succeed");
    assert!(first.enabled);
    assert_eq!(first.notified.len(), 1);
    assert!(first.notified[0].key.contains("closingsoon"));
    assert!(first.notified[0].key.starts_with("reveal:"));

    // Second scan, nothing changed on-chain — must NOT re-notify.
    let second = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .expect("second scan should succeed");
    assert!(
        second.notified.is_empty(),
        "already-notified reveal deadline must not re-fire on the very next tick"
    );

    // Dedup state actually persisted in settings (not just held in memory).
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let settings = db::queries::get_settings(&conn).unwrap();
    let raw = settings
        .get("deadline_notify_state")
        .expect("dedup state must be persisted");
    assert!(raw.contains(&format!("reveal:{profile_id}:closingsoon")));
}

#[tokio::test]
async fn does_not_notify_for_reveal_far_from_lead_time() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_current_height(&conn, &id, 1_000);
        // 10,000 blocks remaining: far outside the 144-block lead.
        seed_pending_bid(&conn, &id, "faraway", 11_000);
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(outcome.notified.is_empty());
}

#[tokio::test]
async fn revealed_bid_is_excluded_even_if_the_window_would_be_imminent() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_current_height(&conn, &id, 1_000);
        seed_pending_bid(&conn, &id, "alreadyrevealed", 1_010);
        db::queries::set_bid_reveal_txid(
            &conn,
            &id,
            "alreadyrevealed",
            &"22".repeat(32),
            "revealtxid",
        )
        .unwrap();
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(
        outcome.notified.is_empty(),
        "a bid that already revealed has no more reveal deadline"
    );
}

#[tokio::test]
async fn bid_commitment_without_a_persisted_reveal_end_height_is_skipped() {
    // Simulates a commitment written by `recover_bid_commitment` (or any
    // pre-existing row from before this column existed) — honestly excluded
    // rather than guessed, per the migration's doc comment.
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_current_height(&conn, &id, 1_000);
        db::queries::insert_bid_commitment(
            &conn,
            &id,
            "legacybid",
            "aabb",
            "rs1qaddr",
            0,
            0,
            1000,
            2000,
            &"11".repeat(32),
            &"33".repeat(32),
        )
        .unwrap();
        // Deliberately no set_reveal_end_height call.
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(outcome.notified.is_empty());
}

#[tokio::test]
async fn notifies_for_imminent_renewal_reusing_task3_compute_renewals() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        // A fixed (recent) chain renewal height; the CURRENT height is what
        // moves to control days-until-expire, matching `renewals_tests.rs`'s
        // convention (`compute_renewals` computes days from renewal_height +
        // window - current_height, so it's current_height that must sit near
        // the end of the window for "10 days left").
        let renewal_height = 1_000;
        let height = renewal_height + RENEWAL_WINDOW - 10 * BLOCKS_PER_DAY; // ~10 days left
        seed_current_height(&conn, &id, height);
        seed_owned_name_near_renewal(&conn, &id, "duesoon", renewal_height);
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert_eq!(outcome.notified.len(), 1);
    assert!(outcome.notified[0].key.starts_with("renewal:"));
    assert!(outcome.notified[0].key.contains("duesoon"));
}

#[tokio::test]
async fn does_not_notify_for_renewal_far_from_lead_time() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        let renewal_height = 1_000;
        // ~200 days out: far outside the 30-day lead.
        let height = renewal_height + RENEWAL_WINDOW - 200 * BLOCKS_PER_DAY;
        seed_current_height(&conn, &id, height);
        seed_owned_name_near_renewal(&conn, &id, "notyet", renewal_height);
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(outcome.notified.is_empty());
}

#[tokio::test]
async fn scan_with_no_wallet_profiles_is_a_harmless_noop() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        enable_notifications(&conn, "144", "30");
    }
    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(outcome.notified.is_empty());
    assert!(outcome.delivery_error.is_none());
}

// --- Finding 3: disabled scans do zero DB writes ----------------------

#[tokio::test]
async fn disabled_scan_does_not_touch_persisted_state() {
    // Notifications are OFF (default). Seed a recognizable sentinel dedup
    // state value up front so we can prove, byte-for-byte, that a disabled
    // scan never rewrites `deadline_notify_state` — not even to write back
    // the same value it read (Finding 3: it must not even READ it).
    const SENTINEL: &str = "[\"reveal:sentinel:untouched:1\"]";
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        seed_current_height(&conn, &id, 1_000);
        // Reveal window closes in 10 blocks — would be imminent if enabled.
        seed_pending_bid(&conn, &id, "imminent", 1_010);
        db::queries::set_setting(&conn, "deadline_notify_state", SENTINEL).unwrap();
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .expect("disabled scan should still succeed");
    assert!(!outcome.enabled);
    assert!(outcome.notified.is_empty());
    assert!(outcome.delivery_error.is_none());

    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(
        settings.get("deadline_notify_state").map(String::as_str),
        Some(SENTINEL),
        "a disabled scan must not write deadline_notify_state at all"
    );
}

// --- Finding 1b: a long-lapsed reveal window drops out of the scanner --

#[tokio::test]
async fn reveal_window_closed_more_than_one_reveal_period_ago_is_excluded() {
    // mainnet reveal_period = 1440 blocks (see `noncustodial/network.rs`).
    // 1441 blocks past close: one block PAST the "still worth a final alarm"
    // cutoff — must be excluded entirely, not merely deduped.
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_current_height(&conn, &id, 10_000);
        seed_pending_bid(&conn, &id, "ancienthistory", 10_000 - 1441);
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(
        outcome.notified.is_empty(),
        "a reveal window closed more than one reveal-period ago is dead, not merely deduped"
    );
}

#[tokio::test]
async fn reveal_window_closed_within_one_reveal_period_ago_still_notifies() {
    // Same setup, but only 1440 blocks past close (exactly one reveal
    // period) — still eligible for the one final "you missed it" alarm.
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_current_height(&conn, &id, 10_000);
        seed_pending_bid(&conn, &id, "justmissed", 10_000 - 1440);
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert_eq!(
        outcome.notified.len(),
        1,
        "a window closed exactly one reveal-period ago still gets its final alarm"
    );
}

// --- Shakedex purchase ready to finalize (R14) ---

fn seed_purchase(
    conn: &rusqlite::Connection,
    profile_id: &str,
    id: &str,
    name: &str,
    state: &str,
    blocks_remaining: Option<i64>,
) {
    db::queries::insert_shakedex_purchase(
        conn,
        &db::queries::ShakedexPurchase {
            id: id.into(),
            wallet_profile_id: profile_id.into(),
            name: name.into(),
            listing_json: "{}".into(),
            lock_txid: format!("{id}-lock"),
            lock_vout: 0,
            price_doos: 5_000_000,
            purchase_draft_id: format!("{id}-draft"),
            purchase_txid: format!("{id}-tx"),
            destination_address: "hs1qdest".into(),
            state: state.parse().unwrap(),
            purchase_height: Some(900),
            blocks_remaining,
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: None,
            finalize_draft_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .unwrap();
}

#[tokio::test]
async fn purchase_ready_to_finalize_notifies_once() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_purchase(&conn, &id, "p1", "bought", "awaiting_finalize", Some(0));
        // Not yet: still in the transfer lockup, or not mined.
        seed_purchase(&conn, &id, "p2", "waiting", "awaiting_finalize", Some(3));
        seed_purchase(&conn, &id, "p3", "pending", "unconfirmed", None);
        id
    };

    let app = mock_app_with(state);
    let first = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .expect("first scan should succeed");
    assert_eq!(first.notified.len(), 1, "got: {:?}", first.notified);
    let n = &first.notified[0];
    assert_eq!(
        n.key,
        format!("purchase_finalize:{profile_id}:bought:p1-tx")
    );
    assert_eq!(n.title, "Ready to finalize");
    assert_eq!(n.body, "bought is paid for — finalize it to make it yours");

    let second = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .expect("second scan should succeed");
    assert!(second.notified.is_empty(), "must not notify twice");

    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let raw = db::queries::get_settings(&conn).unwrap()["deadline_notify_state"].clone();
    assert!(raw.contains("purchase_finalize:"));
}

/// While our FINALIZE is on its way the purchase stays `awaiting_finalize`
/// until the job sees it mined; "finalize it" would then ask for a second one.
/// An unsent, failed or dropped finalize still leaves the name to finalize.
#[tokio::test]
async fn purchase_ready_to_finalize_skips_a_finalize_already_sent() {
    for (status, notifies) in [
        ("draft", true),
        ("signed", true),
        ("failed", true),
        ("dropped", true),
        ("broadcast_pending", false),
        ("broadcasted", false),
        ("confirmed", false),
    ] {
        let state = create_full_test_state();
        {
            let conn = state.db.lock().unwrap();
            let id = insert_valid_profile(&conn, "mainnet");
            enable_notifications(&conn, "144", "30");
            seed_purchase(&conn, &id, "p1", "bought", "awaiting_finalize", Some(0));
            db::queries::insert_tx_draft(
                &conn,
                "fin",
                &id,
                "shakedex_purchase_finalize",
                "00",
                "[]",
                "{}",
            )
            .unwrap();
            conn.execute(
                "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = 'fin'",
                [status],
            )
            .unwrap();
            db::queries::set_shakedex_purchase_finalize_draft(&conn, "p1", "fin").unwrap();
        }
        let app = mock_app_with(state);
        let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
            .await
            .unwrap();
        assert_eq!(outcome.notified.len(), usize::from(notifies), "{status}");
    }
}

#[tokio::test]
async fn purchase_ready_to_finalize_respects_disabled_setting() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        seed_purchase(&conn, &id, "p1", "bought", "awaiting_finalize", Some(0));
    }
    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state())
        .await
        .unwrap();
    assert!(!outcome.enabled);
    assert!(outcome.notified.is_empty());
}

// --- Shakedex listing ready to finalize (R19) ---

fn seed_listing(conn: &rusqlite::Connection, profile_id: &str, id: &str, name: &str, state: &str) {
    db::queries::insert_shakedex_listing(
        conn,
        &db::queries::ShakedexListing {
            id: id.into(),
            wallet_profile_id: profile_id.into(),
            name: name.into(),
            mode: db::queries::ListingMode::BuyNow,
            state: state.parse().unwrap(),
            lock_pubkey_hex: "02".repeat(33),
            lock_transfer_draft_id: None,
            lock_finalize_draft_id: None,
            lock_transfer_txid: Some(format!("{id}-tx")),
            lock_txid: None,
            lock_vout: None,
            payment_address: None,
            cancel_address: None,
            cancel_child_index: None,
            steps_json: "[]".into(),
            listing_file_json: None,
            publish: false,
            market_status: None,
            market_retry_at: None,
            market_attempts: 0,
            market_error: None,
            market_accepted: false,
            market_changed: false,
            expires_at: None,
            abort_draft_id: None,
            abort_txid: None,
            sold_txid: None,
            cancel_txid: None,
            cancel_draft_id: None,
            cancel_vout: None,
            cancel_finalize_draft_id: None,
            cancel_blocks_remaining: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .unwrap();
}

/// R19: the reminder appears once the lockup is over (ReadyToFinalize, not
/// Locking), comes once a day until Finalize & sign runs and its FINALIZE
/// is sent (a signed FINALIZE not sent yet locks nothing in), and is gone
/// from then on.
#[tokio::test]
async fn listing_ready_repeats() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_listing(&conn, &id, "l1", "forsale", "ready_to_finalize");
        seed_listing(&conn, &id, "l2", "stilllocking", "locking");
        id
    };
    let app = mock_app_with(state);
    let scan = |day| scan_deadline_notifications_on_day(app.handle().clone(), app.state(), day);
    let d1 = scan(20_000).await.unwrap();
    assert_eq!(d1.notified.len(), 1, "{:?}", d1.notified);
    assert_eq!(
        d1.notified[0].key,
        format!("listing_ready:{profile_id}:forsale:l1-tx:20000")
    );
    assert!(
        scan(20_000).await.unwrap().notified.is_empty(),
        "once a day"
    );
    let d2 = scan(20_001).await.unwrap();
    assert_eq!(d2.notified.len(), 1, "again the next day");

    let set_finalize = |status: &str| {
        let s: tauri::State<crate::AppState> = app.state();
        let conn = s.db.lock().unwrap();
        conn.execute(
            "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = 'fin'",
            [status],
        )
        .unwrap();
    };
    {
        let s: tauri::State<crate::AppState> = app.state();
        let conn = s.db.lock().unwrap();
        db::queries::insert_tx_draft(
            &conn,
            "fin",
            &profile_id,
            "shakedex_lock_finalize",
            "00",
            "{}",
            "{}",
        )
        .unwrap();
        conn.execute(
            "UPDATE shakedex_listings SET state = 'finalizing', lock_finalize_draft_id = 'fin' \
             WHERE id = 'l1'",
            [],
        )
        .unwrap();
    }
    set_finalize("signed");
    let d3 = scan(20_002).await.unwrap();
    assert_eq!(d3.notified.len(), 1, "signed, not sent yet: still reminded");
    assert_eq!(
        d3.notified[0].key,
        format!("listing_ready:{profile_id}:forsale:l1-tx:20002")
    );
    set_finalize("broadcasted");
    assert!(
        scan(20_003).await.unwrap().notified.is_empty(),
        "gone once the FINALIZE is sent"
    );
    let s: tauri::State<crate::AppState> = app.state();
    let conn = s.db.lock().unwrap();
    let raw = db::queries::get_settings(&conn).unwrap()["deadline_notify_state"].clone();
    assert!(!raw.contains("listing_ready:"), "{raw}");
}

/// The listing reminder follows the setting like every other kind: off, a
/// ready listing notifies nothing and the scan says it is disabled.
#[tokio::test]
async fn listing_ready_respects_disabled_setting() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        seed_listing(&conn, &id, "l1", "forsale", "ready_to_finalize");
    }
    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications_on_day(app.handle().clone(), app.state(), 20_000)
        .await
        .unwrap();
    assert!(!outcome.enabled);
    assert!(outcome.notified.is_empty());
}

// --- Shakedex cancel ready to finalize (R28) ---

/// R28, R14's pattern: a mined cancel notifies once its transfer lockup is
/// over (0 blocks left at the last sync), not while blocks are left, and
/// not again the next day; once its FINALIZE is sent the episode ends and
/// its key leaves the persisted state.
#[tokio::test]
async fn cancel_finalize_ready_after_the_lockup() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_listing(&conn, &id, "l1", "homeward", "cancel_awaiting_finalize");
        conn.execute(
            "UPDATE shakedex_listings
             SET cancel_txid = 'c1-tx', cancel_vout = 0, cancel_blocks_remaining = 3
             WHERE id = 'l1'",
            [],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let scan = |day| scan_deadline_notifications_on_day(app.handle().clone(), app.state(), day);
    let with = |sql: &str| {
        let s: tauri::State<crate::AppState> = app.state();
        let conn = s.db.lock().unwrap();
        conn.execute(sql, []).unwrap();
    };
    assert!(
        scan(20_000).await.unwrap().notified.is_empty(),
        "3 blocks left"
    );
    with("UPDATE shakedex_listings SET cancel_blocks_remaining = 0 WHERE id = 'l1'");
    let d = scan(20_000).await.unwrap();
    assert_eq!(d.notified.len(), 1, "{:?}", d.notified);
    assert_eq!(
        d.notified[0].key,
        format!("cancel_finalize:{profile_id}:homeward:c1-tx")
    );
    assert_eq!(d.notified[0].title, "Ready to finalize");
    assert!(
        d.notified[0].body.contains("bring the name home"),
        "{}",
        d.notified[0].body
    );
    assert!(
        scan(20_001).await.unwrap().notified.is_empty(),
        "once, not daily"
    );
    {
        let s: tauri::State<crate::AppState> = app.state();
        let conn = s.db.lock().unwrap();
        db::queries::insert_tx_draft(
            &conn,
            "cfin",
            &profile_id,
            "shakedex_cancel_finalize",
            "00",
            "{}",
            "{}",
        )
        .unwrap();
    }
    with(
        "UPDATE shakedex_listings SET state = 'cancel_finalizing', \
         cancel_finalize_draft_id = 'cfin' WHERE id = 'l1'",
    );
    with("UPDATE wallet_tx_drafts SET status = 'broadcasted' WHERE id = 'cfin'");
    assert!(scan(20_002).await.unwrap().notified.is_empty(), "sent");
    let s: tauri::State<crate::AppState> = app.state();
    let conn = s.db.lock().unwrap();
    let raw = db::queries::get_settings(&conn).unwrap()["deadline_notify_state"].clone();
    assert!(!raw.contains("cancel_finalize:"), "{raw}");
}

/// R28: an unsent (draft or signed) FINALIZE draft does not end the
/// reminder; a sent one does, and when that draft then fails or is dropped
/// the cancel is ready again and reminds once more; a disabled setting
/// notifies nothing.
#[tokio::test]
async fn cancel_finalize_reminder_follows_its_draft() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "mainnet");
        enable_notifications(&conn, "144", "30");
        seed_listing(&conn, &id, "l1", "homeward", "cancel_finalizing");
        conn.execute(
            "UPDATE shakedex_listings
             SET cancel_txid = 'c1-tx', cancel_vout = 0, cancel_blocks_remaining = 0
             WHERE id = 'l1'",
            [],
        )
        .unwrap();
        db::queries::insert_tx_draft(
            &conn,
            "cfin",
            &id,
            "shakedex_cancel_finalize",
            "00",
            "{}",
            "{}",
        )
        .unwrap();
        conn.execute(
            "UPDATE shakedex_listings SET cancel_finalize_draft_id = 'cfin' WHERE id = 'l1'",
            [],
        )
        .unwrap();
    };
    let app = mock_app_with(state);
    let scan = |day| scan_deadline_notifications_on_day(app.handle().clone(), app.state(), day);
    let with = |sql: &str| {
        let s: tauri::State<crate::AppState> = app.state();
        let conn = s.db.lock().unwrap();
        conn.execute(sql, []).unwrap();
    };
    let draft = |status: &str| {
        with(&format!(
            "UPDATE wallet_tx_drafts SET status = '{status}' WHERE id = 'cfin'"
        ))
    };
    draft("signed");
    assert_eq!(
        scan(20_000).await.unwrap().notified.len(),
        1,
        "unsent signed"
    );
    draft("draft");
    assert!(
        scan(20_001).await.unwrap().notified.is_empty(),
        "still active"
    );
    draft("broadcasted");
    assert!(scan(20_002).await.unwrap().notified.is_empty(), "sent");
    draft("failed");
    assert_eq!(
        scan(20_003).await.unwrap().notified.len(),
        1,
        "failed: again"
    );
    draft("broadcasted");
    assert!(
        scan(20_004).await.unwrap().notified.is_empty(),
        "sent again"
    );
    draft("dropped");
    assert_eq!(
        scan(20_005).await.unwrap().notified.len(),
        1,
        "dropped: again"
    );
    draft("broadcasted");
    assert!(scan(20_006).await.unwrap().notified.is_empty());
    with("UPDATE settings SET value = 'false' WHERE key = 'deadline_notify_enabled'");
    draft("failed");
    let d = scan(20_007).await.unwrap();
    assert!(!d.enabled && d.notified.is_empty(), "disabled");
}
