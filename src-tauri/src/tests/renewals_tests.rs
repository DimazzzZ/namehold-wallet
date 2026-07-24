//! Tests for `read_renewals` (Task 3, C3): days-until-expiry computed LIVE from
//! chain data (`tracked_name_states.renewal_height` + network renewal window +
//! current height) instead of the stale CSV-imported `assets` columns.
//!
//! Covered:
//! * chain-sourced rows: renewal height near / far / past → correct days,
//!   `expiringSoon` flag, `source: "chain"`;
//! * CSV fallback: a name with no chain renewal data falls back to the
//!   `assets` columns with `source: "csv-import"`;
//! * height source selection: live node height → `"node"`; persisted
//!   explorer-derived stats (extrapolated by elapsed wall time) →
//!   `"explorer"`; nothing → `"unknown"` (and chain days stay null — never
//!   fabricated).

use crate::commands::names::EXPIRING_SOON_THRESHOLD_DAYS;
use crate::commands::read::compute_renewals;
use crate::db;

const PROFILE: &str = "p1";
/// Mainnet renewal window in blocks (hsd networks.js).
const WINDOW: i64 = 105_120;
/// ~10-minute blocks.
const BLOCKS_PER_DAY: i64 = 144;

fn mem_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Test",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    conn
}

/// Seed an owned tracked name (owner outpoint recorded → counts as owned for
/// `read_names`/renewals) with an optional chain renewal height + raw_json.
fn seed_tracked(
    conn: &rusqlite::Connection,
    name: &str,
    renewal_height: Option<i64>,
    raw_json: Option<&str>,
) {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
             height, renewal_height, raw_json)
         VALUES (?1, ?2, 'aa', 'CLOSED', 'deadbeef', 0, 100, ?3, ?4)",
        rusqlite::params![PROFILE, name, renewal_height, raw_json],
    )
    .unwrap();
}

fn seed_asset(
    conn: &rusqlite::Connection,
    tld: &str,
    expires_at_height: Option<i64>,
    days_until_expire: Option<f64>,
) {
    conn.execute(
        "INSERT INTO assets (tld, status, name_state, expires_at_height, days_until_expire)
         VALUES (?1, 'finalized_owned', 'CLOSED', ?2, ?3)",
        rusqlite::params![tld, expires_at_height, days_until_expire],
    )
    .unwrap();
}

// ============================================================================
// Chain-sourced rows: near / far / past
// ============================================================================

#[test]
fn chain_name_near_expiry_is_expiring_soon() {
    let conn = mem_db();
    seed_tracked(&conn, "nearname", Some(1_000), None);
    // 10 days of blocks left in the renewal window.
    let height = 1_000 + WINDOW - 10 * BLOCKS_PER_DAY;
    let resp = compute_renewals(&conn, PROFILE, Some(height)).unwrap();

    assert_eq!(resp.height_source, "node");
    assert_eq!(resp.current_height, Some(height));
    let row = resp.names.iter().find(|r| r.name == "nearname").unwrap();
    assert_eq!(row.source, "chain");
    assert_eq!(row.renewal_height, Some(1_000));
    assert_eq!(row.expires_at_height, Some(1_000 + WINDOW));
    assert_eq!(row.blocks_until_expire, Some(10 * BLOCKS_PER_DAY));
    let days = row.days_until_expire.unwrap();
    assert!((days - 10.0).abs() < 0.01, "expected ~10 days, got {days}");
    assert!(row.expiring_soon, "10 days left must be expiring soon");
}

#[test]
fn chain_name_far_from_expiry_is_not_expiring_soon() {
    let conn = mem_db();
    seed_tracked(&conn, "farname", Some(50_000), None);
    // Renewed recently: almost the whole window remains.
    let height = 50_000 + 1_000;
    let resp = compute_renewals(&conn, PROFILE, Some(height)).unwrap();

    let row = resp.names.iter().find(|r| r.name == "farname").unwrap();
    assert_eq!(row.source, "chain");
    let days = row.days_until_expire.unwrap();
    assert!(days > EXPIRING_SOON_THRESHOLD_DAYS, "got {days}");
    assert!(!row.expiring_soon);
}

#[test]
fn chain_name_past_expiry_is_flagged() {
    let conn = mem_db();
    seed_tracked(&conn, "pastname", Some(1_000), None);
    // Current height is beyond the renewal window end.
    let height = 1_000 + WINDOW + 500;
    let resp = compute_renewals(&conn, PROFILE, Some(height)).unwrap();

    let row = resp.names.iter().find(|r| r.name == "pastname").unwrap();
    assert_eq!(row.source, "chain");
    assert_eq!(row.blocks_until_expire, Some(-500));
    assert!(row.days_until_expire.unwrap() < 0.0);
    assert!(row.expiring_soon, "past expiry must be flagged");
}

// ============================================================================
// CSV fallback
// ============================================================================

