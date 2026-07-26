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
            let existed: bool = tx.query_row(
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
    let mut stmt = conn.prepare(
        "SELECT id, name, data_json FROM namebase_history WHERE family = 'subdomains'",
    )?;
    let rows: Vec<(i64, Option<String>, String)> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?))
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../namebase-history-2026-07-26.csv");
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: fixture {path} not present");
            return;
        }
        let csv_text = std::fs::read_to_string(path).unwrap();
        let events = parse_history_csv(&csv_text).unwrap();
        assert!(events.len() > 8000, "expected the full export, got {}", events.len());

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
        let mut conn = mem_db();

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
}
