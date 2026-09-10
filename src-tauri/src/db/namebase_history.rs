//! DB access for imported Namebase account-history events.
//!
//! Kept in its own module (not `queries.rs`) because it's a self-contained,
//! append/upsert-by-id store that is deliberately separate from on-chain data.

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;
use crate::namebase::history::NamebaseEvent;

/// Result of an import run.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHistoryResult {
    /// Rows inserted for the first time.
    pub inserted: usize,
    /// Rows that already existed (same Namebase id) and were updated.
    pub updated: usize,
    /// Total events parsed from the source.
    pub total: usize,
}

/// A row read back for the UI. Same wire shape as [`NamebaseEvent`] plus the
/// `imported_at` bookkeeping timestamp.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamebaseHistoryRow {
    pub id: i64,
    pub created_at: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub family: String,
    pub verb: String,
    pub name: Option<String>,
    pub fee_doos: Option<i64>,
    pub bid_doos: Option<i64>,
    pub stake_doos: Option<i64>,
    pub usd_cents: Option<i64>,
    pub hns_doos: Option<i64>,
    pub auction_id: Option<String>,
    pub bid_id: Option<String>,
    pub sale_id: Option<String>,
    pub data_json: String,
    pub imported_at: String,
}

/// Summary aggregates for the import card.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamebaseHistorySummary {
    /// Total events stored.
    pub event_count: i64,
    /// Distinct domain names touched.
    pub name_count: i64,
    /// Sum of all Namebase platform fees (dollarydoos).
    pub total_fee_doos: i64,
    /// Sum of all USD sale proceeds (cents).
    pub total_usd_cents: i64,
    /// Earliest `created_at`, or `None` when empty.
    pub earliest: Option<String>,
    /// Latest `created_at`, or `None` when empty.
    pub latest: Option<String>,
}

/// Upsert a batch of parsed events in one transaction. Returns counts.
/// Uses `INSERT ... ON CONFLICT(id) DO UPDATE` so re-importing the same export
/// (or the live API) refreshes rows in place rather than duplicating.
pub fn upsert_events(
    conn: &mut Connection,
    events: &[NamebaseEvent],
) -> Result<ImportHistoryResult, AppError> {
    let mut result = ImportHistoryResult {
        total: events.len(),
        ..Default::default()
    };
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, fee_doos, bid_doos,
                stake_doos, usd_cents, hns_doos, auction_id, bid_id, sale_id, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
               created_at = excluded.created_at,
               type = excluded.type,
               family = excluded.family,
               verb = excluded.verb,
               name = excluded.name,
               fee_doos = excluded.fee_doos,
               bid_doos = excluded.bid_doos,
               stake_doos = excluded.stake_doos,
               usd_cents = excluded.usd_cents,
               hns_doos = excluded.hns_doos,
               auction_id = excluded.auction_id,
               bid_id = excluded.bid_id,
               sale_id = excluded.sale_id,
               data_json = excluded.data_json,
               imported_at = datetime('now')",
        )?;
        for e in events {
            // `changes()` is 1 for an insert and 1 for an ON CONFLICT update in
            // SQLite, so we can't distinguish via changes alone. Probe existence
            // first (cheap, indexed PK lookup).
            let existed: bool = tx
                .query_row(
                    "SELECT 1 FROM namebase_history WHERE id = ?1",
                    params![e.id],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            stmt.execute(params![
                e.id,
                e.created_at,
                e.kind,
                e.family,
                e.verb,
                e.name,
                e.fee_doos,
                e.bid_doos,
                e.stake_doos,
                e.usd_cents,
                e.hns_doos,
                e.auction_id,
                e.bid_id,
                e.sale_id,
                e.data_json,
            ])?;

            if existed {
                result.updated += 1;
            } else {
                result.inserted += 1;
            }
        }
    }
    tx.commit()?;
    Ok(result)
}