#[test]
fn csv_only_name_falls_back_with_source_marker() {
    let conn = mem_db();
    seed_asset(&conn, "csvonly", Some(500_000), Some(42.5));
    let resp = compute_renewals(&conn, PROFILE, Some(200_000)).unwrap();

    let row = resp.names.iter().find(|r| r.name == "csvonly").unwrap();
    assert_eq!(row.source, "csv-import");
    assert_eq!(row.days_until_expire, Some(42.5));
    assert_eq!(row.expires_at_height, Some(500_000));
    assert_eq!(row.renewal_height, None);
}

#[test]
fn csv_row_stale_reassurance_is_overridden_when_chain_has_passed_it() {
    // Fix for review Finding 1: a csv-import row's stored `days_until_expire`
    // is an unverified point-in-time snapshot and can OVERSTATE remaining
    // time (green "200d" for an already-expired name — the dangerous
    // direction). Once a known current height is clearly past the stored
    // `expires_at_height`, override to expired styling instead of replaying
    // the stale number.
    let conn = mem_db();
    // Stale snapshot claims 200 days left...
    seed_asset(&conn, "stalecsv", Some(100_000), Some(200.0));
    // ...but the known current height is already past that expiry height.
    let resp = compute_renewals(&conn, PROFILE, Some(150_000)).unwrap();

    let row = resp.names.iter().find(|r| r.name == "stalecsv").unwrap();
    assert_eq!(row.source, "csv-import");
    assert!(
        row.expiring_soon,
        "must be flagged once the chain has passed expiry"
    );
    let days = row
        .days_until_expire
        .expect("days must be recomputed, not left null");
    assert!(
        days < 0.0,
        "expected negative (already-past) days, got {days}"
    );
    assert_eq!(row.blocks_until_expire, Some(100_000 - 150_000));
}

#[test]
fn csv_row_not_yet_expired_keeps_stored_days_untouched() {
    // The override only fires once the chain has clearly passed the stored
    // expiry height — short of that we don't trust/recompute the imported
    // number (semantics unverified), we just don't let it lie in the unsafe
    // direction.
    let conn = mem_db();
    seed_asset(&conn, "notyetcsv", Some(500_000), Some(42.5));
    let resp = compute_renewals(&conn, PROFILE, Some(200_000)).unwrap();

    let row = resp.names.iter().find(|r| r.name == "notyetcsv").unwrap();
    assert_eq!(row.source, "csv-import");
    assert_eq!(row.days_until_expire, Some(42.5));
    assert!(!row.expiring_soon);
}

#[test]
fn tracked_name_without_renewal_height_uses_csv_fallback() {
    let conn = mem_db();
    // Tracked (owned) but sync never recorded a renewal height.
    seed_tracked(&conn, "halfsynced", None, None);
    seed_asset(&conn, "halfsynced", Some(400_000), Some(15.0));
    let resp = compute_renewals(&conn, PROFILE, Some(200_000)).unwrap();

    let rows: Vec<_> = resp
        .names
        .iter()
        .filter(|r| r.name == "halfsynced")
        .collect();
    assert_eq!(rows.len(), 1, "one row per name, not duplicated");
    assert_eq!(rows[0].source, "csv-import");
    assert_eq!(rows[0].days_until_expire, Some(15.0));
    assert!(rows[0].expiring_soon);
}

#[test]
fn chain_data_wins_over_csv_for_the_same_name() {
    let conn = mem_db();
    seed_tracked(&conn, "bothname", Some(50_000), None);
    // Stale CSV says 5 days — chain says plenty. Chain must win.
    seed_asset(&conn, "bothname", Some(100_000), Some(5.0));
    let resp = compute_renewals(&conn, PROFILE, Some(50_000 + 1_000)).unwrap();

    let rows: Vec<_> = resp.names.iter().filter(|r| r.name == "bothname").collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, "chain");
    assert!(!rows[0].expiring_soon);
}

// ============================================================================
// Height source selection
// ============================================================================

#[test]
fn live_node_height_reports_node_source() {
    let conn = mem_db();
    seed_tracked(&conn, "somename", Some(1_000), None);
    let resp = compute_renewals(&conn, PROFILE, Some(123_456)).unwrap();
    assert_eq!(resp.height_source, "node");
    assert_eq!(resp.current_height, Some(123_456));
}

#[test]
fn explorer_stats_derive_height_when_node_not_synced() {
    let conn = mem_db();
    // Explorer-shaped raw_json (HsdName serialization): stats say that at fetch
    // time the chain was at renewalPeriodEnd - blocksUntilExpire = 260_000.
    let raw = r#"{"name":"expname","state":"CLOSED","stats":{"renewalPeriodEnd":300000,"blocksUntilExpire":40000}}"#;
    seed_tracked(&conn, "expname", Some(194_880), Some(raw));
    let resp = compute_renewals(&conn, PROFILE, None).unwrap();

    assert_eq!(resp.height_source, "explorer");
    // updated_at is "now" → no meaningful extrapolation.
    let h = resp.current_height.unwrap();
    assert!((260_000..260_002).contains(&h), "got {h}");
    // Days must be computed from the derived height, not left null.
    let row = resp.names.iter().find(|r| r.name == "expname").unwrap();
    assert_eq!(row.source, "chain");
    assert!(row.days_until_expire.is_some());
}

