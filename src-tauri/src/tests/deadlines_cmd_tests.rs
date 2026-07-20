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

use crate::commands::deadlines::scan_deadline_notifications;
use crate::db;
use crate::tests::names_cmd_tests::{create_full_test_state, insert_valid_profile, mock_app_with};
use tauri::Manager;

const BLOCKS_PER_DAY: i64 = 144;
const RENEWAL_WINDOW: i64 = 105_120; // mainnet

fn enable_notifications(conn: &rusqlite::Connection, reveal_lead_blocks: &str, renewal_lead_days: &str) {
    db::queries::set_setting(conn, "deadline_notify_enabled", "true").unwrap();
    db::queries::set_setting(conn, "deadline_notify_reveal_lead_blocks", reveal_lead_blocks).unwrap();
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
        conn, profile_id, name, "aabb", "rs1qaddr", 0, 0, 1000, 2000, &"11".repeat(32),
        &"22".repeat(32),
    )
    .unwrap();
    db::queries::set_reveal_end_height(conn, profile_id, &"22".repeat(32), reveal_end_height).unwrap();
}

fn seed_owned_name_near_renewal(conn: &rusqlite::Connection, profile_id: &str, name: &str, renewal_height: i64) {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
             height, renewal_height)
         VALUES (?1, ?2, 'aa', 'CLOSED', 'deadbeef', 0, 100, ?3)",
        rusqlite::params![profile_id, name, renewal_height],
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
    assert!(outcome.notified.is_empty(), "must not notify while the feature is off by default");
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
    let raw = settings.get("deadline_notify_state").expect("dedup state must be persisted");
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
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
        db::queries::set_bid_reveal_txid(&conn, &id, "alreadyrevealed", "revealtxid").unwrap();
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
            &conn, &id, "legacybid", "aabb", "rs1qaddr", 0, 0, 1000, 2000, &"11".repeat(32),
            &"33".repeat(32),
        )
        .unwrap();
        // Deliberately no set_reveal_end_height call.
    }

    let app = mock_app_with(state);
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
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
    let outcome = scan_deadline_notifications(app.handle().clone(), app.state()).await.unwrap();
    assert_eq!(outcome.notified.len(), 1, "a window closed exactly one reveal-period ago still gets its final alarm");
}