/// List history rows, optionally filtered by name (exact, normalized), family,
/// and a free-text search over name. Newest first.
pub fn list_history(
    conn: &Connection,
    name: Option<&str>,
    family: Option<&str>,
    search: Option<&str>,
) -> Result<Vec<NamebaseHistoryRow>, AppError> {
    let mut sql = String::from(
        "SELECT id, created_at, type, family, verb, name, fee_doos, bid_doos,
                stake_doos, usd_cents, hns_doos, auction_id, bid_id, sale_id,
                data_json, imported_at
         FROM namebase_history WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(n) = name.filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND name = ?");
        args.push(Box::new(n.trim().to_lowercase()));
    }
    if let Some(f) = family.filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND family = ?");
        args.push(Box::new(f.trim().to_string()));
    }
    if let Some(q) = search.filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND name LIKE ?");
        args.push(Box::new(format!("%{}%", q.trim().to_lowercase())));
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |r| {
        Ok(NamebaseHistoryRow {
            id: r.get(0)?,
            created_at: r.get(1)?,
            kind: r.get(2)?,
            family: r.get(3)?,
            verb: r.get(4)?,
            name: r.get(5)?,
            fee_doos: r.get(6)?,
            bid_doos: r.get(7)?,
            stake_doos: r.get(8)?,
            usd_cents: r.get(9)?,
            hns_doos: r.get(10)?,
            auction_id: r.get(11)?,
            bid_id: r.get(12)?,
            sale_id: r.get(13)?,
            data_json: r.get(14)?,
            imported_at: r.get(15)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Aggregate totals for the summary card.
pub fn summary(conn: &Connection) -> Result<NamebaseHistorySummary, AppError> {
    conn.query_row(
        "SELECT
            COUNT(*),
            COUNT(DISTINCT name),
            COALESCE(SUM(fee_doos), 0),
            COALESCE(SUM(usd_cents), 0),
            MIN(created_at),
            MAX(created_at)
         FROM namebase_history",
        [],
        |r| {
            Ok(NamebaseHistorySummary {
                event_count: r.get(0)?,
                name_count: r.get(1)?,
                total_fee_doos: r.get(2)?,
                total_usd_cents: r.get(3)?,
                earliest: r.get(4)?,
                latest: r.get(5)?,
            })
        },
    )
    .map_err(AppError::from)
}

/// Delete all imported history. Returns the number of rows removed.
pub fn clear(conn: &Connection) -> Result<usize, AppError> {
    let n = conn.execute("DELETE FROM namebase_history", [])?;
    Ok(n)
}

/// Parse a stored `data_json` string back into a JSON value (helper for callers
/// that need the raw payload without re-querying).
#[allow(dead_code)]
pub fn parse_data(row: &NamebaseHistoryRow) -> Value {
    serde_json::from_str(&row.data_json).unwrap_or(Value::Null)
}

/// One-shot fix for subdomain rows imported before the parser composed
/// `{subdomain}.{domain}`: re-derives `name` from each row's stored
/// `data_json` and updates it in place. Non-subdomain rows are untouched.
/// Returns the number of rows updated.
///
/// Idempotent: on a fully-fixed DB it walks the rows but writes zero updates.
/// The `data_json` payload is the source of truth (never dropped by the parser),
/// so this backfill never needs to hit the network.
pub fn backfill_subdomain_names(conn: &Connection) -> Result<usize, AppError> {
    // Load all subdomain rows first (small subset — ~100 in the sample fixture).
    let mut stmt = conn
        .prepare("SELECT id, name, data_json FROM namebase_history WHERE family = 'subdomains'")?;
    let rows: Vec<(i64, Option<String>, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut updates: Vec<(i64, String)> = Vec::new();
    for (id, current, data_json) in rows {
        let Ok(v) = serde_json::from_str::<Value>(&data_json) else {
            continue;
        };
        // Compose only when both parts are present; otherwise leave the row alone
        // (stake-domain and friends carry only `domain`, which is already correct).
        let dom = match v.get("domain").and_then(|x| x.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => continue,
        };
        let sub = match v.get("subdomain").and_then(|x| x.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => continue,
        };
        let composed = format!("{}.{}", sub, dom)
            .trim()
            .trim_start_matches('.')
            .to_lowercase();
        if current.as_deref() != Some(composed.as_str()) {
            updates.push((id, composed));
        }
    }

    if updates.is_empty() {
        return Ok(0);
    }

    let tx = conn.unchecked_transaction()?;
    {
        let mut upd = tx.prepare("UPDATE namebase_history SET name = ?1 WHERE id = ?2")?;
        for (id, name) in &updates {
            upd.execute(params![name, id])?;
        }
    }
    tx.commit()?;
    Ok(updates.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namebase::history::parse_history_csv;

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        // migrations::run takes &Connection; ensure table exists.
        let _ = &mut conn;
        conn
    }

    fn sample_csv() -> String {
        format!(
            "\"preamble\"\n\"\"\n\"more\"\n\nid,created_at,type,data\n{}{}{}",
            "188679284,2026-01-17T12:37:54.492Z,auctions:place-bid:4,\"{\"\"domainName\"\":\"\"diver\"\",\"\"auctionId\"\":\"\"a1\"\",\"\"bidAmountString\"\":\"\"123000000\"\",\"\"stakeAmountString\"\":\"\"2469000000\"\",\"\"prepaidFeeString\"\":\"\"1000283\"\"}\"\n",
            "188784786,2026-01-27T06:25:25.161Z,subdomains:confirm-transfer:2,\"{\"\"domain\"\":\"\"shot\"\",\"\"saleId\"\":\"\"s1\"\",\"\"deliveredAmountUsd\"\":{\"\"amountString\"\":\"\"2900\"\",\"\"asset\"\":\"\"USD\"\"},\"\"deliveredAmountHns\"\":{\"\"amountString\"\":\"\"4832721250\"\",\"\"asset\"\":\"\"HNS\"\"}}\"\n",
            "188680273,2026-01-17T18:32:34.289Z,auctions:charge-fee:0,\"{\"\"domainName\"\":\"\"diver\"\",\"\"feeChargedString\"\":\"\"32600\"\"}\"\n",
        )
    }

    #[test]
    fn upsert_is_idempotent_and_counts_correctly() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        assert_eq!(events.len(), 3);

        let r1 = upsert_events(&mut conn, &events).unwrap();
        assert_eq!(r1.inserted, 3);
        assert_eq!(r1.updated, 0);
        assert_eq!(r1.total, 3);

        // Re-import: all rows already exist → updated, no dupes.
        let r2 = upsert_events(&mut conn, &events).unwrap();
        assert_eq!(r2.inserted, 0);
        assert_eq!(r2.updated, 3);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM namebase_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn summary_aggregates_fees_and_usd() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        upsert_events(&mut conn, &events).unwrap();

        let s = summary(&conn).unwrap();
        assert_eq!(s.event_count, 3);
        // Distinct names: "diver" (x2) + "shot" = 2.
        assert_eq!(s.name_count, 2);
        // Fees: 1000283 (place-bid prepaid) + 32600 (charge-fee) = 1032883.
        assert_eq!(s.total_fee_doos, 1032883);
        // USD: 2900 cents from the confirm-transfer.
        assert_eq!(s.total_usd_cents, 2900);
        assert_eq!(s.earliest.as_deref(), Some("2026-01-17T12:37:54.492Z"));
        assert_eq!(s.latest.as_deref(), Some("2026-01-27T06:25:25.161Z"));
    }

    #[test]
    fn list_filters_by_name_and_family() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        upsert_events(&mut conn, &events).unwrap();

        let by_name = list_history(&conn, Some("diver"), None, None).unwrap();
        assert_eq!(by_name.len(), 2);

        let by_family = list_history(&conn, None, Some("subdomains"), None).unwrap();
        assert_eq!(by_family.len(), 1);
        assert_eq!(by_family[0].name.as_deref(), Some("shot"));

        let by_search = list_history(&conn, None, None, Some("div")).unwrap();
        assert_eq!(by_search.len(), 2);
    }

    #[test]
    fn clear_removes_all() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        upsert_events(&mut conn, &events).unwrap();
        let removed = clear(&conn).unwrap();
        assert_eq!(removed, 3);
        assert_eq!(summary(&conn).unwrap().event_count, 0);
    }

    /// Import the real export fixture from the repo root when it's present
    /// (developer machines have it; CI does not — it's git-ignored as user
    /// data). Asserts the whole file round-trips and re-import is idempotent.
    #[test]
    fn imports_real_fixture_when_present() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../namebase-history-2026-07-26.csv"
        );
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: fixture {path} not present");
            return;
        }
        let csv_text = std::fs::read_to_string(path).unwrap();
        let events = parse_history_csv(&csv_text).unwrap();
        assert!(
            events.len() > 8000,
            "expected the full export, got {}",
            events.len()
        );

        let mut conn = mem_db();
        let r1 = upsert_events(&mut conn, &events).unwrap();
        assert_eq!(r1.inserted, events.len());
        assert_eq!(r1.updated, 0);

        // Idempotent re-import: no new rows.
        let r2 = upsert_events(&mut conn, &events).unwrap();
        assert_eq!(r2.inserted, 0);
        assert_eq!(r2.updated, events.len());

        let s = summary(&conn).unwrap();
        assert_eq!(s.event_count as usize, events.len());
        assert!(s.total_fee_doos > 0, "expected some fees");
        assert!(s.name_count > 0, "expected some names");
    }

    #[test]
    fn backfill_updates_subdomain_names_and_is_idempotent() {
        let conn = mem_db();

        // Seed a confirm-transfer row with the OLD (buggy) name = "shot"
        // but the full data_json containing both domain + subdomain.
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                188784786_i64,
                "2026-01-27T06:25:25.161Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "shot", // <-- buggy: should be "moon.shot"
                r#"{"domain":"shot","subdomain":"moon","saleId":"s1"}"#,
            ],
        )
        .unwrap();

        // Seed a stake-domain row (no subdomain field) — should NOT be changed.
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                127574416_i64,
                "2023-08-18T15:45:29.209Z",
                "subdomains:stake-domain:0",
                "subdomains",
                "stake-domain",
                "ecology",
                r#"{"domain":"ecology","custodian":"uk"}"#,
            ],
        )
        .unwrap();

        // First run: fixes the confirm-transfer row.
        let count = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count, 1);

        let fixed: String = conn
            .query_row(
                "SELECT name FROM namebase_history WHERE id = 188784786",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fixed, "moon.shot");

        // stake-domain row untouched.
        let ecology: String = conn
            .query_row(
                "SELECT name FROM namebase_history WHERE id = 127574416",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ecology, "ecology");

        // Second run: idempotent — nothing left to fix.
        let count2 = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count2, 0);
    }

    // --- Coverage-driven tests ---

    /// `parse_data` returns the parsed JSON value (or Null for invalid JSON).
    #[test]
    fn parse_data_returns_value_or_null() {
        let row = NamebaseHistoryRow {
            id: 1,
            created_at: "2026-01-01T00:00:00Z".into(),
            kind: "test:type:0".into(),
            family: "test".into(),
            verb: "type".into(),
            name: Some("example".into()),
            fee_doos: None,
            bid_doos: None,
            stake_doos: None,
            usd_cents: None,
            hns_doos: None,
            auction_id: None,
            bid_id: None,
            sale_id: None,
            data_json: r#"{"key":"value"}"#.into(),
            imported_at: "2026-01-01T00:00:00Z".into(),
        };
        let v = parse_data(&row);
        assert_eq!(v["key"], "value");

        // Invalid JSON returns Null.
        let bad_row = NamebaseHistoryRow {
            data_json: "not json {{{".into(),
            ..row
        };
        assert_eq!(parse_data(&bad_row), Value::Null);
    }

    /// `backfill_subdomain_names` skips rows with invalid JSON in data_json.
    #[test]
    fn backfill_skips_rows_with_invalid_json() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                1_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "broken",
                "not valid json {{{",
            ],
        )
        .unwrap();

        // Should not panic or error — just skips the row.
        let count = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count, 0);
    }

    /// `backfill_subdomain_names` skips rows where domain is missing or empty.
    #[test]
    fn backfill_skips_rows_without_domain() {
        let conn = mem_db();
        // Row with valid JSON but no "domain" key at all.
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                2_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "nodomain",
                r#"{"subdomain":"sub","saleId":"s1"}"#,
            ],
        )
        .unwrap();
        // Row with domain = "" (empty string).
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                3_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "emptydomain",
                r#"{"domain":"  ","subdomain":"sub"}"#,
            ],
        )
        .unwrap();

        let count = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count, 0);
    }

    // --- Error-propagation tests: exercise the `?` branches by using a bare
    // connection that lacks the `namebase_history` table. ---

    #[test]
    fn upsert_events_propagates_db_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        let events = parse_history_csv(&sample_csv()).unwrap();
        let err = upsert_events(&mut conn, &events);
        assert!(err.is_err(), "expected error from missing table");
    }

    #[test]
    fn list_history_propagates_db_error() {
        let conn = Connection::open_in_memory().unwrap();
        let err = list_history(&conn, None, None, None);
        assert!(err.is_err(), "expected error from missing table");
    }

    #[test]
    fn summary_propagates_db_error() {
        let conn = Connection::open_in_memory().unwrap();
        let err = summary(&conn);
        assert!(err.is_err(), "expected error from missing table");
    }

    #[test]
    fn clear_propagates_db_error() {
        let conn = Connection::open_in_memory().unwrap();
        let err = clear(&conn);
        assert!(err.is_err(), "expected error from missing table");
    }

    #[test]
    fn backfill_subdomain_names_propagates_db_error() {
        let conn = Connection::open_in_memory().unwrap();
        let err = backfill_subdomain_names(&conn);
        assert!(err.is_err(), "expected error from missing table");
    }

    /// Synthetic large-event import test that exercises the same code paths as
    /// `imports_real_fixture_when_present` but without requiring the fixture file.
    /// This covers the branches in `upsert_events` that count inserted vs updated rows.
    #[test]
    fn upsert_large_batch_exercises_insert_and_update_branches() {
        let mut conn = mem_db();

        // Build synthetic events directly (bypassing CSV escaping ceremony) so
        // we exercise `upsert_events`'s insert-then-update branches across many
        // rows — the same code paths the fixture-guarded test would cover.
        let mut events: Vec<NamebaseEvent> = Vec::new();
        for i in 0..150_i64 {
            let id = 1_000_000 + i;
            let domain_name = format!("domain{}", i % 10);
            let bid = 100_000_000 + i * 1_000_000;
            let fee = 10_000 + i;
            let data_json = format!(
                r#"{{"domainName":"{}","auctionId":"a{}","bidAmountString":"{}","prepaidFeeString":"{}"}}"#,
                domain_name, i, bid, fee
            );
            events.push(NamebaseEvent {
                id,
                created_at: format!("2026-01-{:02}T12:00:00Z", (i % 28) + 1),
                kind: "auctions:place-bid:4".into(),
                family: "auctions".into(),
                verb: "place-bid".into(),
                name: Some(domain_name),
                fee_doos: Some(fee),
                bid_doos: Some(bid),
                stake_doos: None,
                usd_cents: None,
                hns_doos: None,
                auction_id: Some(format!("a{i}")),
                bid_id: None,
                sale_id: None,
                data_json,
            });
        }

        // First import: all rows are inserted.
        let r1 = upsert_events(&mut conn, &events).unwrap();
        assert_eq!(r1.inserted, events.len());
        assert_eq!(r1.updated, 0);
        assert_eq!(r1.total, events.len());

        // Verify the rows are in the DB.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM namebase_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count as usize, events.len());

        // Second import: all rows already exist, so they're updated.
        let r2 = upsert_events(&mut conn, &events).unwrap();
        assert_eq!(r2.inserted, 0);
        assert_eq!(r2.updated, events.len());
        assert_eq!(r2.total, events.len());

        // Verify no duplicates were created.
        let count2: i64 = conn
            .query_row("SELECT COUNT(*) FROM namebase_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count2, count, "row count should not change on re-import");

        // Verify the summary aggregates correctly across all rows.
        let s = summary(&conn).unwrap();
        assert_eq!(s.event_count as usize, events.len());
        assert!(s.total_fee_doos > 0, "expected some fees from bids");
        assert!(s.name_count > 0, "expected distinct names");
    }

    /// `list_history` with empty/whitespace name/family/search filters
    /// exercises the `filter(|s| !s.trim().is_empty())` branches.
    #[test]
    fn list_history_empty_and_whitespace_filters_are_ignored() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        upsert_events(&mut conn, &events).unwrap();

        // Empty string filters should be treated as None (no filtering).
        let all = list_history(&conn, Some(""), None, None).unwrap();
        assert_eq!(all.len(), 3, "empty name filter should not restrict");

        let all2 = list_history(&conn, None, Some("  "), None).unwrap();
        assert_eq!(
            all2.len(),
            3,
            "whitespace family filter should not restrict"
        );

        let all3 = list_history(&conn, None, None, Some("  ")).unwrap();
        assert_eq!(
            all3.len(),
            3,
            "whitespace search filter should not restrict"
        );

        // All three empty at once.
        let all4 = list_history(&conn, Some(""), Some("  "), Some("")).unwrap();
        assert_eq!(all4.len(), 3, "all empty filters should not restrict");
    }

    /// `summary` on an empty table returns zeros and None for earliest/latest.
    #[test]
    fn summary_on_empty_table_returns_zeros_and_none() {
        let conn = mem_db();
        let s = summary(&conn).unwrap();
        assert_eq!(s.event_count, 0);
        assert_eq!(s.name_count, 0);
        assert_eq!(s.total_fee_doos, 0);
        assert_eq!(s.total_usd_cents, 0);
        assert!(s.earliest.is_none());
        assert!(s.latest.is_none());
    }

    /// `backfill_subdomain_names` skips rows where subdomain is missing or empty.
    #[test]
    fn backfill_skips_rows_without_subdomain() {
        let conn = mem_db();
        // Row with valid JSON, domain present, but no "subdomain" key.
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                10_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "onlydomain",
                r#"{"domain":"onlydomain","saleId":"s1"}"#,
            ],
        )
        .unwrap();
        // Row with domain present but subdomain = "" (empty).
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                11_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "emptysub",
                r#"{"domain":"emptysub","subdomain":"  "}"#,
            ],
        )
        .unwrap();

        let count = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count, 0);
    }

    // --- Row-getter error-arm tests -------------------------------------
    //
    // Each read path (`list_history`, `summary`, `backfill_subdomain_names`)
    // maps a row via closures that use `r.get::<T>(i)?`. The `?` on each
    // getter is a distinct region whose *error* arm only fires when the
    // stored value has an incompatible type. SQLite keeps a BLOB as a BLOB
    // even in a TEXT-affinity column, and `r.get::<String>()` on a BLOB
    // returns `InvalidColumnType` — the lever we use here to drive those
    // otherwise-unreachable error branches.

    /// Insert a row whose `created_at` is a BLOB (not TEXT), so any
    /// `r.get::<String>(created_at_col)` call fails mid-map.
    fn insert_row_with_blob_created_at(conn: &Connection, id: i64) {
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                vec![0xDE_u8, 0xAD, 0xBE, 0xEF], // BLOB in a TEXT column
                "auctions:place-bid:4",
                "auctions",
                "place-bid",
                "blobrow",
                r#"{"domainName":"blobrow"}"#,
            ],
        )
        .unwrap();
    }

    /// `list_history`'s row-mapping closure propagates a getter error when a
    /// column holds an incompatible type. Exercises the `r.get(..)?` error
    /// arm, the `row?` propagation, and the `Err` return of the whole fn.
    #[test]
    fn list_history_propagates_row_getter_error() {
        let conn = mem_db();
        insert_row_with_blob_created_at(&conn, 900_001);
        let err = list_history(&conn, None, None, None);
        assert!(err.is_err(), "expected a row-getter type error to surface");
    }

    /// `summary`'s `MIN(created_at)` / `MAX(created_at)` return a BLOB when
    /// all rows store BLOB timestamps, so `r.get::<Option<String>>(4)` in the
    /// summary closure errors — driving that closure's error arm.
    #[test]
    fn summary_propagates_row_getter_error() {
        let conn = mem_db();
        insert_row_with_blob_created_at(&conn, 900_002);
        let err = summary(&conn);
        assert!(
            err.is_err(),
            "expected MIN/MAX blob timestamp to fail String conversion"
        );
    }

    /// `backfill_subdomain_names` reads `name` as `Option<String>`; a BLOB
    /// there makes `r.get::<_, Option<String>>(1)?` error inside the
    /// `query_map` closure, propagated by `.collect::<Result<..>>()?`.
    #[test]
    fn backfill_propagates_row_getter_error() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                900_003_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                vec![0x01_u8, 0x02, 0x03], // BLOB in the TEXT `name` column
                r#"{"domain":"shot","subdomain":"moon"}"#,
            ],
        )
        .unwrap();
        let err = backfill_subdomain_names(&conn);
        assert!(
            err.is_err(),
            "expected blob `name` to fail String conversion"
        );
    }

    // --- backfill: already-correct-name branch --------------------------

    /// A subdomain row whose `name` ALREADY equals the composed
    /// `{sub}.{dom}` must be skipped (the `if current != Some(composed)`
    /// FALSE arm), while a genuinely stale sibling row is still fixed. This
    /// exercises both sides of the equality check and keeps `updates`
    /// non-empty so the transaction/commit path also runs.
    #[test]
    fn backfill_skips_already_correct_row_but_fixes_stale_one() {
        let conn = mem_db();

        // Already correct: name == "moon.shot" — should NOT be re-written.
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                700_001_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "moon.shot",
                r#"{"domain":"shot","subdomain":"moon"}"#,
            ],
        )
        .unwrap();
        // Stale: name == "star" but should become "sky.star".
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                700_002_i64,
                "2026-01-02T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "star",
                r#"{"domain":"star","subdomain":"sky"}"#,
            ],
        )
        .unwrap();

        // Only the stale row is updated (the already-correct one is skipped).
        let count = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count, 1);

        let correct: String = conn
            .query_row(
                "SELECT name FROM namebase_history WHERE id = 700001",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(correct, "moon.shot");
        let fixed: String = conn
            .query_row(
                "SELECT name FROM namebase_history WHERE id = 700002",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fixed, "sky.star");
    }

    /// `backfill` composes/normalizes: trims surrounding whitespace, strips a
    /// leading dot, and lowercases. A row whose composed value differs from
    /// the stored `name` only by case/whitespace still counts as an update,
    /// confirming the normalization arm inside the composition.
    #[test]
    fn backfill_normalizes_case_and_whitespace() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                710_001_i64,
                "2026-01-01T00:00:00Z",
                "subdomains:confirm-transfer:2",
                "subdomains",
                "confirm-transfer",
                "OLDNAME",
                r#"{"domain":"Shot","subdomain":"Moon"}"#,
            ],
        )
        .unwrap();
        let count = backfill_subdomain_names(&conn).unwrap();
        assert_eq!(count, 1);
        let fixed: String = conn
            .query_row(
                "SELECT name FROM namebase_history WHERE id = 710001",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Lowercased composition of subdomain + domain.
        assert_eq!(fixed, "moon.shot");
    }

    /// `parse_data` on an empty `data_json` string falls back to `Value::Null`
    /// (empty string is not valid JSON), covering the fallback arm distinctly
    /// from the "garbage text" case already tested.
    #[test]
    fn parse_data_empty_string_is_null() {
        let base = NamebaseHistoryRow {
            id: 1,
            created_at: "2026-01-01T00:00:00Z".into(),
            kind: "test:type:0".into(),
            family: "test".into(),
            verb: "type".into(),
            name: None,
            fee_doos: None,
            bid_doos: None,
            stake_doos: None,
            usd_cents: None,
            hns_doos: None,
            auction_id: None,
            bid_id: None,
            sale_id: None,
            data_json: String::new(),
            imported_at: "2026-01-01T00:00:00Z".into(),
        };
        assert_eq!(parse_data(&base), Value::Null);

        // A JSON literal `null` also round-trips to `Value::Null`.
        let null_row = NamebaseHistoryRow {
            data_json: "null".into(),
            ..base.clone()
        };
        assert_eq!(parse_data(&null_row), Value::Null);

        // A non-object valid JSON value (array) is preserved, exercising the
        // Ok arm with a non-object payload.
        let arr_row = NamebaseHistoryRow {
            data_json: "[1,2,3]".into(),
            ..base
        };
        assert_eq!(parse_data(&arr_row), serde_json::json!([1, 2, 3]));
    }

    /// `list_history` on an empty table returns an empty vec (the `for row in
    /// rows` loop body never runs — covers the zero-iteration path and the
    /// `Ok(out)` return with no filters applied).
    #[test]
    fn list_history_empty_table_returns_empty_vec() {
        let conn = mem_db();
        let rows = list_history(&conn, None, None, None).unwrap();
        assert!(rows.is_empty());
    }

    /// `list_history` orders newest-first by `created_at DESC, id DESC`.
    /// Seeds rows with mixed timestamps (and a tie on timestamp to force the
    /// secondary `id DESC` key) and asserts the exact returned order.
    #[test]
    fn list_history_orders_newest_first_with_id_tiebreak() {
        let conn = mem_db();
        let insert = |id: i64, created_at: &str| {
            conn.execute(
                "INSERT INTO namebase_history
                   (id, created_at, type, family, verb, name, data_json)
                 VALUES (?1, ?2, 'x:y:0', 'fam', 'y', 'n', '{}')",
                params![id, created_at],
            )
            .unwrap();
        };
        insert(1, "2026-01-01T00:00:00Z");
        insert(2, "2026-03-01T00:00:00Z");
        // Tie with id=2 on timestamp — id DESC must place 3 before 2.
        insert(3, "2026-03-01T00:00:00Z");
        insert(4, "2026-02-01T00:00:00Z");

        let rows = list_history(&conn, None, None, None).unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        // Newest first; tie broken by id DESC (3 before 2); oldest last.
        assert_eq!(ids, vec![3, 2, 4, 1]);
    }

    /// `list_history` name filter normalizes to lowercase and trims, so a
    /// mixed-case padded query still matches a stored lowercased name.
    #[test]
    fn list_history_name_filter_normalizes_query() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        upsert_events(&mut conn, &events).unwrap();

        // "  DIVER " → trimmed + lowercased → "diver" (2 rows).
        let rows = list_history(&conn, Some("  DIVER "), None, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.name.as_deref() == Some("diver")));
    }

    /// All three filters supplied simultaneously — exercises the combined
    /// `AND name = ? AND family = ? AND name LIKE ?` SQL-building path.
    #[test]
    fn list_history_all_filters_combined() {
        let mut conn = mem_db();
        let events = parse_history_csv(&sample_csv()).unwrap();
        upsert_events(&mut conn, &events).unwrap();

        let rows = list_history(&conn, Some("diver"), Some("auctions"), Some("div")).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|r| r.family == "auctions" && r.name.as_deref() == Some("diver")));

        // A family that excludes all name-matching rows yields nothing.
        let none = list_history(&conn, Some("diver"), Some("subdomains"), None).unwrap();
        assert!(none.is_empty());
    }

    /// `upsert_events` with an empty slice performs no inserts and commits an
    /// empty transaction (covers the zero-iteration `for e in events` path
    /// and `total == 0`).
    #[test]
    fn upsert_events_empty_slice_is_noop() {
        let mut conn = mem_db();
        let r = upsert_events(&mut conn, &[]).unwrap();
        assert_eq!(r.inserted, 0);
        assert_eq!(r.updated, 0);
        assert_eq!(r.total, 0);
        assert_eq!(summary(&conn).unwrap().event_count, 0);
    }

    /// `upsert_events` for a single event with all optional money/id fields
    /// present, then a re-upsert that flips them to `None` — verifying the
    /// `ON CONFLICT DO UPDATE` overwrites nullable columns and the row is
    /// read back with `Some`/`None` correctly through `list_history`.
    #[test]
    fn upsert_single_event_with_and_without_optional_fields() {
        let mut conn = mem_db();
        let full = NamebaseEvent {
            id: 5_000_001,
            created_at: "2026-05-01T00:00:00Z".into(),
            kind: "auctions:place-bid:4".into(),
            family: "auctions".into(),
            verb: "place-bid".into(),
            name: Some("optional".into()),
            fee_doos: Some(111),
            bid_doos: Some(222),
            stake_doos: Some(333),
            usd_cents: Some(444),
            hns_doos: Some(555),
            auction_id: Some("a1".into()),
            bid_id: Some("b1".into()),
            sale_id: Some("s1".into()),
            data_json: r#"{"domainName":"optional"}"#.into(),
        };
        let r1 = upsert_events(&mut conn, std::slice::from_ref(&full)).unwrap();
        assert_eq!(r1.inserted, 1);
        assert_eq!(r1.updated, 0);

        let rows = list_history(&conn, Some("optional"), None, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].fee_doos, Some(111));
        assert_eq!(rows[0].bid_doos, Some(222));
        assert_eq!(rows[0].stake_doos, Some(333));
        assert_eq!(rows[0].usd_cents, Some(444));
        assert_eq!(rows[0].hns_doos, Some(555));
        assert_eq!(rows[0].auction_id.as_deref(), Some("a1"));
        assert_eq!(rows[0].bid_id.as_deref(), Some("b1"));
        assert_eq!(rows[0].sale_id.as_deref(), Some("s1"));

        // Re-upsert the same id with every optional field cleared to None.
        let empty = NamebaseEvent {
            name: None,
            fee_doos: None,
            bid_doos: None,
            stake_doos: None,
            usd_cents: None,
            hns_doos: None,
            auction_id: None,
            bid_id: None,
            sale_id: None,
            ..full
        };
        let r2 = upsert_events(&mut conn, std::slice::from_ref(&empty)).unwrap();
        assert_eq!(r2.inserted, 0);
        assert_eq!(r2.updated, 1);

        // name is now NULL → not matched by the name filter; fetch via family.
        let rows2 = list_history(&conn, None, Some("auctions"), None).unwrap();
        assert_eq!(rows2.len(), 1);
        assert_eq!(rows2[0].name, None);
        assert_eq!(rows2[0].fee_doos, None);
        assert_eq!(rows2[0].auction_id, None);
    }

    /// `clear` on an empty table returns 0 (the count-deleted path for the
    /// zero-rows case), distinct from the populated `clear_removes_all` test.
    #[test]
    fn clear_on_empty_table_returns_zero() {
        let conn = mem_db();
        assert_eq!(clear(&conn).unwrap(), 0);
    }

    // --- Per-column getter error arms -----------------------------------
    //
    // `list_history`'s row-mapping closure calls `r.get::<T>(i)?` for all 16
    // columns in order. Once a getter errors, the `?` short-circuits and the
    // later getters never run — so a single bad row only ever exercises ONE
    // getter's error arm. To drive every getter's error branch we insert one
    // row per column with a BLOB planted in exactly that column (a BLOB fails
    // both `String` and `i64`/`Option<_>` conversion, and SQLite preserves a
    // BLOB even in a TEXT/INTEGER-affinity column). `id` (column 0) is an
    // INTEGER PRIMARY KEY and cannot hold a BLOB, so it is excluded.

    /// Seed a valid row, then overwrite one column with a raw BLOB so the
    /// corresponding `r.get(col)?` in the read closures errors.
    fn seed_then_blob_column(conn: &Connection, id: i64, column: &str) {
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, fee_doos, bid_doos,
                stake_doos, usd_cents, hns_doos, auction_id, bid_id, sale_id,
                data_json)
             VALUES (?1, '2026-01-01T00:00:00Z', 'auctions:place-bid:4',
                     'auctions', 'place-bid', 'nm', 1, 2, 3, 4, 5, 'a', 'b',
                     's', '{}')",
            params![id],
        )
        .unwrap();
        // X'DEADBEEF' is a 4-byte BLOB literal; affinity leaves it a BLOB.
        conn.execute(
            &format!("UPDATE namebase_history SET {column} = X'DEADBEEF' WHERE id = ?1"),
            params![id],
        )
        .unwrap();
    }

    /// Drive each `list_history` column getter's error arm (columns 2..=15;
    /// `id`/column 0 can't hold a BLOB, and `created_at`/column 1 is covered
    /// by an earlier test). Each iteration uses its own single-row table so a
    /// BLOB in the target column is the first getter to fail.
    #[test]
    fn list_history_each_column_getter_error_arm() {
        let columns = [
            "type",
            "family",
            "verb",
            "name",
            "fee_doos",
            "bid_doos",
            "stake_doos",
            "usd_cents",
            "hns_doos",
            "auction_id",
            "bid_id",
            "sale_id",
            "data_json",
            // `imported_at` (col 15) is read last; its error arm is covered
            // by planting a BLOB there as well.
            "imported_at",
        ];
        for (i, col) in columns.iter().enumerate() {
            let conn = mem_db();
            seed_then_blob_column(&conn, 800_000 + i as i64, col);
            let res = list_history(&conn, None, None, None);
            assert!(
                res.is_err(),
                "expected a getter type error when column `{col}` is a BLOB"
            );
        }
    }

    /// Drive the `backfill_subdomain_names` query_map getters for the
    /// remaining columns it reads: `id` (col 0, i64) and `data_json`
    /// (col 2, String). A BLOB in `data_json` makes `r.get::<_, String>(2)?`
    /// error inside the map closure. (`id` as PK can't be blobbed; the `name`
    /// col-1 error arm is covered by `backfill_propagates_row_getter_error`.)
    #[test]
    fn backfill_data_json_getter_error_arm() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, '2026-01-01T00:00:00Z', 'subdomains:confirm-transfer:2',
                     'subdomains', 'confirm-transfer', 'nm', '{}')",
            params![800_100_i64],
        )
        .unwrap();
        conn.execute(
            "UPDATE namebase_history SET data_json = X'DEADBEEF' WHERE id = ?1",
            params![800_100_i64],
        )
        .unwrap();
        let err = backfill_subdomain_names(&conn);
        assert!(
            err.is_err(),
            "expected blob data_json to fail String conversion"
        );
    }

    /// Drive the `summary` closure's remaining aggregate getters. `COUNT(*)`
    /// and `COUNT(DISTINCT name)` are always integers, and `SUM(...)` is
    /// coerced by COALESCE — but `MIN`/`MAX(created_at)` echo the stored value
    /// type. A single BLOB `created_at` alongside normal rows makes MIN a BLOB
    /// (byte order sorts BLOBs after text is class-ordered) — but to be robust
    /// we make ALL created_at BLOBs so both MIN and MAX are BLOBs, failing the
    /// `earliest`/`latest` String conversions. Already covered for a single
    /// row; here we confirm the multi-row aggregate path too.
    #[test]
    fn summary_getter_error_with_multiple_blob_timestamps() {
        let conn = mem_db();
        for id in [820_001_i64, 820_002] {
            conn.execute(
                "INSERT INTO namebase_history
                   (id, created_at, type, family, verb, name, data_json)
                 VALUES (?1, '2026-01-01T00:00:00Z', 'x:y:0', 'f', 'y', 'n', '{}')",
                params![id],
            )
            .unwrap();
            conn.execute(
                "UPDATE namebase_history SET created_at = X'DEADBEEF' WHERE id = ?1",
                params![id],
            )
            .unwrap();
        }
        let err = summary(&conn);
        assert!(err.is_err(), "expected blob MIN/MAX(created_at) to fail");
    }

    /// Cover the `latest: r.get(5)?` error arm specifically. The summary
    /// closure reads `earliest` (MIN, col 4) before `latest` (MAX, col 5), so
    /// both being BLOBs short-circuits at `earliest`. SQLite's storage-class
    /// sort order places BLOBs *after* TEXT, so a table with one valid TEXT
    /// timestamp and one BLOB timestamp yields MIN = valid TEXT (earliest
    /// succeeds) and MAX = BLOB (latest's String conversion fails) — isolating
    /// the `latest` getter's error branch.
    #[test]
    fn summary_latest_getter_error_arm_isolated() {
        let conn = mem_db();
        // Valid TEXT timestamp row (this is the MIN).
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, '2026-01-01T00:00:00Z', 'x:y:0', 'f', 'y', 'n', '{}')",
            params![821_001_i64],
        )
        .unwrap();
        // BLOB timestamp row (sorts last → becomes the MAX).
        conn.execute(
            "INSERT INTO namebase_history
               (id, created_at, type, family, verb, name, data_json)
             VALUES (?1, '2026-01-01T00:00:00Z', 'x:y:0', 'f', 'y', 'n', '{}')",
            params![821_002_i64],
        )
        .unwrap();
        conn.execute(
            "UPDATE namebase_history SET created_at = X'DEADBEEF' WHERE id = ?1",
            params![821_002_i64],
        )
        .unwrap();
        let err = summary(&conn);
        assert!(
            err.is_err(),
            "expected MAX(created_at) BLOB to fail the `latest` String getter"
        );
    }
}