#[test]
fn node_shaped_raw_json_also_derives_height() {
    let conn = mem_db();
    // Node-shaped raw_json ({"info": {...}}) from `upsert_name_state`.
    let raw = r#"{"info":{"name":"nodename","state":"CLOSED","stats":{"renewalPeriodEnd":150000,"blocksUntilExpire":30000}}}"#;
    seed_tracked(&conn, "nodename", Some(44_880), Some(raw));
    let resp = compute_renewals(&conn, PROFILE, None).unwrap();
    assert_eq!(resp.height_source, "explorer");
    let h = resp.current_height.unwrap();
    assert!((120_000..120_002).contains(&h), "got {h}");
}

#[test]
fn explorer_height_extrapolates_elapsed_wall_time() {
    let conn = mem_db();
    let raw = r#"{"name":"oldname","state":"CLOSED","stats":{"renewalPeriodEnd":300000,"blocksUntilExpire":40000}}"#;
    seed_tracked(&conn, "oldname", Some(194_880), Some(raw));
    // Snapshot taken 1 day ago → ~144 blocks must be added, otherwise a stale
    // snapshot silently INFLATES days-until-expiry (dangerous direction).
    conn.execute(
        "UPDATE tracked_name_states SET updated_at = datetime('now', '-1 day')",
        [],
    )
    .unwrap();
    let resp = compute_renewals(&conn, PROFILE, None).unwrap();
    assert_eq!(resp.height_source, "explorer");
    let h = resp.current_height.unwrap();
    assert!(
        (260_000 + 143..=260_000 + 145).contains(&h),
        "expected ~+144 blocks extrapolation, got {h}"
    );
}

#[test]
fn last_synced_height_used_when_no_stats_snapshot() {
    let conn = mem_db();
    seed_tracked(&conn, "plainname", Some(1_000), None);
    db::queries::update_profile_sync(&conn, PROFILE, 90_000).unwrap();
    let resp = compute_renewals(&conn, PROFILE, None).unwrap();
    assert_eq!(resp.height_source, "explorer");
    let h = resp.current_height.unwrap();
    assert!((90_000..90_002).contains(&h), "got {h}");
}

#[test]
fn no_height_at_all_is_honest_unknown() {
    let conn = mem_db();
    seed_tracked(&conn, "noheight", Some(1_000), None);
    let resp = compute_renewals(&conn, PROFILE, None).unwrap();

    assert_eq!(resp.height_source, "unknown");
    assert_eq!(resp.current_height, None);
    let row = resp.names.iter().find(|r| r.name == "noheight").unwrap();
    // Chain renewal height is known, but days can NOT be computed — must be
    // null rather than fabricated.
    assert_eq!(row.source, "chain");
    assert_eq!(row.renewal_height, Some(1_000));
    assert_eq!(row.expires_at_height, Some(1_000 + WINDOW));
    assert_eq!(row.days_until_expire, None);
    assert!(!row.expiring_soon);
}

// ============================================================================
// Response shape
// ============================================================================

#[test]
fn response_serializes_camel_case_and_reports_threshold() {
    let conn = mem_db();
    seed_tracked(&conn, "shapename", Some(1_000), None);
    let resp = compute_renewals(&conn, PROFILE, Some(2_000)).unwrap();
    assert_eq!(
        resp.expiring_soon_threshold_days,
        EXPIRING_SOON_THRESHOLD_DAYS
    );

    let v = serde_json::to_value(&resp).unwrap();
    assert!(v.get("heightSource").is_some());
    assert!(v.get("currentHeight").is_some());
    assert!(v.get("expiringSoonThresholdDays").is_some());
    let row = &v.get("names").unwrap().as_array().unwrap()[0];
    assert!(row.get("daysUntilExpire").is_some());
    assert!(row.get("expiresAtHeight").is_some());
    assert!(row.get("renewalHeight").is_some());
    assert!(row.get("expiringSoon").is_some());
    assert!(row.get("source").is_some());
}

#[test]
fn rows_sorted_by_days_ascending_nulls_last() {
    let conn = mem_db();
    seed_tracked(&conn, "aaa-far", Some(50_000), None);
    seed_tracked(&conn, "bbb-near", Some(1_000), None);
    // No renewal height and no csv → days null → sorted last.
    seed_tracked(&conn, "ccc-nodata", None, None);
    let height = 1_000 + WINDOW - 5 * BLOCKS_PER_DAY;
    let resp = compute_renewals(&conn, PROFILE, Some(height)).unwrap();
    let order: Vec<&str> = resp.names.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(order, vec!["bbb-near", "aaa-far", "ccc-nodata"]);
}
