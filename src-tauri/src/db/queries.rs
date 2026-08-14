#![allow(dead_code)]

use crate::error::AppError;
use crate::models::asset::Asset;
use crate::models::batch::{Batch, BatchWithAssets};
use crate::models::settings::SettingsMap;
use crate::noncustodial::types::{TxDraftSummary, WalletProfileSummary};
use rusqlite::{params, OptionalExtension};

pub fn get_settings(conn: &rusqlite::Connection) -> Result<SettingsMap, AppError> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = SettingsMap::new();
    for row in rows {
        let (k, v) = row?;
        map.insert(k, v);
    }
    Ok(map)
}

pub fn set_setting(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn list_assets(
    conn: &rusqlite::Connection,
    status: Option<&str>,
    is_staked: Option<bool>,
    search: Option<&str>,
    sort_by: Option<&str>,
    sort_dir: Option<&str>,
) -> Result<Vec<Asset>, AppError> {
    let mut sql = String::from("SELECT * FROM assets WHERE 1=1");
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut param_idx = 1;

    if let Some(s) = status {
        sql.push_str(&format!(" AND status = ?{}", param_idx));
        param_values.push(Box::new(s.to_string()));
        param_idx += 1;
    }

    if let Some(staked) = is_staked {
        sql.push_str(&format!(" AND is_staked = ?{}", param_idx));
        param_values.push(Box::new(if staked { 1 } else { 0 }));
        param_idx += 1;
    }

    if let Some(q) = search {
        if !q.is_empty() {
            sql.push_str(&format!(
                " AND (tld LIKE ?{param_idx} OR notes LIKE ?{param_idx} OR category LIKE ?{param_idx})",
                param_idx = param_idx
            ));
            param_values.push(Box::new(format!("%{}%", q)));
        }
    }

    let valid_sort_cols = [
        "tld",
        "status",
        "is_staked",
        "category",
        "hns_received",
        "expires_at_height",
        "updated_at",
        "created_at",
    ];
    let col = sort_by
        .filter(|c| valid_sort_cols.contains(c))
        .unwrap_or("tld");
    let dir = if sort_dir == Some("desc") {
        "DESC"
    } else {
        "ASC"
    };
    sql.push_str(&format!(" ORDER BY {} {}", col, dir));

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), |row| Ok(Asset::from_row(row)))?;

    let mut assets = Vec::new();
    for row in rows {
        assets.push(row??);
    }
    Ok(assets)
}

pub fn get_asset(conn: &rusqlite::Connection, id: i64) -> Result<Asset, AppError> {
    conn.query_row("SELECT * FROM assets WHERE id = ?1", params![id], |row| {
        Ok(Asset::from_row(row))
    })?
    .map_err(AppError::from)
}

#[allow(clippy::too_many_arguments)]
pub fn update_asset(
    conn: &rusqlite::Connection,
    id: i64,
    status: Option<&str>,
    category: Option<&str>,
    tags: Option<&str>,
    notes: Option<&str>,
    hns_received: Option<i64>,
    transfer_tx_hash: Option<&str>,
    finalize_tx_hash: Option<&str>,
) -> Result<(), AppError> {
    let mut sets = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut param_idx = 1;

    if let Some(v) = status {
        sets.push(format!("status = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = category {
        sets.push(format!("category = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = tags {
        sets.push(format!("tags = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = notes {
        sets.push(format!("notes = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = hns_received {
        sets.push(format!("hns_received = ?{}", param_idx));
        param_values.push(Box::new(v));
        param_idx += 1;
    }
    if let Some(v) = transfer_tx_hash {
        sets.push(format!("transfer_tx_hash = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = finalize_tx_hash {
        sets.push(format!("finalize_tx_hash = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = datetime('now')".to_string());
    let sql = format!(
        "UPDATE assets SET {} WHERE id = ?{}",
        sets.join(", "),
        param_idx
    );
    param_values.push(Box::new(id));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    conn.execute(&sql, params_ref.as_slice())?;

    conn.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('asset_update', 'asset', ?1, ?2)",
        params![id, serde_json::json!({"fields_updated": sets.len() - 1}).to_string()],
    )?;

    Ok(())
}

pub fn bulk_update_status(
    conn: &rusqlite::Connection,
    ids: &[i64],
    status: &str,
) -> Result<usize, AppError> {
    let tx = conn.unchecked_transaction()?;
    let mut updated = 0;
    for &id in ids {
        let n = tx.execute(
            "UPDATE assets SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![status, id],
        )?;
        updated += n;
    }
    tx.execute(
        "INSERT INTO audit_log (action, entity, detail) VALUES ('bulk_status_change', 'asset', ?1)",
        params![serde_json::json!({"ids": ids, "status": status, "count": updated}).to_string()],
    )?;
    tx.commit()?;
    Ok(updated)
}

pub fn bulk_update_tags(
    conn: &rusqlite::Connection,
    ids: &[i64],
    tags: &str,
) -> Result<usize, AppError> {
    let tx = conn.unchecked_transaction()?;
    let mut updated = 0;
    for &id in ids {
        let n = tx.execute(
            "UPDATE assets SET tags = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![tags, id],
        )?;
        updated += n;
    }
    tx.execute(
        "INSERT INTO audit_log (action, entity, detail) VALUES ('bulk_tag_change', 'asset', ?1)",
        params![serde_json::json!({"ids": ids, "tags": tags, "count": updated}).to_string()],
    )?;
    tx.commit()?;
    Ok(updated)
}

/// Set an inventory asset's migration status by TLD (no-op if the name isn't in
/// the inventory). Used to reflect an initiated Namebase transfer.
pub fn set_asset_status_by_tld(
    conn: &rusqlite::Connection,
    tld: &str,
    status: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE assets SET status = ?1, updated_at = datetime('now') WHERE tld = ?2",
        params![status, tld],
    )?;
    Ok(())
}

pub fn delete_asset(conn: &rusqlite::Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM assets WHERE id = ?1", params![id])?;
    conn.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('asset_delete', 'asset', ?1, ?2)",
        params![id, "{}"],
    )?;
    Ok(())
}

pub fn list_batches(conn: &rusqlite::Connection) -> Result<Vec<Batch>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT b.*, COUNT(ba.id) as asset_count
         FROM batches b
         LEFT JOIN batch_assets ba ON ba.batch_id = b.id
         GROUP BY b.id
         ORDER BY b.created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| Ok(Batch::from_row(row)))?;
    let mut batches = Vec::new();
    for row in rows {
        batches.push(row??);
    }
    Ok(batches)
}

pub fn get_batch_with_assets(
    conn: &rusqlite::Connection,
    batch_id: i64,
) -> Result<BatchWithAssets, AppError> {
    let batch = conn.query_row(
        "SELECT b.*, COUNT(ba.id) as asset_count
         FROM batches b
         LEFT JOIN batch_assets ba ON ba.batch_id = b.id
         WHERE b.id = ?1
         GROUP BY b.id",
        params![batch_id],
        |row| Ok(Batch::from_row(row)),
    )??;

    let mut stmt = conn.prepare(
        "SELECT a.* FROM assets a
         INNER JOIN batch_assets ba ON ba.asset_id = a.id
         WHERE ba.batch_id = ?1
         ORDER BY ba.sort_order",
    )?;
    let assets = stmt
        .query_map(params![batch_id], Asset::from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(BatchWithAssets {
        id: batch.id,
        name: batch.name,
        description: batch.description,
        status: batch.status,
        asset_count: batch.asset_count,
        assets,
        created_at: batch.created_at,
        updated_at: batch.updated_at,
    })
}

pub fn create_batch(
    conn: &rusqlite::Connection,
    name: &str,
    description: Option<&str>,
    asset_ids: &[i64],
) -> Result<i64, AppError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO batches (name, description) VALUES (?1, ?2)",
        params![name, description],
    )?;
    let batch_id = tx.last_insert_rowid();

    for (i, &asset_id) in asset_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO batch_assets (batch_id, asset_id, sort_order) VALUES (?1, ?2, ?3)",
            params![batch_id, asset_id, i as i64],
        )?;
    }

    tx.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('batch_create', 'batch', ?1, ?2)",
        params![batch_id, serde_json::json!({"name": name, "asset_count": asset_ids.len()}).to_string()],
    )?;
    tx.commit()?;
    Ok(batch_id)
}

pub fn update_batch(
    conn: &rusqlite::Connection,
    id: i64,
    name: Option<&str>,
    description: Option<&str>,
    status: Option<&str>,
) -> Result<(), AppError> {
    let mut sets = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut param_idx = 1;

    if let Some(v) = name {
        sets.push(format!("name = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = description {
        sets.push(format!("description = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }
    if let Some(v) = status {
        sets.push(format!("status = ?{}", param_idx));
        param_values.push(Box::new(v.to_string()));
        param_idx += 1;
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = datetime('now')".to_string());
    let sql = format!(
        "UPDATE batches SET {} WHERE id = ?{}",
        sets.join(", "),
        param_idx
    );
    param_values.push(Box::new(id));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    conn.execute(&sql, params_ref.as_slice())?;

    conn.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('batch_update', 'batch', ?1, ?2)",
        params![id, serde_json::json!({"fields_updated": sets.len() - 1}).to_string()],
    )?;
    Ok(())
}

pub fn delete_batch(conn: &rusqlite::Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM batches WHERE id = ?1", params![id])?;
    conn.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('batch_delete', 'batch', ?1, ?2)",
        params![id, "{}"],
    )?;
    Ok(())
}

pub fn add_to_batch(
    conn: &rusqlite::Connection,
    batch_id: i64,
    asset_ids: &[i64],
) -> Result<usize, AppError> {
    let max_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM batch_assets WHERE batch_id = ?1",
            params![batch_id],
            |row| row.get(0),
        )
        .unwrap_or(-1);

    let mut added = 0;
    for (i, &asset_id) in asset_ids.iter().enumerate() {
        let n = conn.execute(
            "INSERT OR IGNORE INTO batch_assets (batch_id, asset_id, sort_order) VALUES (?1, ?2, ?3)",
            params![batch_id, asset_id, max_order + 1 + i as i64],
        )?;
        added += n;
    }
    conn.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('batch_add_assets', 'batch', ?1, ?2)",
        params![batch_id, serde_json::json!({"asset_ids": asset_ids, "added": added}).to_string()],
    )?;
    Ok(added)
}

pub fn remove_from_batch(
    conn: &rusqlite::Connection,
    batch_id: i64,
    asset_ids: &[i64],
) -> Result<usize, AppError> {
    let mut removed = 0;
    for &asset_id in asset_ids {
        let n = conn.execute(
            "DELETE FROM batch_assets WHERE batch_id = ?1 AND asset_id = ?2",
            params![batch_id, asset_id],
        )?;
        removed += n;
    }
    conn.execute(
        "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('batch_remove_assets', 'batch', ?1, ?2)",
        params![batch_id, serde_json::json!({"asset_ids": asset_ids, "removed": removed}).to_string()],
    )?;
    Ok(removed)
}

pub fn get_dashboard_stats(conn: &rusqlite::Connection) -> Result<serde_json::Value, AppError> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?;
    let staked: i64 =
        conn.query_row("SELECT COUNT(*) FROM assets WHERE is_staked = 1", [], |r| {
            r.get(0)
        })?;
    let unstaked = total - staked;

    let mut status_counts = serde_json::Map::new();
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM assets GROUP BY status")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (status, count) = row?;
        status_counts.insert(status, serde_json::Value::Number(count.into()));
    }

    let recent_audit = get_recent_audit_log(conn, 10)?;

    Ok(serde_json::json!({
        "total": total,
        "staked": staked,
        "unstaked": unstaked,
        "status_counts": status_counts,
        "recent_audit": recent_audit,
    }))
}

pub fn get_recent_audit_log(
    conn: &rusqlite::Connection,
    limit: i64,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, action, entity, entity_id, detail, created_at
         FROM audit_log ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "timestamp": row.get::<_, String>(1)?,
            "action": row.get::<_, String>(2)?,
            "entity": row.get::<_, Option<String>>(3)?,
            "entity_id": row.get::<_, Option<i64>>(4)?,
            "detail": row.get::<_, Option<String>>(5)?,
            "created_at": row.get::<_, String>(6)?,
        }))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

pub fn insert_wallet_snapshot(
    conn: &rusqlite::Connection,
    wallet_name: &str,
    balance: i64,
    address: Option<&str>,
    name_count: i64,
    raw_json: Option<&str>,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO wallet_snapshots (wallet_name, balance, address, name_count, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![wallet_name, balance, address, name_count, raw_json],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_latest_wallet_snapshot(
    conn: &rusqlite::Connection,
) -> Result<Option<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, snapshot_at, wallet_name, balance, address, name_count
         FROM wallet_snapshots ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "snapshot_at": row.get::<_, String>(1)?,
            "wallet_name": row.get::<_, String>(2)?,
            "balance": row.get::<_, i64>(3)?,
            "address": row.get::<_, Option<String>>(4)?,
            "name_count": row.get::<_, i64>(5)?,
        }))
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn get_wallet_snapshots(
    conn: &rusqlite::Connection,
    limit: i64,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, snapshot_at, wallet_name, balance, address, name_count
         FROM wallet_snapshots ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "snapshot_at": row.get::<_, String>(1)?,
            "wallet_name": row.get::<_, String>(2)?,
            "balance": row.get::<_, i64>(3)?,
            "address": row.get::<_, Option<String>>(4)?,
            "name_count": row.get::<_, i64>(5)?,
        }))
    })?;
    let mut snapshots = Vec::new();
    for row in rows {
        snapshots.push(row?);
    }
    Ok(snapshots)
}

/// Collect distinct, non-empty addresses recorded in wallet snapshots, newest
/// first. Used to auto-derive watch addresses for external read-only mode so
/// the user does not have to enter them manually.
pub fn get_known_wallet_addresses(
    conn: &rusqlite::Connection,
    limit: i64,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT address FROM wallet_snapshots
         WHERE address IS NOT NULL AND address != ''
         ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| row.get::<_, String>(0))?;
    let mut addresses = Vec::new();
    for row in rows {
        addresses.push(row?);
    }
    Ok(addresses)
}

/// Replace the cached address set for a specific wallet. Called after a sync
/// against a (local or remote) hsd, so external read-only mode can resolve the
/// selected wallet's full balance/assets without manual watch addresses.
pub fn replace_wallet_addresses(
    conn: &rusqlite::Connection,
    wallet_id: &str,
    addresses: &[String],
) -> Result<usize, AppError> {
    // Upsert each address (preserving first_seen, refreshing last_seen). We do
    // not delete stale rows: an address that was ever owned by the wallet stays
    // relevant for read-only history.
    let mut inserted = 0usize;
    for addr in addresses {
        let trimmed = addr.trim();
        if trimmed.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO wallet_addresses (wallet_id, address, last_seen)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(wallet_id, address)
             DO UPDATE SET last_seen = datetime('now')",
            params![wallet_id, trimmed],
        )?;
        inserted += 1;
    }
    Ok(inserted)
}

/// Get the cached addresses for a specific wallet, newest activity first.
pub fn get_wallet_addresses_for_wallet(
    conn: &rusqlite::Connection,
    wallet_id: &str,
    limit: i64,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT address FROM wallet_addresses
         WHERE wallet_id = ?1
         ORDER BY last_seen DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![wallet_id, limit], |row| row.get::<_, String>(0))?;
    let mut addresses = Vec::new();
    for row in rows {
        addresses.push(row?);
    }
    Ok(addresses)
}

/// Collect the TLDs tracked in the local inventory. Used to auto-derive watch
/// names for external read-only mode.
pub fn get_inventory_tlds(conn: &rusqlite::Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare("SELECT tld FROM assets ORDER BY tld ASC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut tlds = Vec::new();
    for row in rows {
        tlds.push(row?);
    }
    Ok(tlds)
}

pub fn get_assets_by_tlds(
    conn: &rusqlite::Connection,
    tlds: &[String],
) -> Result<Vec<Asset>, AppError> {
    let mut assets = Vec::new();
    for tld in tlds {
        let result = conn.query_row("SELECT * FROM assets WHERE tld = ?1", params![tld], |row| {
            Asset::from_row(row)
        });
        match result {
            Ok(asset) => assets.push(asset),
            Err(_) => continue,
        }
    }
    Ok(assets)
}

/// Select the next window of names to re-check during a background repair sweep.
///
/// Candidates are the union of inventory TLDs (`assets`) and this profile's
/// tracked names not already in `assets`. Rows whose `last_synced_at` falls
/// within the last `min_age_hours` are excluded so a background loop converges
/// instead of re-checking the same ~hundreds of names every run. Ordering is
/// oldest-first (`NULL`/never-synced first, then `name ASC` as an explicit,
/// deterministic tiebreak — see below), LIMIT `max` — so successive runs page
/// through the whole inventory.
///
/// Known limitation: tracked-only names (no `assets` row) always report
/// `last_synced_at = NULL` here, because `touch_asset_synced`/
/// `mark_asset_finalized_owned` only ever `UPDATE assets ... WHERE tld = ?`,
/// which is a no-op when the tld isn't in `assets`. So a tracked-only name
/// always sorts into the NULL group and never "converges" the way inventory
/// rows do (its `last_synced_at` never advances). This is why the ORDER BY
/// has an explicit `name ASC` tiebreak: `repair_step_windowed`'s caller-side
/// `attempted` set relies on repeated calls with a GROWING `max` returning a
/// stable, strictly-increasing prefix of the same ordering, so that
/// filtering out already-attempted names always surfaces the next unseen
/// ones instead of re-fetching the same top-`max` rows forever (which, before
/// this tiebreak was added, could happen if ties broke inconsistently).
pub fn list_repair_candidates(
    conn: &rusqlite::Connection,
    profile_id: &str,
    max: u32,
    min_age_hours: i64,
) -> Result<Vec<String>, AppError> {
    let age_modifier = format!("-{} hours", min_age_hours);
    let mut stmt = conn.prepare(
        "SELECT name, last_synced_at FROM (
             SELECT tld AS name, last_synced_at FROM assets
             UNION
             SELECT name AS name, NULL AS last_synced_at
               FROM tracked_name_states
              WHERE wallet_profile_id = ?1
                AND name NOT IN (SELECT tld FROM assets)
         )
         WHERE last_synced_at IS NULL OR last_synced_at <= datetime('now', ?2)
         ORDER BY (last_synced_at IS NOT NULL), last_synced_at ASC, name ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![profile_id, age_modifier, max as i64], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Count how many candidates [`list_repair_candidates`] would return with an
/// unbounded window — i.e. the total backlog of names still needing a repair
/// check this run (same UNION + recency filter, no `LIMIT`). Used to seed an
/// honest, monotonically-shrinking "remaining" progress figure for the
/// background repair convergence loop, which pages through the backlog in
/// fixed-size windows. Inherits the same known limitation documented on
/// [`list_repair_candidates`]: tracked-only names always match (never stamped),
/// so the caller de-duplicates already-attempted names via an in-run set.
pub fn count_repair_candidates(
    conn: &rusqlite::Connection,
    profile_id: &str,
    min_age_hours: i64,
) -> Result<u32, AppError> {
    let age_modifier = format!("-{} hours", min_age_hours);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT tld AS name, last_synced_at FROM assets
             UNION
             SELECT name AS name, NULL AS last_synced_at
               FROM tracked_name_states
              WHERE wallet_profile_id = ?1
                AND name NOT IN (SELECT tld FROM assets)
         )
         WHERE last_synced_at IS NULL OR last_synced_at <= datetime('now', ?2)",
        params![profile_id, age_modifier],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u32)
}

/// Mark an inventory asset as confirmed-owned on-chain: advance `status` to
/// `finalized_owned`, record the live `name_state`, and stamp `last_synced_at`.
/// Staked names (`do_not_touch_staked`) are never auto-advanced. A `tld` that
/// isn't in `assets` (e.g. a tracked-only name) simply updates zero rows.
pub fn mark_asset_finalized_owned(
    conn: &rusqlite::Connection,
    tld: &str,
    name_state: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE assets
            SET status = 'finalized_owned',
                name_state = ?2,
                last_synced_at = datetime('now'),
                updated_at = datetime('now')
          WHERE tld = ?1 AND status != 'do_not_touch_staked'",
        params![tld, name_state],
    )?;
    Ok(())
}

/// Record that an inventory asset was checked during a repair sweep but is not
/// (or not yet) owned by this wallet: stamp `last_synced_at` only, leaving the
/// `status` untouched. This is what lets repeated repair runs converge instead
/// of re-checking not-owned names forever. A `tld` not in `assets` updates zero
/// rows.
pub fn touch_asset_synced(conn: &rusqlite::Connection, tld: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE assets
            SET last_synced_at = datetime('now'),
                updated_at = datetime('now')
          WHERE tld = ?1",
        params![tld],
    )?;
    Ok(())
}

/// Collect inventory TLDs whose `last_synced_at` falls within the last `hours`
/// — i.e. names a recent repair or discover sweep already checked. Used by
/// `discover_step` as a "recently checked" memo so it skips re-verifying names
/// still fresh from a prior run (resumable across Sync clicks).
///
/// Only `assets` rows are considered: a discovered-but-foreign name with no
/// `assets` row is never stamped by `touch_asset_synced` (that UPDATE is a
/// no-op), so it can't appear here — an accepted gap, such names are few.
pub fn list_recently_synced_tlds(
    conn: &rusqlite::Connection,
    hours: i64,
) -> Result<Vec<String>, AppError> {
    let age_modifier = format!("-{} hours", hours);
    let mut stmt =
        conn.prepare("SELECT tld FROM assets WHERE last_synced_at >= datetime('now', ?1)")?;
    let rows = stmt.query_map(params![age_modifier], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// =============================================================================
// Non-custodial wallet helpers
//
// Centralized query layer for the non-custodial schema (migrations 006-009):
// wallet profiles, encrypted secrets, and transaction drafts. Lower-level
// chain-cache helpers (derived addresses, UTXOs, name states, sync cursors)
// live in `noncustodial::{derivation, send, sync}` and are called directly by
// the command layer; these helpers cover the tables that had no home yet.
//
// IMPORTANT: nothing returned from this section to the frontend may contain
// secret material. `wallet_secrets` rows are read into backend-only buffers.
// =============================================================================

/// Explicit column list for `wallet_profiles`, in struct order, so `SELECT`s
/// stay stable regardless of future schema additions.
const PROFILE_COLS: &str = "id, label, kind, network, account_xpub, account_index, \
     receive_depth, change_depth, receive_address, last_synced_height, \
     last_synced_at, watch_only, \
     (SELECT CASE WHEN s.kdf IS NULL OR s.kdf = 'none' THEN 0 ELSE 1 END \
        FROM wallet_secrets s WHERE s.wallet_profile_id = wallet_profiles.id) \
        AS has_passphrase, \
     last_explorer_sync_at";

fn row_to_profile(row: &rusqlite::Row, active_id: &str) -> rusqlite::Result<WalletProfileSummary> {
    let id: String = row.get(0)?;
    let active = id == active_id;
    Ok(WalletProfileSummary {
        id,
        label: row.get(1)?,
        kind: row.get(2)?,
        network: row.get(3)?,
        account_xpub: row.get(4)?,
        account_index: row.get(5)?,
        receive_depth: row.get(6)?,
        change_depth: row.get(7)?,
        receive_address: row.get(8)?,
        last_synced_height: row.get(9)?,
        last_synced_at: row.get(10)?,
        watch_only: row.get::<_, i64>(11)? != 0,
        // NULL (watch-only / no secret row) -> no passphrase.
        has_passphrase: row.get::<_, Option<i64>>(12)?.unwrap_or(0) != 0,
        last_explorer_sync_at: row.get(13)?,
        active,
    })
}

/// The active wallet profile id from settings (empty string when none).
pub fn get_active_profile_id(conn: &rusqlite::Connection) -> Result<String, AppError> {
    let id: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'active_wallet_profile_id'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id.unwrap_or_default())
}

/// Mark a profile active (persisted in settings).
pub fn set_active_profile(conn: &rusqlite::Connection, profile_id: &str) -> Result<(), AppError> {
    set_setting(conn, "active_wallet_profile_id", profile_id)
}

/// Insert a new wallet profile. `receive_address` is set later (after the first
/// address is derived); depths start at 0.
#[allow(clippy::too_many_arguments)]
pub fn insert_wallet_profile(
    conn: &rusqlite::Connection,
    id: &str,
    label: &str,
    kind: &str,
    network: &str,
    account_xpub: &str,
    account_index: i64,
    watch_only: bool,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO wallet_profiles
            (id, label, kind, network, account_xpub, account_index, watch_only)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            label,
            kind,
            network,
            account_xpub,
            account_index,
            watch_only as i64
        ],
    )?;
    Ok(())
}

/// Fetch one profile, or `None` if it doesn't exist.
pub fn get_wallet_profile(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<WalletProfileSummary>, AppError> {
    let active_id = get_active_profile_id(conn)?;
    let sql = format!("SELECT {PROFILE_COLS} FROM wallet_profiles WHERE id = ?1");
    let profile = conn
        .query_row(&sql, params![id], |row| row_to_profile(row, &active_id))
        .optional()?;
    Ok(profile)
}

/// List all wallet profiles, newest first.
pub fn list_wallet_profiles(
    conn: &rusqlite::Connection,
) -> Result<Vec<WalletProfileSummary>, AppError> {
    let active_id = get_active_profile_id(conn)?;
    let sql = format!("SELECT {PROFILE_COLS} FROM wallet_profiles ORDER BY created_at DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row_to_profile(row, &active_id))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Delete a wallet profile and (via `ON DELETE CASCADE`) all its secrets,
/// addresses, UTXOs, drafts, bids, name-states, and sync cursors.
pub fn delete_wallet_profile(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM wallet_profiles WHERE id = ?1", params![id])?;
    Ok(())
}

/// Update the cached receive address and bump the receive depth high-water mark.
pub fn update_profile_receive(
    conn: &rusqlite::Connection,
    id: &str,
    receive_address: &str,
    receive_depth: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_profiles
            SET receive_address = ?2,
                receive_depth = MAX(receive_depth, ?3),
                updated_at = datetime('now')
         WHERE id = ?1",
        params![id, receive_address, receive_depth],
    )?;
    Ok(())
}

/// Bump the change depth high-water mark.
pub fn update_profile_change_depth(
    conn: &rusqlite::Connection,
    id: &str,
    change_depth: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_profiles
            SET change_depth = MAX(change_depth, ?2), updated_at = datetime('now')
         WHERE id = ?1",
        params![id, change_depth],
    )?;
    Ok(())
}

/// Record the last synced height/time after a sync pass.
pub fn update_profile_sync(
    conn: &rusqlite::Connection,
    id: &str,
    height: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_profiles
            SET last_synced_height = ?2,
                last_synced_at = datetime('now'),
                updated_at = datetime('now')
         WHERE id = ?1",
        params![id, height],
    )?;
    Ok(())
}

/// Stamp `last_explorer_sync_at` for a profile (Task 11 review, Finding 2).
///
/// Called exactly ONCE, from the "Done" block of `start_full_sync`'s
/// background thread (`commands/sync.rs`) — never from inside
/// `repair_step_windowed`/`discover_step` — and only when that run reached
/// the end with no cancellation and no `SYNC_MAX_CONSECUTIVE_ERRORS` abort.
/// This is a plain, separate timestamp from `update_profile_sync`'s
/// `last_synced_at` (which only the node-RPC step advances): explorer-only
/// mode has no node step, so without this column the UI's "Last successful
/// sync" line stayed "—" forever even after fully successful explorer syncs.
pub fn stamp_explorer_sync(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_profiles
            SET last_explorer_sync_at = datetime('now'),
                updated_at = datetime('now')
         WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Store an encrypted secret envelope for a hot profile.
///
/// The `vault::encrypt` blob is self-describing (it embeds salt + nonce + ct),
/// so the whole blob is stored hex-encoded in `ciphertext_hex`; the separate
/// `kdf_salt_hex` / `nonce_hex` columns are left empty (they are redundant with
/// the blob). `public_fingerprint` is a non-secret identifier of the account key.
/// `kdf` is `'argon2id'` for passphrase-protected secrets, or `'none'` when the
/// user opted out of a passphrase (the seed is still encrypted under a
/// device-local key, but unlocking requires no prompt).
pub fn insert_wallet_secret(
    conn: &rusqlite::Connection,
    profile_id: &str,
    vault_blob: &[u8],
    kdf: &str,
    public_fingerprint: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO wallet_secrets
            (wallet_profile_id, kdf, kdf_salt_hex, nonce_hex, ciphertext_hex, public_fingerprint)
         VALUES (?1, ?2, '', '', ?3, ?4)",
        params![profile_id, kdf, hex::encode(vault_blob), public_fingerprint],
    )?;
    Ok(())
}

/// Read the encrypted vault blob + its `kdf` marker for a profile.
///
/// Returns `None` for watch-only profiles (no secret row). The blob is passed
/// straight to `vault::decrypt`; it is NEVER returned to React. `kdf == "none"`
/// means the wallet has no passphrase (decrypt with the device-local key).
pub fn get_wallet_secret_meta(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Option<(Vec<u8>, String)>, AppError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT ciphertext_hex, kdf FROM wallet_secrets WHERE wallet_profile_id = ?1",
            params![profile_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((h, kdf)) => {
            let bytes = hex::decode(&h)
                .map_err(|e| AppError::Crypto(format!("corrupt secret blob: {e}")))?;
            Ok(Some((bytes, kdf)))
        }
        None => Ok(None),
    }
}

// --- Transaction drafts ----------------------------------------------------

/// A draft row as stored, including fields the command layer needs to sign and
/// broadcast (kept backend-internal; the frontend gets [`TxDraftSummary`]).
#[derive(Debug, Clone)]
pub struct TxDraftRow {
    pub id: String,
    pub wallet_profile_id: String,
    pub action: String,
    pub unsigned_tx_hex: String,
    pub signed_tx_hex: Option<String>,
    pub signing_inputs_json: String,
    pub summary_json: String,
    pub status: String,
    pub error_message: Option<String>,
    pub txid: Option<String>,
    pub confirmation_height: Option<i64>,
    pub created_at: String,
}

const DRAFT_COLS: &str = "id, wallet_profile_id, action, unsigned_tx_hex, signed_tx_hex, \
     signing_inputs_json, summary_json, status, error_message, txid, confirmation_height, created_at";

fn row_to_draft(row: &rusqlite::Row) -> rusqlite::Result<TxDraftRow> {
    Ok(TxDraftRow {
        id: row.get(0)?,
        wallet_profile_id: row.get(1)?,
        action: row.get(2)?,
        unsigned_tx_hex: row.get(3)?,
        signed_tx_hex: row.get(4)?,
        signing_inputs_json: row.get(5)?,
        summary_json: row.get(6)?,
        status: row.get(7)?,
        error_message: row.get(8)?,
        txid: row.get(9)?,
        confirmation_height: row.get(10)?,
        created_at: row.get(11)?,
    })
}

impl TxDraftRow {
    /// Project to the frontend-facing summary (parsing `summary_json`).
    pub fn to_summary(&self) -> TxDraftSummary {
        let summary = serde_json::from_str(&self.summary_json).unwrap_or(serde_json::Value::Null);
        TxDraftSummary {
            id: self.id.clone(),
            wallet_profile_id: self.wallet_profile_id.clone(),
            action: self.action.clone(),
            status: self.status.clone(),
            summary,
            error_message: self.error_message.clone(),
            txid: self.txid.clone(),
            confirmation_height: self.confirmation_height,
            created_at: self.created_at.clone(),
        }
    }
}

/// Insert a new draft in `draft` status.
pub fn insert_tx_draft(
    conn: &rusqlite::Connection,
    id: &str,
    profile_id: &str,
    action: &str,
    unsigned_tx_hex: &str,
    signing_inputs_json: &str,
    summary_json: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO wallet_tx_drafts
            (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json, summary_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            profile_id,
            action,
            unsigned_tx_hex,
            signing_inputs_json,
            summary_json
        ],
    )?;
    Ok(())
}

/// Insert a new draft AND atomically claim its input coins (`tracked_utxos.
/// reserved_by_draft_id`) in one transaction (I3) — either both the draft row
/// and every reservation land, or neither does. There is never a window where
/// the draft exists but its inputs are still free for another draft to pick,
/// or vice versa.
///
/// Each input is claimed with a conditional `UPDATE ... WHERE (reserved_by_draft_id
/// IS NULL OR = this draft) AND spent_by_txid IS NULL`, so a coin already
/// claimed by a *different*, still-live draft (or since spent) cannot be
/// silently stolen. If any input fails to claim — e.g. two builds raced past
/// their own `load_spendable_coins` read before either persisted — the whole
/// transaction rolls back (the draft row disappears with it) and this
/// returns `AppError::InvalidInput`, so the second build fails fast with a
/// clear "try again" error instead of quietly producing a transaction that
/// would only surface as a double-spend later, at broadcast time.
#[allow(clippy::too_many_arguments)]
pub fn insert_tx_draft_reserving_coins(
    conn: &rusqlite::Connection,
    id: &str,
    profile_id: &str,
    action: &str,
    unsigned_tx_hex: &str,
    signing_inputs_json: &str,
    summary_json: &str,
    inputs: &[(String, u32)],
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;

    // Self-heal stale reservations first so an abandoned earlier draft never
    // blocks a legitimate new one.
    crate::noncustodial::send::release_stale_reservations(&tx, profile_id)?;

    tx.execute(
        "INSERT INTO wallet_tx_drafts
            (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json, summary_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            profile_id,
            action,
            unsigned_tx_hex,
            signing_inputs_json,
            summary_json
        ],
    )?;

    for (txid, vout) in inputs {
        let claimed = tx.execute(
            "UPDATE tracked_utxos SET reserved_by_draft_id = ?1
             WHERE wallet_profile_id = ?2 AND txid = ?3 AND vout = ?4
               AND spent_by_txid IS NULL
               AND (reserved_by_draft_id IS NULL OR reserved_by_draft_id = ?1)",
            params![id, profile_id, txid, *vout as i64],
        )?;
        if claimed == 0 {
            // Dropping `tx` without commit() rolls back everything above,
            // including the draft insert.
            return Err(AppError::InvalidInput(
                "one or more coins for this transaction were just reserved by another \
                 pending draft (or already spent) — please try again"
                    .to_string(),
            ));
        }
    }

    tx.commit()?;
    Ok(())
}

/// Release every coin reservation held by a draft (I3): on delete, on
/// broadcast rejection, and when a draft is found `dropped` (evicted /
/// never confirmed) so its coins become selectable again without waiting out
/// the full TTL. A no-op if the draft holds no reservations.
pub fn release_reserved_utxos_for_draft(
    conn: &rusqlite::Connection,
    draft_id: &str,
) -> Result<usize, AppError> {
    let n = conn.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = NULL WHERE reserved_by_draft_id = ?1",
        params![draft_id],
    )?;
    Ok(n)
}

/// Delete a draft and release any coins it had reserved, atomically. Refuses
/// to delete a draft that has actually reached, or may have reached, the
/// chain (`broadcasted` / `confirmed` / `broadcast_pending` — the last is a
/// transport-ambiguous broadcast attempt where the node may already hold the
/// tx, see `commands::tx::broadcast_tx_draft`) — deleting it would both
/// discard real tx history and free its reservation for re-selection while
/// the coin might genuinely be spent; those age out via their own status
/// lifecycle instead. `signed`/`draft`/`failed`/`dropped` drafts — nothing
/// irreversible has happened, or the node definitively rejected the tx — can
/// always be discarded.
pub fn delete_tx_draft(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    let status: Option<String> = tx
        .query_row(
            "SELECT status FROM wallet_tx_drafts WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let status = status.ok_or_else(|| AppError::NotFound(format!("draft {id}")))?;
    if status == "broadcasted" || status == "confirmed" || status == "broadcast_pending" {
        return Err(AppError::InvalidInput(
            "cannot delete a draft that has already been broadcast".to_string(),
        ));
    }
    tx.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = NULL WHERE reserved_by_draft_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM wallet_tx_drafts WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(())
}

/// Fetch one draft, or `None`.
pub fn get_tx_draft(conn: &rusqlite::Connection, id: &str) -> Result<Option<TxDraftRow>, AppError> {
    let sql = format!("SELECT {DRAFT_COLS} FROM wallet_tx_drafts WHERE id = ?1");
    let row = conn.query_row(&sql, params![id], row_to_draft).optional()?;
    Ok(row)
}

/// Mark a draft signed: store the signed tx hex, refresh the summary, set
/// status `signed`.
pub fn update_tx_draft_signed(
    conn: &rusqlite::Connection,
    id: &str,
    signed_tx_hex: &str,
    summary_json: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_tx_drafts
            SET signed_tx_hex = ?2, summary_json = ?3, status = 'signed',
                error_message = NULL, updated_at = datetime('now')
         WHERE id = ?1",
        params![id, signed_tx_hex, summary_json],
    )?;
    Ok(())
}

/// Update a draft's status, optional error, and optional broadcast txid.
pub fn update_tx_draft_status(
    conn: &rusqlite::Connection,
    id: &str,
    status: &str,
    error_message: Option<&str>,
    txid: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_tx_drafts
            SET status = ?2, error_message = ?3, txid = COALESCE(?4, txid),
                updated_at = datetime('now')
         WHERE id = ?1",
        params![id, status, error_message, txid],
    )?;
    Ok(())
}

/// Mark a draft `confirmed` and record the block height it was mined at.
/// `txid` is optional and only overwrites the stored value when `Some` (via
/// `COALESCE`) — needed when a `broadcast_pending` draft (which has no DB
/// txid, only a locally-computed one) is promoted straight to `confirmed` in
/// one step (I5 / broadcast_pending auto-resolution).
pub fn update_tx_draft_confirmation(
    conn: &rusqlite::Connection,
    id: &str,
    confirmation_height: i64,
    txid: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_tx_drafts
            SET status = 'confirmed', confirmation_height = ?2,
                txid = COALESCE(?3, txid),
                error_message = NULL, updated_at = datetime('now')
         WHERE id = ?1",
        params![id, confirmation_height, txid],
    )?;
    Ok(())
}

/// Revert a `confirmed` draft back to `broadcasted` and clear its recorded
/// height (I5 reorg handling): the node no longer knows the tx at the height
/// it was previously confirmed at, so it re-enters mempool tracking — the
/// existing eviction-grace logic in `refresh_tx_confirmations` then decides
/// whether it eventually lands again or is judged `dropped`. The txid is
/// preserved (it never changes). `note` explains the revert via
/// `error_message` (surfaced to the user), mirroring the `dropped` path.
pub fn revert_tx_draft_to_broadcasted(
    conn: &rusqlite::Connection,
    id: &str,
    note: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE wallet_tx_drafts
            SET status = 'broadcasted', confirmation_height = NULL,
                error_message = ?2, updated_at = datetime('now')
         WHERE id = ?1",
        params![id, note],
    )?;
    Ok(())
}

/// Seconds elapsed since a draft row was created (`created_at`). Errors if
/// absent. NOTE: the eviction/failure grace windows deliberately do NOT use
/// this — they key off [`draft_updated_age_secs`], because `created_at` never
/// moves: an old draft that re-enters tracking (e.g. a confirmed draft
/// reorg-reverted back to `broadcasted`) would flunk a created_at-based grace
/// instantly and be mislabeled `dropped` forever (Task 8 review).
pub fn draft_age_secs(conn: &rusqlite::Connection, id: &str) -> Result<i64, AppError> {
    let secs: i64 = conn.query_row(
        "SELECT CAST((julianday('now') - julianday(created_at)) * 86400 AS INTEGER)
         FROM wallet_tx_drafts WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(secs)
}

/// Seconds elapsed since a draft row was last updated (`updated_at`). Used as
/// the grace window before a) a `broadcast_pending` draft the node
/// definitively doesn't know about is judged `failed`, and b) a `broadcasted`
/// -but-unfound draft is judged `dropped` — measured from the draft's last
/// update rather than its original creation, since a draft can sit in earlier
/// statuses (`draft`/`signed`) for an arbitrary user-paced amount of time
/// before ever being broadcast, and a reorg-reverted `confirmed` draft
/// re-enters `broadcasted` tracking with its `updated_at` freshly set by the
/// revert (giving it a full new window instead of an instant drop; Task 8
/// review). Every status transition (`update_tx_draft_status`,
/// `update_tx_draft_confirmation`, `revert_tx_draft_to_broadcasted`) sets
/// `updated_at = datetime('now')`, so "last update" always means "when it
/// entered its current status".
pub fn draft_updated_age_secs(conn: &rusqlite::Connection, id: &str) -> Result<i64, AppError> {
    let secs: i64 = conn.query_row(
        "SELECT CAST((julianday('now') - julianday(updated_at)) * 86400 AS INTEGER)
         FROM wallet_tx_drafts WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(secs)
}

/// Drafts whose on-chain status the node should be re-polled for (I5):
/// `broadcasted` (mempool → confirmed/dropped), `broadcast_pending`
/// (transport-ambiguous broadcasts — txid is computed locally by the caller,
/// not read from this row, since the DB `txid` column is NULL until the node
/// confirms it knows the tx), and `confirmed` drafts that have NOT yet
/// reached `finality_depth` confirmations at `tip_height` (I5 core: keep
/// re-verifying a confirmed tx until it's deeply buried, so a reorg that
/// un-mines it is caught instead of trusting a stale `confirmed` status
/// forever). A `confirmed` row with no recorded height (shouldn't normally
/// happen, but tolerated) is always included so it can be backfilled. Newest
/// first.
pub fn list_drafts_awaiting_confirmation(
    conn: &rusqlite::Connection,
    profile_id: &str,
    tip_height: i64,
    finality_depth: i64,
) -> Result<Vec<TxDraftRow>, AppError> {
    let sql = format!(
        "SELECT {DRAFT_COLS} FROM wallet_tx_drafts
         WHERE wallet_profile_id = ?1
           AND (
             status = 'broadcast_pending'
             OR (status = 'broadcasted' AND txid IS NOT NULL)
             OR (status = 'confirmed' AND txid IS NOT NULL
                 AND (confirmation_height IS NULL
                      OR (?2 - confirmation_height + 1) < ?3))
           )
         ORDER BY created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![profile_id, tip_height, finality_depth],
        row_to_draft,
    )?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// All derived address strings for a profile (both branches). Used by the sync
/// engine to scan the node for coins.
pub fn get_profile_addresses(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT address FROM derived_addresses
         WHERE wallet_profile_id = ?1 ORDER BY branch, child_index",
    )?;
    let rows = stmt.query_map(params![profile_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// List drafts for a profile, newest first.
pub fn list_tx_drafts(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<TxDraftSummary>, AppError> {
    let sql = format!(
        "SELECT {DRAFT_COLS} FROM wallet_tx_drafts
         WHERE wallet_profile_id = ?1 ORDER BY created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id], row_to_draft)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?.to_summary());
    }
    Ok(out)
}

/// Whether a not-yet-terminal draft of `action` already exists for `name` in
/// this profile. Generic form of the I2 bid-multiplicity guard's draft check
/// (part 2 — part 1 is an unspent on-chain covenant coin, see
/// [`find_unspent_covenant_utxos_by_name_hash`]); reused by the Task 1
/// double-open guard (`action = "open"`) so both guards share one
/// implementation instead of duplicating the `summary_json` parse.
///
/// "Not-yet-terminal" = `draft`, `signed`, `broadcast_pending`, or
/// `broadcasted`: a second build must not be able to queue a second action
/// for the same name while an earlier one might still land on-chain.
/// `confirmed` is deliberately excluded here — a confirmed action already has
/// an unspent covenant coin, which part (a) of each guard catches;
/// `dropped`/`failed` drafts never reached (or will never reach) the chain
/// and must not block a retry.
///
/// There is no `name` column on `wallet_tx_drafts` — the name lives inside
/// `summary_json` (see [`ActionSummary`] in `commands::names`) — so this
/// filters by `action` + status in SQL, then parses `summary_json` in Rust to
/// match the exact name (avoids relying on the `json1` SQLite extension and
/// avoids substring false-positives from a raw `LIKE`).
pub fn has_pending_draft_for_name(
    conn: &rusqlite::Connection,
    profile_id: &str,
    action: &str,
    name: &str,
) -> Result<bool, AppError> {
    let sql = format!(
        "SELECT {DRAFT_COLS} FROM wallet_tx_drafts
         WHERE wallet_profile_id = ?1 AND action = ?2
           AND status IN ('draft','signed','broadcast_pending','broadcasted')"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id, action], row_to_draft)?;
    for r in rows {
        let row = r?;
        if draft_summary_covers_name(&row.summary_json, name) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// True when a draft's `summary_json` names `name` — either as its single
/// `name` field OR as a member of its `nameList` array (batch drafts persist
/// one row covering many names). Single-name drafts have no `nameList`, so
/// this stays equivalent to the old `name`-only match for them.
fn draft_summary_covers_name(summary_json: &str, name: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(summary_json) else {
        return false;
    };
    if v.get("name").and_then(|n| n.as_str()) == Some(name) {
        return true;
    }
    v.get("nameList")
        .and_then(|l| l.as_array())
        .map(|arr| arr.iter().any(|e| e.as_str() == Some(name)))
        .unwrap_or(false)
}

/// True when a pending bid draft (single `"bid"` OR `"batch-bid"`) already
/// covers `name`. Batch-bid persists ONE draft row with all names in its
/// `nameList`, so we must scan both action verbs and both the `name` field and
/// the `nameList` array — otherwise a follow-up single bid on a name that is
/// mid-batch would slip past the multiplicity guard.
pub fn has_pending_bid_draft_for_name(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<bool, AppError> {
    Ok(has_pending_draft_for_name(conn, profile_id, "bid", name)?
        || has_pending_draft_for_name(conn, profile_id, "batch-bid", name)?)
}

/// Look up the status of a tx draft by its broadcast txid. Returns `None` if
/// no draft with that txid exists for the given profile.
pub fn get_draft_status_by_txid(
    conn: &rusqlite::Connection,
    profile_id: &str,
    txid: &str,
) -> Result<Option<String>, AppError> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM wallet_tx_drafts
             WHERE wallet_profile_id = ?1 AND txid = ?2
             ORDER BY created_at DESC LIMIT 1",
            params![profile_id, txid],
            |row| row.get(0),
        )
        .optional()?;
    Ok(status)
}

// --- Cache-backed read model (non-custodial) ------------------------------

/// Balance for a profile from the local UTXO cache, shaped like the frontend
/// `HsdBalance` ({confirmed, unconfirmed, locked_confirmed, locked_unconfirmed}).
/// Liquid coins map to `confirmed`; name-bound value (control + lockup) maps to
/// `locked_confirmed`. We don't yet split a mempool/unconfirmed bucket.
pub fn read_cached_balance(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<serde_json::Value, AppError> {
    let b = crate::noncustodial::sync::compute_balances(conn, profile_id)?;
    Ok(serde_json::json!({
        "confirmed": b.liquid,
        "unconfirmed": 0,
        "locked_confirmed": b.name_control + b.name_lockup,
        "locked_unconfirmed": 0,
    }))
}

/// Wallet-owned names from `tracked_name_states`, shaped like the frontend
/// `HsdName`. "Owned" = the name's owner outpoint matches an unspent tracked
/// UTXO for this profile.
pub fn read_cached_names(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    // COV_REGISTER = 6. The owner UTXO's covenant type determines if the name
    // is already registered (covenant >= 6) or just won (covenant < 6, e.g.
    // REVEAL=4). We derive `registered` from the covenant type rather than
    // relying on `raw_json` because the node RPC response (getnameinfo) never
    // includes a `registered` field — only the explorer API provides it.
    let mut stmt = conn.prepare(
        "SELECT n.name, n.state, n.height, n.renewal_height, n.owner_txid, n.owner_vout,
                (SELECT u.covenant_type FROM tracked_utxos u
                 WHERE u.wallet_profile_id = n.wallet_profile_id
                   AND u.txid = n.owner_txid
                   AND u.vout = n.owner_vout
                   AND u.spent_by_txid IS NULL) AS covenant_type,
                n.owner_address
         FROM tracked_name_states n
         WHERE n.wallet_profile_id = ?1
           AND EXISTS (
               SELECT 1 FROM tracked_utxos u
               WHERE u.wallet_profile_id = n.wallet_profile_id
                 AND u.txid = n.owner_txid
                 AND u.vout = n.owner_vout
                 AND u.spent_by_txid IS NULL
           )
         ORDER BY n.name",
    )?;
    let rows = stmt.query_map(params![profile_id], |row| {
        let name: String = row.get(0)?;
        let state: Option<String> = row.get(1)?;
        let height: Option<i64> = row.get(2)?;
        let renewal: Option<i64> = row.get(3)?;
        let owner_txid: Option<String> = row.get(4)?;
        let owner_vout: Option<i64> = row.get(5)?;
        let covenant_type: Option<i64> = row.get(6)?;
        let owner_address: Option<String> = row.get(7)?;
        let owner = owner_txid
            .map(|hash| serde_json::json!({ "hash": hash, "index": owner_vout.unwrap_or(0) }));

        // Derive registered from covenant type: >= COV_REGISTER (6) means registered.
        let registered = covenant_type.map(|ct| ct >= 6).unwrap_or(false);

        Ok(serde_json::json!({
            "name": name,
            "state": state,
            "height": height,
            "renewal": renewal,
            "owner": owner,
            "owner_address": owner_address,
            "registered": Some(registered),
            "expired": None::<bool>,
            "stats": serde_json::Value::Null,
        }))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Persist an explorer-discovered owned name into `tracked_name_states`,
/// recording the current owner outpoint so a node-free read can return it.
pub fn upsert_owned_name(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &crate::hsd::types::HsdName,
    owner_txid: &str,
    owner_vout: u32,
    owner_address: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
             owner_address, height, renewal_height, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(wallet_profile_id, name) DO UPDATE SET
            name_hash_hex  = excluded.name_hash_hex,
            state          = excluded.state,
            owner_txid     = excluded.owner_txid,
            owner_vout     = excluded.owner_vout,
            owner_address  = excluded.owner_address,
            height         = excluded.height,
            renewal_height = excluded.renewal_height,
            raw_json       = excluded.raw_json,
            updated_at     = datetime('now')",
        params![
            profile_id,
            name.name,
            name.name_hash.clone().unwrap_or_default(),
            name.state.clone().unwrap_or_else(|| "UNKNOWN".to_string()),
            owner_txid,
            owner_vout as i64,
            owner_address,
            name.height.map(|h| h as i64),
            name.renewal.map(|r| r as i64),
            serde_json::to_string(name).unwrap_or_default(),
        ],
    )?;
    Ok(())
}

/// Explorer-discovered owned names for a profile, shaped like the frontend
/// `HsdName`. Unlike [`read_cached_names`] this is NOT gated on `tracked_utxos`
/// (which only a node sync fills) — it returns the names whose current owner
/// outpoint was recorded by node-free discovery (`owner_txid IS NOT NULL`).
///
/// Also extracts `registered` and `expired` from the persisted `raw_json`
/// when available, so the frontend has accurate registration-status metadata
/// even when the local node is not fully synced.
pub fn read_owned_names_explorer(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT name, state, height, renewal_height, owner_txid, owner_vout, raw_json, owner_address
         FROM tracked_name_states
         WHERE wallet_profile_id = ?1 AND owner_txid IS NOT NULL
         ORDER BY name",
    )?;
    let rows = stmt.query_map(params![profile_id], |row| {
        let name: String = row.get(0)?;
        let state: Option<String> = row.get(1)?;
        let height: Option<i64> = row.get(2)?;
        let renewal: Option<i64> = row.get(3)?;
        let owner_txid: Option<String> = row.get(4)?;
        let owner_vout: Option<i64> = row.get(5)?;
        let raw_json: Option<String> = row.get(6)?;
        let owner_address: Option<String> = row.get(7)?;
        let owner = owner_txid
            .map(|hash| serde_json::json!({ "hash": hash, "index": owner_vout.unwrap_or(0) }));

        // Extract registered/expired from the persisted raw JSON.
        // The node RPC (getnameinfo) never includes `registered` — only the explorer.
        // When raw_json has `registered: null` but the name is CLOSED with an
        // owner_txid and a set renewal (far in the future from height), we can safely
        // derive `registered: true`. The name is clearly already owned and registered.
        let (registered, expired) = raw_json
            .as_deref()
            .and_then(|j| {
                let v: serde_json::Value = serde_json::from_str(j).ok()?;
                let raw_reg = v.get("registered").and_then(|x| x.as_bool());
                let raw_exp = v.get("expired").and_then(|x| x.as_bool());
                // If raw_json explicitly has registered, use it.
                if raw_reg.is_some() {
                    return Some((raw_reg, raw_exp));
                }
                // raw_json has no `registered` field (node response) → derive from context.
                // CLOSED state + renewal set = already registered.
                let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
                let renewal = v.get("renewal").and_then(|x| x.as_u64());
                let derived_reg = if state == "CLOSED" && renewal.unwrap_or(0) > 0 {
                    Some(true)
                } else {
                    None
                };
                Some((derived_reg, raw_exp))
            })
            .unwrap_or((None, None));

        Ok(serde_json::json!({
            "name": name,
            "state": state,
            "height": height,
            "renewal": renewal,
            "owner": owner,
            "owner_address": owner_address,
            "registered": registered,
            "expired": expired,
            "stats": serde_json::Value::Null,
        }))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// All name strings tracked for a profile (used as sync candidates).
pub fn list_tracked_name_names(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<String>, AppError> {
    let mut stmt =
        conn.prepare("SELECT name FROM tracked_name_states WHERE wallet_profile_id = ?1")?;
    let rows = stmt.query_map(params![profile_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Local (Sync-populated) evidence about a tracked name — the phase (`state`),
/// the recorded current-owner address (from explorer/history reconciliation),
/// and the raw name-info JSON. This carries NO spend authority on its own: a
/// spendable owner coin still requires a node-synced `tracked_utxos` row (see
/// [`get_name_coin`]). Used by `get_name_action_capabilities` to classify a name
/// as "owned but not locally manageable" when the node is unreachable.
#[derive(Debug, Clone)]
pub struct TrackedNameRow {
    pub name: String,
    pub state: Option<String>,
    pub owner_address: Option<String>,
    pub raw_json: Option<String>,
    /// Chain renewal height (`getnameinfo().info.renewal` / explorer
    /// `renewal`), when sync has recorded one. Used by the
    /// `get_name_action_capabilities` node-unreachable fallback to derive
    /// `days_until_expire` the same way `read_renewals` does (renewal height +
    /// network renewal window vs. a persisted height estimate) instead of
    /// leaving the expiry alarm silent for lack of live node stats.
    pub renewal_height: Option<i64>,
}

/// Fetch the tracked-name-state row for `name` under `profile_id`, if one exists.
pub fn get_tracked_name_state(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<TrackedNameRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT name, state, owner_address, raw_json, renewal_height
             FROM tracked_name_states
             WHERE wallet_profile_id = ?1 AND name = ?2",
            params![profile_id, name],
            |row| {
                Ok(TrackedNameRow {
                    name: row.get(0)?,
                    state: row.get(1)?,
                    owner_address: row.get(2)?,
                    raw_json: row.get(3)?,
                    renewal_height: row.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Cached transaction history for a profile, normalized to the flat shape the
/// frontend `normalizeTransaction` understands ({hash, value, direction,
/// address, confirmed, height, time}).
///
/// Direction/amount are derived from each cached `getrawtransaction` body by
/// comparing outputs against the profile's derived addresses (receives) and
/// inputs against its tracked UTXOs (spends). Parsing is best-effort: a tx whose
/// shape we don't recognize is reported as direction "other" with amount 0.
pub fn read_cached_transactions(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    use std::collections::HashSet;

    // Our receive/change addresses, and our utxo outpoints (for spend detection).
    let our_addrs: HashSet<String> = {
        let mut stmt =
            conn.prepare("SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1")?;
        let rows = stmt.query_map(params![profile_id], |r| r.get::<_, String>(0))?;
        let mut s = HashSet::new();
        for r in rows {
            s.insert(r?);
        }
        s
    };
    let our_outpoints: HashSet<(String, i64)> = {
        let mut stmt =
            conn.prepare("SELECT txid, vout FROM tracked_utxos WHERE wallet_profile_id = ?1")?;
        let rows = stmt.query_map(params![profile_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut s = HashSet::new();
        for r in rows {
            s.insert(r?);
        }
        s
    };

    let mut stmt = conn.prepare(
        "SELECT txid, height, time, raw_json FROM wallet_transactions_cache
         WHERE wallet_profile_id = ?1 ORDER BY height DESC, txid",
    )?;
    let rows = stmt.query_map(params![profile_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut out = Vec::new();
    for r in rows {
        let (txid, height, time, raw_json) = r?;
        let parsed: Option<serde_json::Value> = raw_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        let mut received: i64 = 0;
        let mut sent_outputs: i64 = 0;
        let mut first_addr = String::new();
        let mut spends_ours = false;

        if let Some(tx) = parsed.as_ref() {
            if let Some(outputs) = tx.get("outputs").and_then(|v| v.as_array()) {
                for o in outputs {
                    let value = o.get("value").and_then(|v| v.as_i64()).unwrap_or(0);
                    let addr = o
                        .get("address")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if our_addrs.contains(&addr) {
                        received += value;
                    } else {
                        sent_outputs += value;
                        if first_addr.is_empty() && !addr.is_empty() {
                            first_addr = addr;
                        }
                    }
                }
            }
            if let Some(inputs) = tx.get("inputs").and_then(|v| v.as_array()) {
                for i in inputs {
                    let prev = i.get("prevout");
                    let h = prev
                        .and_then(|p| p.get("hash"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let idx = prev
                        .and_then(|p| p.get("index"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(-1);
                    if !h.is_empty() && our_outpoints.contains(&(h.to_string(), idx)) {
                        spends_ours = true;
                    }
                }
            }
        }

        let (direction, value, address) = if spends_ours && sent_outputs > 0 {
            ("send", sent_outputs, first_addr)
        } else if received > 0 {
            ("receive", received, String::new())
        } else {
            ("other", 0, String::new())
        };

        out.push(serde_json::json!({
            "hash": txid,
            "value": value,
            "direction": direction,
            "address": address,
            "confirmed": height.is_some(),
            "height": height,
            "time": time,
        }));
    }
    Ok(out)
}

/// The current owner UTXO for a wallet-owned name, with its derivation path and
/// covenant — everything needed to spend it in a name action.
#[derive(Debug, Clone)]
pub struct NameCoin {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub address: String,
    pub branch: u32,
    pub child_index: u32,
    pub covenant_type: i64,
    pub covenant_json: Option<String>,
    /// The name's on-chain `height` (auction OPEN height) from name-state.
    pub name_height: Option<i64>,
}

/// Find the spendable owner UTXO for `name`, joining name-state → tracked UTXO →
/// derived address. `None` if we don't currently hold the name's coin.
pub fn get_name_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<NameCoin>, AppError> {
    let row = conn
        .query_row(
            "SELECT u.txid, u.vout, u.value_doos, u.address, d.branch, d.child_index,
                    u.covenant_type, u.covenant_json, n.height
             FROM tracked_name_states n
             JOIN tracked_utxos u
               ON u.wallet_profile_id = n.wallet_profile_id
              AND u.txid = n.owner_txid AND u.vout = n.owner_vout
              AND u.spent_by_txid IS NULL
             JOIN derived_addresses d
               ON d.wallet_profile_id = u.wallet_profile_id AND d.address = u.address
             WHERE n.wallet_profile_id = ?1 AND n.name = ?2",
            params![profile_id, name],
            |row| {
                Ok(NameCoin {
                    txid: row.get(0)?,
                    vout: row.get::<_, i64>(1)? as u32,
                    value: row.get::<_, i64>(2)? as u64,
                    address: row.get(3)?,
                    branch: row.get::<_, i64>(4)? as u32,
                    child_index: row.get::<_, i64>(5)? as u32,
                    covenant_type: row.get(6)?,
                    covenant_json: row.get(7)?,
                    name_height: row.get(8)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Extract `items[index]` (hex string, lowercased) from a stored covenant
/// JSON blob (`{"type":..,"action":..,"items":[...]}`, exactly as written by
/// `noncustodial::sync::covenant_json`). Returns `None` when the blob is
/// absent, unparseable, has no such item, or the item is empty. Shared parser
/// for every covenant-item lookup (name hash at items[0], BID blind at
/// items[3], …) so the JSON shape is decoded in exactly one place.
pub fn covenant_item_hex(covenant_json: Option<&str>, index: usize) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(covenant_json?).ok()?;
    let h = v.get("items")?.as_array()?.get(index)?.as_str()?;
    if h.is_empty() {
        return None;
    }
    Some(h.to_ascii_lowercase())
}

/// Extract the name-hash hex from a stored covenant JSON blob — every
/// Handshake name covenant carries the name hash as items[0].
fn covenant_name_hash_hex(covenant_json: Option<&str>) -> Option<String> {
    covenant_item_hex(covenant_json, 0)
}

/// Row-map a `tracked_utxos JOIN derived_addresses` result into a [`NameCoin`].
/// Shared by [`find_unspent_covenant_utxo`] and
/// [`find_unspent_covenant_utxos_by_name_hash`] so the column layout is
/// decoded in exactly one place.
fn row_to_name_coin(row: &rusqlite::Row) -> rusqlite::Result<NameCoin> {
    Ok(NameCoin {
        txid: row.get(0)?,
        vout: row.get::<_, i64>(1)? as u32,
        value: row.get::<_, i64>(2)? as u64,
        address: row.get(3)?,
        branch: row.get::<_, i64>(4)? as u32,
        child_index: row.get::<_, i64>(5)? as u32,
        covenant_type: row.get(6)?,
        covenant_json: row.get(7)?,
        name_height: row.get(8)?,
    })
}

/// Find the unspent tracked UTXO at `address` with a given covenant type whose
/// covenant belongs to `name` (matched by `name_hash_hex` against the covenant
/// items), with its derivation path. Used to locate our BID coin (to reveal)
/// or a losing REVEAL coin (to redeem).
///
/// The name-hash filter is what makes reveal/redeem safe when several names'
/// coins share one address (all pre-rotation bids sit on receive[0]): without
/// it, a lookup for name A could grab name B's coin and either get rejected by
/// the node or — if unnoticed until the reveal window closes — forfeit the
/// entire lockup.
///
/// Returns:
/// - `Ok(Some)` — exactly one coin matches `name_hash_hex`;
/// - `Ok(None)` — no coin for this name at this address (caller surfaces a
///   "sync first?" error naming the name);
/// - `Err` — MORE than one coin matches (e.g. a double bid on the same name at
///   one address). Picking arbitrarily could pair the coin with the wrong
///   stored nonce, so we refuse instead of guessing.
///
/// Documented fallback: a candidate whose `covenant_json` is NULL/unparseable
/// (possible only for degenerate/legacy rows — the sync path always stores
/// items for name covenants) is accepted ONLY when it is the single candidate
/// at this address+type, i.e. when the `bid_commitments.name` → address
/// association that produced `address` is unambiguous on its own.
pub fn find_unspent_covenant_utxo(
    conn: &rusqlite::Connection,
    profile_id: &str,
    address: &str,
    covenant_type: i64,
    name: &str,
    name_hash_hex: &str,
) -> Result<Option<NameCoin>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT u.txid, u.vout, u.value_doos, u.address, d.branch, d.child_index,
                u.covenant_type, u.covenant_json, NULL
         FROM tracked_utxos u
         JOIN derived_addresses d
           ON d.wallet_profile_id = u.wallet_profile_id AND d.address = u.address
         WHERE u.wallet_profile_id = ?1 AND u.address = ?2
           AND u.covenant_type = ?3 AND u.spent_by_txid IS NULL
         ORDER BY u.txid, u.vout",
    )?;
    let candidates: Vec<NameCoin> = stmt
        .query_map(
            params![profile_id, address, covenant_type],
            row_to_name_coin,
        )?
        .collect::<Result<_, _>>()?;

    let want = name_hash_hex.to_ascii_lowercase();
    let total = candidates.len();
    let mut matches: Vec<NameCoin> = Vec::new();
    let mut unknown: Vec<NameCoin> = Vec::new();
    for c in candidates {
        match covenant_name_hash_hex(c.covenant_json.as_deref()) {
            Some(h) if h == want => matches.push(c),
            Some(_) => {} // another name's coin — never touch it
            None => unknown.push(c),
        }
    }
    match matches.len() {
        1 => return Ok(matches.pop()),
        0 => {}
        n => {
            return Err(AppError::InvalidInput(format!(
                "{n} unspent coins (covenant type {covenant_type}) at {address} match \
                 name '{name}' — cannot pick one safely (multiple bids on the same \
                 name at one address?); resolve manually before spending"
            )))
        }
    }
    // Fallback: a lone candidate with no readable covenant items.
    if total == 1 && unknown.len() == 1 {
        return Ok(unknown.pop());
    }
    Ok(None)
}

/// Find ALL unspent tracked UTXOs of `covenant_type` across EVERY address of
/// `profile_id` whose covenant belongs to `name_hash_hex`.
///
/// Unlike [`find_unspent_covenant_utxo`] (address-scoped, used once we already
/// know the coin's address from a `bid_commitments` row), this scans the whole
/// profile. It exists for bid-commitment recovery: when the commitment row is
/// lost, the coin's address is exactly what's missing, so lookup can't be
/// address-scoped. Reuses [`covenant_name_hash_hex`] — the same Rust-side
/// covenant_json parser as the address-scoped lookup — so both stay in sync.
///
/// Callers must independently verify each candidate (e.g. by recomputing the
/// bid blind for a proposed value) before trusting it; multiple coins can
/// legitimately match the same name (e.g. two bids at different addresses).
pub fn find_unspent_covenant_utxos_by_name_hash(
    conn: &rusqlite::Connection,
    profile_id: &str,
    covenant_type: i64,
    name_hash_hex: &str,
) -> Result<Vec<NameCoin>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT u.txid, u.vout, u.value_doos, u.address, d.branch, d.child_index,
                u.covenant_type, u.covenant_json, NULL
         FROM tracked_utxos u
         JOIN derived_addresses d
           ON d.wallet_profile_id = u.wallet_profile_id AND d.address = u.address
         WHERE u.wallet_profile_id = ?1
           AND u.covenant_type = ?2 AND u.spent_by_txid IS NULL
         ORDER BY u.txid, u.vout",
    )?;
    let candidates: Vec<NameCoin> = stmt
        .query_map(params![profile_id, covenant_type], row_to_name_coin)?
        .collect::<Result<_, _>>()?;

    let want = name_hash_hex.to_ascii_lowercase();
    Ok(candidates
        .into_iter()
        .filter(|c| {
            covenant_name_hash_hex(c.covenant_json.as_deref()).as_deref() == Some(want.as_str())
        })
        .collect())
}

/// A distinct (nameHash, optional rawName) pair pulled from every unspent
/// name-covenant coin the wallet holds. Emitted by
/// [`list_unspent_wallet_name_hashes`] for node-only owned-name discovery: the
/// caller resolves each nameHash → name via `getnamebyhash`, falling back to
/// `raw_name_hex` when the node can't resolve the hash (or the wrapper isn't
/// available). Duplicates are collapsed — the wallet may hold several coins for
/// the same name (OPEN + BID + REVEAL + owner), but only one `getnameinfo` per
/// name is worth doing per sync pass.
#[derive(Debug, Clone)]
pub struct WalletNameHash {
    pub name_hash_hex: String,
    /// Hex-encoded rawName from covenant items[2], present only for OPEN, BID,
    /// and FINALIZE covenants (see `noncustodial::covenants`). REVEAL/REDEEM/
    /// REGISTER/UPDATE/RENEW/TRANSFER carry only the nameHash.
    pub raw_name_hex: Option<String>,
}

/// List every distinct nameHash referenced by an unspent name-covenant coin in
/// the profile's `tracked_utxos`, together with the coin's `rawName` (items[2])
/// when the covenant type carries it (OPEN=2, BID=3, FINALIZE=10). Used by the
/// node-only owned-name discovery path: each hash is a name the wallet either
/// has an active auction position in OR currently owns.
pub fn list_unspent_wallet_name_hashes(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<WalletNameHash>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT u.covenant_type, u.covenant_json
         FROM tracked_utxos u
         WHERE u.wallet_profile_id = ?1
           AND u.spend_class IN ('name_control', 'name_lockup')
           AND u.spent_by_txid IS NULL",
    )?;
    let rows = stmt.query_map(params![profile_id], |r| {
        let cov_type: i64 = r.get(0)?;
        let cov_json: Option<String> = r.get(1)?;
        Ok((cov_type, cov_json))
    })?;

    let mut seen: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for row in rows {
        let (cov_type, cov_json) = row?;
        let name_hash = match covenant_name_hash_hex(cov_json.as_deref()) {
            Some(h) => h,
            None => continue,
        };
        // items[2] carries rawName ONLY for OPEN/BID/FINALIZE — see
        // `noncustodial::covenants`. For every other covenant type items[2] is
        // something else (a nonce for REVEAL, a resource blob for REGISTER/UPDATE,
        // …) and must NOT be read as a name.
        let raw = if cov_type as u8 == crate::noncustodial::sync::COV_OPEN
            || cov_type as u8 == crate::noncustodial::sync::COV_BID
            || cov_type as u8 == crate::noncustodial::sync::COV_FINALIZE
        {
            covenant_item_hex(cov_json.as_deref(), 2)
        } else {
            None
        };
        // Keep the first non-None rawName seen for a given hash.
        seen.entry(name_hash)
            .and_modify(|existing| {
                if existing.is_none() && raw.is_some() {
                    *existing = raw.clone();
                }
            })
            .or_insert(raw);
    }

    Ok(seen
        .into_iter()
        .map(|(name_hash_hex, raw_name_hex)| WalletNameHash {
            name_hash_hex,
            raw_name_hex,
        })
        .collect())
}

// --- Bid commitments (secret blind/nonce; backend-only) --------------------

/// A persisted bid commitment. `nonce_hex`/`blind_hex` are SECRET wallet state
/// and must never be returned to the frontend.
#[derive(Debug, Clone)]
pub struct BidCommitmentRow {
    pub name: String,
    pub name_hash_hex: String,
    pub address: String,
    pub branch: i64,
    pub child_index: i64,
    pub bid_value_doos: i64,
    pub lockup_value_doos: i64,
    pub nonce_hex: String,
    pub blind_hex: String,
    pub bid_txid: Option<String>,
    pub reveal_txid: Option<String>,
    /// Estimated height at which the reveal window closes (`start +
    /// (treeInterval + 1) + biddingPeriod + revealPeriod`), when derivable —
    /// see `014_reveal_end_height.sql`. `None` for commitments recovered via
    /// `recover_bid_commitment` or written before this column existed.
    pub reveal_end_height: Option<i64>,
}

/// Insert a bid commitment row. Errors (rather than silently no-op'ing) when a
/// row with the same `(wallet_profile_id, name, blind_hex)` already exists.
///
/// I2 fix: this used to be `ON CONFLICT ... DO NOTHING`, so a re-bid that
/// happened to recompute the same blind (e.g. a race replaying the same
/// value/address) would silently drop the new commitment row while the
/// caller went on to build and persist the tx draft anyway — a direct path
/// to an unrevealable bid (the on-chain BID coin exists but its true
/// value/nonce were never (re-)persisted). Callers that build a tx draft from
/// this MUST treat an `Err` here as fatal and abort before persisting the
/// draft (see `commands::names::build_bid_draft`) — never build-then-ignore.
///
/// `commands::bids::recover_bid_commitment` is the one legitimate idempotent
/// caller (re-running recovery for an already-recovered bid should succeed,
/// not error) — it checks [`bid_commitment_exists`] first and skips the
/// insert entirely rather than relying on this function's conflict handling.
#[allow(clippy::too_many_arguments)]
pub fn insert_bid_commitment(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    name_hash_hex: &str,
    address: &str,
    branch: i64,
    child_index: i64,
    bid_value: i64,
    lockup: i64,
    nonce_hex: &str,
    blind_hex: &str,
) -> Result<(), AppError> {
    let changed = conn.execute(
        "INSERT INTO bid_commitments
            (wallet_profile_id, name, name_hash_hex, address, branch, child_index,
             bid_value_doos, lockup_value_doos, nonce_hex, blind_hex)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(wallet_profile_id, name, blind_hex) DO NOTHING",
        params![
            profile_id,
            name,
            name_hash_hex,
            address,
            branch,
            child_index,
            bid_value,
            lockup,
            nonce_hex,
            blind_hex
        ],
    )?;
    if changed == 0 {
        return Err(AppError::InvalidInput(format!(
            "a bid commitment for '{name}' with this exact value/address already exists \
             — refusing to silently drop it (that would leave an unrevealable bid)"
        )));
    }
    Ok(())
}

/// Whether a bid commitment with this exact `(wallet_profile_id, name,
/// blind_hex)` key already exists — the idempotency check
/// `recover_bid_commitment` uses to make re-running recovery for an
/// already-recovered bid a safe no-op instead of hitting
/// [`insert_bid_commitment`]'s honest conflict error.
pub fn bid_commitment_exists(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    blind_hex: &str,
) -> Result<bool, AppError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM bid_commitments
         WHERE wallet_profile_id = ?1 AND name = ?2 AND blind_hex = ?3)",
        params![profile_id, name, blind_hex],
        |r| r.get(0),
    )?;
    Ok(exists)
}

const BID_COLS: &str = "name, name_hash_hex, address, branch, child_index, \
     bid_value_doos, lockup_value_doos, nonce_hex, blind_hex, bid_txid, reveal_txid, \
     reveal_end_height";

fn row_to_bid(row: &rusqlite::Row) -> rusqlite::Result<BidCommitmentRow> {
    Ok(BidCommitmentRow {
        name: row.get(0)?,
        name_hash_hex: row.get(1)?,
        address: row.get(2)?,
        branch: row.get(3)?,
        child_index: row.get(4)?,
        bid_value_doos: row.get(5)?,
        lockup_value_doos: row.get(6)?,
        nonce_hex: row.get(7)?,
        blind_hex: row.get(8)?,
        bid_txid: row.get(9)?,
        reveal_txid: row.get(10)?,
        reveal_end_height: row.get(11)?,
    })
}

/// Persist the reveal-window-close height estimate for the bid commitment
/// just inserted by `build_bid_draft` (the only caller with a live auction
/// `start` height to compute it from — see `014_reveal_end_height.sql`).
pub fn set_reveal_end_height(
    conn: &rusqlite::Connection,
    profile_id: &str,
    blind_hex: &str,
    reveal_end_height: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE bid_commitments SET reveal_end_height = ?3
         WHERE wallet_profile_id = ?1 AND blind_hex = ?2",
        params![profile_id, blind_hex, reveal_end_height],
    )?;
    Ok(())
}

/// Every un-revealed bid commitment across ALL profiles with a known
/// reveal-window-close estimate — the deadline scanner's input. Un-revealed =
/// `reveal_txid IS NULL` (once revealed there is no more reveal deadline).
pub fn list_pending_reveal_deadlines(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, String, i64)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT wallet_profile_id, name, reveal_end_height FROM bid_commitments
         WHERE reveal_txid IS NULL AND reveal_end_height IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// The most recent bid commitment for a name (used to reveal).
pub fn get_bid_commitment(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<BidCommitmentRow>, AppError> {
    let sql = format!(
        "SELECT {BID_COLS} FROM bid_commitments
         WHERE wallet_profile_id = ?1 AND name = ?2
         ORDER BY created_at DESC LIMIT 1"
    );
    Ok(conn
        .query_row(&sql, params![profile_id, name], row_to_bid)
        .optional()?)
}

pub fn list_bid_commitments(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<BidCommitmentRow>, AppError> {
    let sql = format!(
        "SELECT {BID_COLS} FROM bid_commitments
         WHERE wallet_profile_id = ?1 ORDER BY created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id], row_to_bid)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Names this profile currently holds an *auction position* in — opened, bid,
/// or revealed, but not (yet) owned. Complements [`read_cached_names`] /
/// [`read_owned_names_explorer`] (owned-only) so a name like an in-progress
/// `.namehold` open can surface on the Auctions view before it's won.
///
/// Union of two sources, deduplicated and sorted (`BTreeSet`):
///   - [`list_tx_drafts`] filtered to `action IN (open, bid, reveal)` AND
///     `status IN (signed, broadcast_pending, broadcasted, confirmed)` — the
///     name comes from `summary_json.name` (drafts have no `name` column, see
///     [`has_pending_draft_for_name`] for the same parse idiom). `draft` status
///     is excluded (never queued to chain, could vanish); `dropped`/`failed`
///     are excluded (terminal, never landed and never will).
///   - [`list_bid_commitments`] — every bid/reveal commitment's `name`, which
///     covers a recovered bid whose draft was pruned or never existed locally.
///
/// Names that are already OWNED (an unspent owner coin — see [`get_name_coin`])
/// are excluded even if an old bid commitment still references them: once
/// owned, the name belongs in "Owned Names", not "in progress". Pure DB reads,
/// no network calls.
pub fn auction_position_names(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<String>, AppError> {
    const RELEVANT_ACTIONS: [&str; 3] = ["open", "bid", "reveal"];
    const IN_FLIGHT_STATUSES: [&str; 4] =
        ["signed", "broadcast_pending", "broadcasted", "confirmed"];

    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for draft in list_tx_drafts(conn, profile_id)? {
        if !RELEVANT_ACTIONS.contains(&draft.action.as_str())
            || !IN_FLIGHT_STATUSES.contains(&draft.status.as_str())
        {
            continue;
        }
        if let Some(name) = draft.summary.get("name").and_then(|n| n.as_str()) {
            names.insert(name.to_string());
        }
    }

    for bid in list_bid_commitments(conn, profile_id)? {
        names.insert(bid.name);
    }

    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if get_name_coin(conn, profile_id, &name)?.is_none() {
            out.push(name);
        }
    }
    Ok(out)
}

pub fn set_bid_txid(
    conn: &rusqlite::Connection,
    profile_id: &str,
    blind_hex: &str,
    txid: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE bid_commitments SET bid_txid = ?3
         WHERE wallet_profile_id = ?1 AND blind_hex = ?2",
        params![profile_id, blind_hex, txid],
    )?;
    Ok(())
}

pub fn set_bid_reveal_txid(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    txid: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE bid_commitments SET reveal_txid = ?3
         WHERE wallet_profile_id = ?1 AND name = ?2",
        params![profile_id, name, txid],
    )?;
    Ok(())
}

#[cfg(test)]
mod noncustodial_query_tests {
    use super::*;
    use crate::noncustodial::sync::{cache_transaction, upsert_name_state};
    use rusqlite::Connection;

    /// Fresh in-memory DB with all migrations applied (001 settings + 006-009).
    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    fn seed_profile(conn: &Connection, id: &str) {
        insert_wallet_profile(
            conn,
            id,
            "Primary",
            "mnemonic_hot",
            "regtest",
            "xpubFAKE",
            0,
            false,
        )
        .unwrap();
    }

    #[test]
    fn profile_crud_and_active_selection() {
        let conn = db();
        seed_profile(&conn, "p1");

        // Active id starts empty; the profile is therefore not active.
        assert_eq!(get_active_profile_id(&conn).unwrap(), "");
        let p = get_wallet_profile(&conn, "p1").unwrap().unwrap();
        assert_eq!(p.label, "Primary");
        assert!(!p.active);
        assert!(!p.watch_only);
        assert_eq!(list_wallet_profiles(&conn).unwrap().len(), 1);

        // Activate and re-read.
        set_active_profile(&conn, "p1").unwrap();
        assert_eq!(get_active_profile_id(&conn).unwrap(), "p1");
        assert!(get_wallet_profile(&conn, "p1").unwrap().unwrap().active);

        // Receive + sync updates persist.
        update_profile_receive(&conn, "p1", "rs1qaddr", 20).unwrap();
        update_profile_sync(&conn, "p1", 12345).unwrap();
        let p = get_wallet_profile(&conn, "p1").unwrap().unwrap();
        assert_eq!(p.receive_address.as_deref(), Some("rs1qaddr"));
        assert_eq!(p.receive_depth, 20);
        assert_eq!(p.last_synced_height, Some(12345));

        // Missing profile -> None.
        assert!(get_wallet_profile(&conn, "nope").unwrap().is_none());
    }

    #[test]
    fn secret_blob_round_trips_and_watch_only_has_none() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_wallet_secret(&conn, "p1", &[0xde, 0xad, 0xbe, 0xef], "argon2id", "fp123").unwrap();
        assert_eq!(
            get_wallet_secret_meta(&conn, "p1").unwrap(),
            Some((vec![0xde, 0xad, 0xbe, 0xef], "argon2id".to_string()))
        );
        // A profile with no secret row returns None (e.g. watch-only).
        insert_wallet_profile(
            &conn,
            "p2",
            "Watch",
            "watch_only_xpub",
            "regtest",
            "xpubW",
            0,
            true,
        )
        .unwrap();
        assert_eq!(get_wallet_secret_meta(&conn, "p2").unwrap(), None);
        // No-passphrase wallets are marked kdf='none'.
        insert_wallet_secret(&conn, "p2", &[1, 2, 3], "none", "fp2").unwrap();
        assert_eq!(
            get_wallet_secret_meta(&conn, "p2").unwrap().unwrap().1,
            "none"
        );
    }

    #[test]
    fn draft_lifecycle_draft_signed_broadcasted() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_tx_draft(
            &conn,
            "d1",
            "p1",
            "send_hns",
            "",
            r#"{"toAddress":"rs1qdest","amountDoos":1000000}"#,
            r#"{"action":"send_hns","sendTotalDoos":1000000}"#,
        )
        .unwrap();

        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "draft");
        assert!(d.signed_tx_hex.is_none());
        // Summary parses into a JSON value for the frontend.
        assert!(d.to_summary().summary.is_object());

        update_tx_draft_signed(
            &conn,
            "d1",
            "0011aabb",
            r#"{"action":"send_hns","txid":"tx1"}"#,
        )
        .unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "signed");
        assert_eq!(d.signed_tx_hex.as_deref(), Some("0011aabb"));

        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("txidABC")).unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "broadcasted");
        assert_eq!(d.txid.as_deref(), Some("txidABC"));

        let list = list_tx_drafts(&conn, "p1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "d1");
    }

    #[test]
    fn profile_addresses_listed_in_branch_order() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Insert two derived addresses on different branches.
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,1,0,'rs1qchange','0014','02'),
                    ('p1',0,0,0,'rs1qrecv','0014','02')",
            [],
        )
        .unwrap();
        let addrs = get_profile_addresses(&conn, "p1").unwrap();
        // Ordered by branch, child_index: receive (branch 0) first.
        assert_eq!(
            addrs,
            vec!["rs1qrecv".to_string(), "rs1qchange".to_string()]
        );
    }

    fn insert_utxo(conn: &Connection, txid: &str, vout: i64, value: i64, class: &str, cov: i64) {
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES (?1, ?2, 'p1', 'rs1qrecv', '0014', ?3, ?4, ?5)",
            params![txid, vout, value, cov, class],
        )
        .unwrap();
    }

    #[test]
    fn cached_balance_maps_liquid_and_locked() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_utxo(&conn, "aa", 0, 1_000_000, "liquid_hns", 0);
        insert_utxo(&conn, "bb", 0, 3_000_000, "name_control", 6);
        insert_utxo(&conn, "cc", 0, 2_000_000, "name_lockup", 3);
        let bal = read_cached_balance(&conn, "p1").unwrap();
        assert_eq!(bal["confirmed"], 1_000_000);
        assert_eq!(bal["locked_confirmed"], 5_000_000); // control + lockup
        assert_eq!(bal["unconfirmed"], 0);
    }

    #[test]
    fn cached_names_only_returns_owned() {
        let conn = db();
        seed_profile(&conn, "p1");
        // We hold the UTXO that owns "mine" but not the one owning "theirs".
        insert_utxo(&conn, "owntx", 0, 2_000_000, "name_control", 6);
        upsert_name_state(
            &conn,
            "p1",
            "mine",
            &serde_json::json!({"info":{"name":"mine","nameHash":"h1","state":"CLOSED","owner":{"hash":"owntx","index":0}}}),
        )
        .unwrap();
        upsert_name_state(
            &conn,
            "p1",
            "theirs",
            &serde_json::json!({"info":{"name":"theirs","nameHash":"h2","state":"CLOSED","owner":{"hash":"othertx","index":4}}}),
        )
        .unwrap();

        let names = read_cached_names(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0]["name"], "mine");
        assert_eq!(names[0]["owner"]["hash"], "owntx");

        let tracked = list_tracked_name_names(&conn, "p1").unwrap();
        assert_eq!(tracked.len(), 2); // both tracked, only one owned
    }

    #[test]
    fn cached_transactions_classify_receive_and_send() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,0,'rs1qmine','0014','02')",
            [],
        )
        .unwrap();
        // A receive: an output pays our address; no input spends our coin.
        cache_transaction(
            &conn,
            "p1",
            "rxtx",
            Some(100),
            None,
            r#"{"outputs":[{"value":500000,"address":"rs1qmine"}],"inputs":[]}"#,
        )
        .unwrap();
        // A send: spends our tracked UTXO, pays a foreign address.
        insert_utxo(&conn, "prevtx", 1, 700_000, "liquid_hns", 0);
        cache_transaction(
            &conn,
            "p1",
            "sendtx",
            Some(101),
            None,
            r#"{"outputs":[{"value":300000,"address":"rs1qother"}],"inputs":[{"prevout":{"hash":"prevtx","index":1}}]}"#,
        )
        .unwrap();

        let txs = read_cached_transactions(&conn, "p1").unwrap();
        let by_hash = |h: &str| txs.iter().find(|t| t["hash"] == h).unwrap().clone();
        let rx = by_hash("rxtx");
        assert_eq!(rx["direction"], "receive");
        assert_eq!(rx["value"], 500000);
        assert_eq!(rx["confirmed"], true);
        let sx = by_hash("sendtx");
        assert_eq!(sx["direction"], "send");
        assert_eq!(sx["value"], 300000);
        assert_eq!(sx["address"], "rs1qother");
    }

    // ── Additional coverage tests ────────────────────────────────────────

    #[test]
    fn get_dashboard_stats_returns_counts() {
        let conn = db();
        // Seed some assets with different statuses.
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES ('aaa','finalized_owned',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES ('bbb','not_started',0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES ('ccc','finalized_owned',0)",
            [],
        )
        .unwrap();

        let stats = get_dashboard_stats(&conn).unwrap();
        assert_eq!(stats["total"], 3);
        assert_eq!(stats["staked"], 1);
        assert_eq!(stats["unstaked"], 2);
        assert_eq!(stats["status_counts"]["finalized_owned"], 2);
        assert_eq!(stats["status_counts"]["not_started"], 1);
        assert!(stats["recent_audit"].as_array().unwrap().is_empty());
    }

    #[test]
    fn get_known_wallet_addresses_deduplicates() {
        let conn = db();
        insert_wallet_snapshot(&conn, "w", 100, Some("rs1qaaa"), 1, None).unwrap();
        insert_wallet_snapshot(&conn, "w", 200, Some("rs1qbbb"), 2, None).unwrap();
        insert_wallet_snapshot(&conn, "w", 300, Some("rs1qaaa"), 3, None).unwrap();
        // Empty / NULL addresses should be excluded.
        insert_wallet_snapshot(&conn, "w", 400, None, 4, None).unwrap();
        insert_wallet_snapshot(&conn, "w", 500, Some(""), 5, None).unwrap();

        let addrs = get_known_wallet_addresses(&conn, 10).unwrap();
        // Distinct, newest first: rs1qaaa (id 4th row) then rs1qbbb (2nd row).
        assert_eq!(addrs, vec!["rs1qaaa".to_string(), "rs1qbbb".to_string()]);
    }

    #[test]
    fn replace_and_get_wallet_addresses() {
        let conn = db();
        let inserted = replace_wallet_addresses(
            &conn,
            "wallet1",
            &["rs1qone".into(), "rs1qtwo".into(), "  ".into()],
        )
        .unwrap();
        // Blank address is skipped.
        assert_eq!(inserted, 2);

        let addrs = get_wallet_addresses_for_wallet(&conn, "wallet1", 10).unwrap();
        assert_eq!(addrs.len(), 2);
        assert!(addrs.contains(&"rs1qone".to_string()));
        assert!(addrs.contains(&"rs1qtwo".to_string()));

        // Re-upserting same addresses should not error (ON CONFLICT DO UPDATE).
        let inserted2 = replace_wallet_addresses(&conn, "wallet1", &["rs1qone".into()]).unwrap();
        assert_eq!(inserted2, 1);
        let addrs2 = get_wallet_addresses_for_wallet(&conn, "wallet1", 10).unwrap();
        assert_eq!(addrs2.len(), 2);
    }

    #[test]
    fn get_inventory_tlds_returns_sorted() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('zzz')", [])
            .unwrap();
        conn.execute("INSERT INTO assets (tld) VALUES ('aaa')", [])
            .unwrap();
        conn.execute("INSERT INTO assets (tld) VALUES ('mmm')", [])
            .unwrap();

        let tlds = get_inventory_tlds(&conn).unwrap();
        assert_eq!(tlds, vec!["aaa", "mmm", "zzz"]);
    }

    #[test]
    fn get_assets_by_tlds_returns_matches() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('alpha','finalized_owned')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('beta','not_started')",
            [],
        )
        .unwrap();

        let assets =
            get_assets_by_tlds(&conn, &["beta".into(), "missing".into(), "alpha".into()]).unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].tld, "beta");
        assert_eq!(assets[1].tld, "alpha");
    }

    #[test]
    fn list_repair_candidates_excludes_recently_synced_and_orders_oldest_first() {
        let conn = db();
        seed_profile(&conn, "p1");

        // never synced (NULL) — highest priority
        conn.execute("INSERT INTO assets (tld) VALUES ('never')", [])
            .unwrap();
        // synced long ago — eligible
        conn.execute(
            "INSERT INTO assets (tld, last_synced_at) VALUES ('old', datetime('now','-3 days'))",
            [],
        )
        .unwrap();
        // synced just now — within the 12h window, excluded
        conn.execute(
            "INSERT INTO assets (tld, last_synced_at) VALUES ('fresh', datetime('now'))",
            [],
        )
        .unwrap();

        let got = list_repair_candidates(&conn, "p1", 150, 12).unwrap();
        // 'fresh' is excluded; NULL sorts before the aged timestamp.
        assert_eq!(got, vec!["never".to_string(), "old".to_string()]);
    }

    #[test]
    fn list_repair_candidates_unions_tracked_names_and_respects_limit() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute("INSERT INTO assets (tld) VALUES ('inv')", [])
            .unwrap();
        // A tracked name not in `assets` must appear as a candidate...
        conn.execute(
            "INSERT INTO tracked_name_states (wallet_profile_id, name, name_hash_hex, state)
             VALUES ('p1','tracked','hh','CLOSED')",
            [],
        )
        .unwrap();
        // ...but one that IS in assets must not be duplicated.
        conn.execute(
            "INSERT INTO tracked_name_states (wallet_profile_id, name, name_hash_hex, state)
             VALUES ('p1','inv','hh2','CLOSED')",
            [],
        )
        .unwrap();

        let mut got = list_repair_candidates(&conn, "p1", 150, 12).unwrap();
        got.sort();
        assert_eq!(got, vec!["inv".to_string(), "tracked".to_string()]);

        // LIMIT is honored.
        let limited = list_repair_candidates(&conn, "p1", 1, 12).unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn mark_asset_finalized_owned_advances_status_but_skips_staked() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('own','not_started')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('stk','do_not_touch_staked')",
            [],
        )
        .unwrap();

        mark_asset_finalized_owned(&conn, "own", Some("CLOSED")).unwrap();
        mark_asset_finalized_owned(&conn, "stk", Some("CLOSED")).unwrap();

        let (status, name_state, synced): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT status, name_state, last_synced_at FROM assets WHERE tld='own'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "finalized_owned");
        assert_eq!(name_state.as_deref(), Some("CLOSED"));
        assert!(synced.is_some());

        // Staked row is untouched.
        let staked_status: String = conn
            .query_row("SELECT status FROM assets WHERE tld='stk'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(staked_status, "do_not_touch_staked");
    }

    #[test]
    fn touch_asset_synced_stamps_timestamp_without_changing_status() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('x','not_started')",
            [],
        )
        .unwrap();
        touch_asset_synced(&conn, "x").unwrap();
        let (status, synced): (String, Option<String>) = conn
            .query_row(
                "SELECT status, last_synced_at FROM assets WHERE tld='x'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "not_started");
        assert!(synced.is_some());
    }

    #[test]
    fn list_recently_synced_tlds_returns_only_fresh_rows() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status, last_synced_at) VALUES ('fresh','not_started', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, last_synced_at) VALUES ('old','not_started', datetime('now','-3 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('never','not_started')",
            [],
        )
        .unwrap();
        let got = list_recently_synced_tlds(&conn, 12).unwrap();
        assert_eq!(
            got,
            vec!["fresh".to_string()],
            "only the within-12h row is memoized"
        );
    }

    #[test]
    fn update_profile_change_depth_bumps() {
        let conn = db();
        seed_profile(&conn, "p1");
        update_profile_change_depth(&conn, "p1", 5).unwrap();
        let p = get_wallet_profile(&conn, "p1").unwrap().unwrap();
        assert_eq!(p.change_depth, 5);

        // Should only increase, never decrease.
        update_profile_change_depth(&conn, "p1", 3).unwrap();
        let p2 = get_wallet_profile(&conn, "p1").unwrap().unwrap();
        assert_eq!(p2.change_depth, 5);
    }

    #[test]
    fn tx_draft_confirmation_and_age() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_tx_draft(&conn, "d1", "p1", "send_hns", "", "{}", "{}").unwrap();

        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("txid1")).unwrap();
        update_tx_draft_confirmation(&conn, "d1", 12345, None).unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "confirmed");
        assert_eq!(d.confirmation_height, Some(12345));
        assert_eq!(
            d.txid.as_deref(),
            Some("txid1"),
            "existing txid preserved when None passed"
        );

        let age = draft_age_secs(&conn, "d1").unwrap();
        assert!(age >= 0);

        let updated_age = draft_updated_age_secs(&conn, "d1").unwrap();
        assert!(updated_age >= 0);
    }

    #[test]
    fn update_tx_draft_confirmation_can_set_txid() {
        // Used when promoting a `broadcast_pending` draft straight to
        // `confirmed` in one step: the draft has no DB txid yet (only a
        // locally-computed one), so the confirmation write must be able to
        // persist it too.
        let conn = db();
        seed_profile(&conn, "p1");
        insert_tx_draft(&conn, "d1", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d1", "broadcast_pending", None, None).unwrap();

        update_tx_draft_confirmation(&conn, "d1", 999, Some("computed_txid")).unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "confirmed");
        assert_eq!(d.confirmation_height, Some(999));
        assert_eq!(d.txid.as_deref(), Some("computed_txid"));
    }

    #[test]
    fn revert_tx_draft_to_broadcasted_clears_height_and_sets_note() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_tx_draft(&conn, "d1", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("txid1")).unwrap();
        update_tx_draft_confirmation(&conn, "d1", 12345, None).unwrap();

        revert_tx_draft_to_broadcasted(&conn, "d1", "reorg: tx no longer found at recorded height")
            .unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "broadcasted");
        assert_eq!(d.confirmation_height, None);
        assert_eq!(
            d.txid.as_deref(),
            Some("txid1"),
            "txid must survive the revert"
        );
        assert!(d.error_message.unwrap().contains("reorg"));
    }

    #[test]
    fn list_drafts_awaiting_confirmation_filters() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Draft status → should NOT appear.
        insert_tx_draft(&conn, "d_draft", "p1", "send_hns", "", "{}", "{}").unwrap();
        // Broadcasted with txid → should appear.
        insert_tx_draft(&conn, "d_bcast", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d_bcast", "broadcasted", None, Some("txid1")).unwrap();
        // Confirmed with txid, shallow (well within finality depth) → should appear.
        insert_tx_draft(&conn, "d_conf", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d_conf", "confirmed", None, Some("txid2")).unwrap();
        update_tx_draft_confirmation(&conn, "d_conf", 990, None).unwrap();
        // Broadcasted but NO txid → should NOT appear.
        insert_tx_draft(&conn, "d_notx", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d_notx", "broadcasted", None, None).unwrap();
        // broadcast_pending (no txid yet — it's only known locally) → should appear.
        insert_tx_draft(&conn, "d_pending", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d_pending", "broadcast_pending", None, None).unwrap();
        // Confirmed, deeply buried (>= finality depth) → should NOT appear.
        insert_tx_draft(&conn, "d_buried", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d_buried", "confirmed", None, Some("txid3")).unwrap();
        update_tx_draft_confirmation(&conn, "d_buried", 100, None).unwrap();

        // tip = 1000, finality depth = 12: d_conf has 1000-990+1=11 confs (< 12,
        // still shallow); d_buried has 1000-100+1=901 confs (>= 12, buried).
        let awaiting = list_drafts_awaiting_confirmation(&conn, "p1", 1000, 12).unwrap();
        let ids: Vec<&str> = awaiting.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"d_bcast"));
        assert!(ids.contains(&"d_conf"));
        assert!(ids.contains(&"d_pending"));
        assert!(!ids.contains(&"d_draft"));
        assert!(!ids.contains(&"d_notx"));
        assert!(
            !ids.contains(&"d_buried"),
            "deeply-buried confirmed draft must stop being polled"
        );
    }

    #[test]
    fn has_pending_bid_draft_for_name_matches_in_flight_bid_drafts() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "alpha").unwrap());

        // A `draft`-status bid for "alpha" counts as pending.
        insert_tx_draft(
            &conn,
            "d1",
            "p1",
            "bid",
            "",
            "{}",
            r#"{"action":"bid","name":"alpha"}"#,
        )
        .unwrap();
        assert!(has_pending_bid_draft_for_name(&conn, "p1", "alpha").unwrap());
        // A different name is unaffected.
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "beta").unwrap());

        // Once dropped/failed, it no longer blocks a retry.
        update_tx_draft_status(&conn, "d1", "dropped", None, None).unwrap();
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "alpha").unwrap());

        // signed / broadcast_pending / broadcasted all still count as pending.
        for status in ["signed", "broadcast_pending", "broadcasted"] {
            update_tx_draft_status(&conn, "d1", status, None, None).unwrap();
            assert!(
                has_pending_bid_draft_for_name(&conn, "p1", "alpha").unwrap(),
                "status {status} should still be considered pending"
            );
        }

        // A non-bid action for the same name never counts, even in `draft`.
        update_tx_draft_status(&conn, "d1", "draft", None, None).unwrap();
        insert_tx_draft(
            &conn,
            "d2",
            "p1",
            "reveal",
            "",
            "{}",
            r#"{"action":"reveal","name":"gamma"}"#,
        )
        .unwrap();
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "gamma").unwrap());
    }

    #[test]
    fn has_pending_bid_draft_for_name_matches_batch_bid_namelist() {
        let conn = db();
        seed_profile(&conn, "p1");

        // A batch-bid draft covers many names via `nameList` (display `name`
        // is just "alpha + 1 more"). The guard must recognise EVERY member,
        // not only the first — otherwise a follow-up single bid on "beta"
        // would slip past while the batch is still in flight.
        insert_tx_draft(
            &conn,
            "db1",
            "p1",
            "batch-bid",
            "",
            "{}",
            r#"{"action":"batch-bid","name":"alpha + 1 more","nameList":["alpha","beta"]}"#,
        )
        .unwrap();

        assert!(has_pending_bid_draft_for_name(&conn, "p1", "alpha").unwrap());
        assert!(has_pending_bid_draft_for_name(&conn, "p1", "beta").unwrap());
        // A name NOT in the batch is unaffected.
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "gamma").unwrap());

        // Dropping the batch draft frees all its names for a retry.
        update_tx_draft_status(&conn, "db1", "dropped", None, None).unwrap();
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "alpha").unwrap());
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "beta").unwrap());

        // Backward compat: a legacy single-name "bid" draft (no nameList) is
        // still matched via its `name` field.
        insert_tx_draft(
            &conn,
            "db2",
            "p1",
            "bid",
            "",
            "{}",
            r#"{"action":"bid","name":"delta"}"#,
        )
        .unwrap();
        assert!(has_pending_bid_draft_for_name(&conn, "p1", "delta").unwrap());
    }

    /// The generic [`has_pending_draft_for_name`] behind the bid wrapper works
    /// for any action — exercised here with `"open"` (Task 1's double-open
    /// guard), independently of `has_pending_bid_draft_for_name`'s own
    /// regression coverage above.
    #[test]
    fn has_pending_draft_for_name_generalizes_to_open_action() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(!has_pending_draft_for_name(&conn, "p1", "open", "alpha").unwrap());

        insert_tx_draft(
            &conn,
            "d1",
            "p1",
            "open",
            "",
            "{}",
            r#"{"action":"open","name":"alpha"}"#,
        )
        .unwrap();
        assert!(has_pending_draft_for_name(&conn, "p1", "open", "alpha").unwrap());
        // A different action for the same name never counts.
        assert!(!has_pending_draft_for_name(&conn, "p1", "bid", "alpha").unwrap());
        // A different name is unaffected.
        assert!(!has_pending_draft_for_name(&conn, "p1", "open", "beta").unwrap());

        update_tx_draft_status(&conn, "d1", "dropped", None, None).unwrap();
        assert!(!has_pending_draft_for_name(&conn, "p1", "open", "alpha").unwrap());
    }

    #[test]
    fn upsert_and_read_owned_names_explorer() {
        let conn = db();
        seed_profile(&conn, "p1");

        let name = crate::hsd::types::HsdName {
            name: "testname".into(),
            name_hash: Some("aabb".into()),
            state: Some("CLOSED".into()),
            height: Some(100),
            renewal: Some(200),
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        upsert_owned_name(&conn, "p1", &name, "txid1", 0, "rs1qaddr1").unwrap();

        let names = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0]["name"], "testname");
        assert_eq!(names[0]["owner"]["hash"], "txid1");
        assert_eq!(names[0]["owner_address"], "rs1qaddr1");

        // Upsert again (update path).
        let name2 = crate::hsd::types::HsdName {
            name: "testname".into(),
            name_hash: Some("ccdd".into()),
            state: Some("REVOKED".into()),
            height: Some(100),
            renewal: Some(300),
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        upsert_owned_name(&conn, "p1", &name2, "txid2", 1, "rs1qaddr2").unwrap();
        let names2 = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names2.len(), 1);
        assert_eq!(names2[0]["owner"]["hash"], "txid2");
    }

    #[test]
    fn upsert_owned_name_updates_owner_address_on_conflict() {
        let conn = db();
        seed_profile(&conn, "p1");

        let name = crate::hsd::types::HsdName {
            name: "conflictname".into(),
            name_hash: Some("aabb".into()),
            state: Some("CLOSED".into()),
            height: Some(100),
            renewal: Some(200),
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        upsert_owned_name(&conn, "p1", &name, "txid1", 0, "rs1qoriginal").unwrap();

        let names = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names[0]["owner_address"], "rs1qoriginal");

        // Repeat the upsert for the same (profile, name) with a new address —
        // the ON CONFLICT path should overwrite owner_address, not append/ignore it.
        upsert_owned_name(&conn, "p1", &name, "txid1", 0, "rs1qupdated").unwrap();

        let names_after = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names_after.len(), 1);
        assert_eq!(names_after[0]["owner_address"], "rs1qupdated");
    }

    #[test]
    fn get_name_coin_returns_none_for_missing() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(get_name_coin(&conn, "p1", "nonexistent").unwrap().is_none());
    }

    #[test]
    fn find_unspent_covenant_utxo_returns_none_for_missing() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(
            find_unspent_covenant_utxo(&conn, "p1", "rs1qnone", 3, "somename", "aabb")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn find_unspent_covenant_utxos_by_name_hash_scans_all_addresses() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,0,'rs1qaddr0','0014','02'),
                    ('p1',0,0,1,'rs1qaddr1','0014','02')",
            [],
        )
        .unwrap();
        let cov_a = |addr_marker: &str| {
            serde_json::json!({
                "type": 3, "action": "BID",
                "items": ["namehash1", "64000000", "72617728", addr_marker],
            })
            .to_string()
        };
        // Two BID coins for the SAME name hash at two DIFFERENT addresses.
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('aa', 0, 'p1', 'rs1qaddr0', '00', 2000, 3, ?1, 'name_lockup', NULL)",
            params![cov_a("blindA")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('bb', 0, 'p1', 'rs1qaddr1', '00', 3000, 3, ?1, 'name_lockup', NULL)",
            params![cov_a("blindB")],
        )
        .unwrap();
        // A different name's coin — must never be returned.
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('cc', 0, 'p1', 'rs1qaddr0', '00', 4000, 3, ?1, 'name_lockup', NULL)",
            params![serde_json::json!({
                "type": 3, "action": "BID",
                "items": ["othernamehash", "64000000", "72617728", "blindC"],
            })
            .to_string()],
        )
        .unwrap();
        // A spent coin — must never be returned.
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('dd', 0, 'p1', 'rs1qaddr1', '00', 5000, 3, ?1, 'name_lockup', 'spendingtx')",
            params![cov_a("blindD")],
        )
        .unwrap();

        let coins = find_unspent_covenant_utxos_by_name_hash(&conn, "p1", 3, "namehash1").unwrap();
        let mut txids: Vec<&str> = coins.iter().map(|c| c.txid.as_str()).collect();
        txids.sort();
        assert_eq!(txids, vec!["aa", "bb"]);

        // Missing name hash -> empty, not an error.
        assert!(
            find_unspent_covenant_utxos_by_name_hash(&conn, "p1", 3, "nosuchhash")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn list_unspent_wallet_name_hashes_dedups_and_extracts_rawname() {
        let conn = db();
        seed_profile(&conn, "p1");
        // "namehold" = 6e616d65686f6c64 hex — carried as rawName in a BID's
        // items[2] (OPEN/BID/FINALIZE only).
        let raw_namehold = "6e616d65686f6c64";
        let bid = serde_json::json!({
            "type": 3, "action": "BID",
            "items": ["hashA", "64000000", raw_namehold, "blind"],
        })
        .to_string();
        // A REVEAL coin for the SAME name hash — items[2] is a nonce, NOT a
        // name, so it must NOT be read as rawName. The dedup should still keep
        // the rawName recovered from the BID above.
        let reveal = serde_json::json!({
            "type": 4, "action": "REVEAL",
            "items": ["hashA", "64000000", "deadbeefnonce"],
        })
        .to_string();
        // A REGISTER coin for a DIFFERENT name hash, no rawName recoverable.
        let register = serde_json::json!({
            "type": 6, "action": "REGISTER",
            "items": ["hashB", "64000000", "aa", "bb"],
        })
        .to_string();
        for (txid, addr, cov_type, spend_class, cov, spent) in [
            ("t1", "rs1qa", 3, "name_lockup", &bid, None::<&str>),
            ("t2", "rs1qa", 4, "name_control", &reveal, None),
            ("t3", "rs1qa", 6, "name_control", &register, None),
            // A spent name coin — must be excluded.
            ("t4", "rs1qa", 3, "name_lockup", &bid, Some("spendtx")),
            // A liquid coin — no name covenant, must be excluded.
        ] {
            conn.execute(
                "INSERT INTO tracked_utxos
                    (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                     value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
                 VALUES (?1, 0, 'p1', ?2, '00', 1000, ?3, ?4, ?5, ?6)",
                params![txid, addr, cov_type as i64, cov, spend_class, spent],
            )
            .unwrap();
        }
        // A plain liquid coin.
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('t5', 0, 'p1', 'rs1qa', '00', 9000, 0, NULL, 'liquid_hns', NULL)",
            [],
        )
        .unwrap();

        let mut out = list_unspent_wallet_name_hashes(&conn, "p1").unwrap();
        out.sort_by(|a, b| a.name_hash_hex.cmp(&b.name_hash_hex));
        // Only hashA (from BID+REVEAL, deduped) and hashB (REGISTER) — spent and
        // liquid coins excluded.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name_hash_hex, "hasha");
        // rawName recovered from the BID coin, even though REVEAL for the same
        // hash carries a nonce at items[2].
        assert_eq!(out[0].raw_name_hex.as_deref(), Some(raw_namehold));
        assert_eq!(out[1].name_hash_hex, "hashb");
        // REGISTER carries no rawName.
        assert_eq!(out[1].raw_name_hex, None);
    }

    #[test]
    fn covenant_item_hex_extracts_and_lowercases() {
        let json = serde_json::json!({
            "type": 3, "action": "BID",
            "items": ["AABB", "64000000", "7261", "CCDD"],
        })
        .to_string();
        assert_eq!(covenant_item_hex(Some(&json), 0).as_deref(), Some("aabb"));
        assert_eq!(covenant_item_hex(Some(&json), 3).as_deref(), Some("ccdd"));
        assert_eq!(covenant_item_hex(Some(&json), 9), None); // out of range
        assert_eq!(covenant_item_hex(None, 0), None);
        assert_eq!(covenant_item_hex(Some("not json"), 0), None);
    }

    #[test]
    fn insert_bid_commitment_and_get() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_bid_commitment(
            &conn, "p1", "myname", "aabb", "rs1qbid", 1, 0, 100000, 200000, "nonce123", "blind456",
        )
        .unwrap();

        // I2: inserting the exact same commitment again must error, not
        // silently no-op — a silent drop here is a direct path to an
        // unrevealable bid (see `insert_bid_commitment` doc comment).
        let result = insert_bid_commitment(
            &conn, "p1", "myname", "aabb", "rs1qbid", 1, 0, 100000, 200000, "nonce123", "blind456",
        );
        assert!(result.is_err(), "duplicate commitment insert must error");

        // Verify via raw SQL since get_bid_commitment may not be exposed.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = 'p1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "no second row, and the first must not be lost");
    }

    #[test]
    fn bid_commitment_exists_reflects_exact_key() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(!bid_commitment_exists(&conn, "p1", "myname", "blind456").unwrap());
        insert_bid_commitment(
            &conn, "p1", "myname", "aabb", "rs1qbid", 1, 0, 100000, 200000, "nonce123", "blind456",
        )
        .unwrap();
        assert!(bid_commitment_exists(&conn, "p1", "myname", "blind456").unwrap());
        // Different name or different blind is not a match.
        assert!(!bid_commitment_exists(&conn, "p1", "othername", "blind456").unwrap());
        assert!(!bid_commitment_exists(&conn, "p1", "myname", "otherblind").unwrap());
    }

    #[test]
    fn upsert_name_state_and_list_tracked() {
        let conn = db();
        seed_profile(&conn, "p1");
        upsert_name_state(
            &conn,
            "p1",
            "alpha",
            &serde_json::json!({"info":{"name":"alpha","state":"CLOSED"}}),
        )
        .unwrap();
        upsert_name_state(
            &conn,
            "p1",
            "beta",
            &serde_json::json!({"info":{"name":"beta","state":"OPEN"}}),
        )
        .unwrap();

        let tracked = list_tracked_name_names(&conn, "p1").unwrap();
        assert_eq!(tracked.len(), 2);
        assert!(tracked.contains(&"alpha".to_string()));
        assert!(tracked.contains(&"beta".to_string()));
    }

    #[test]
    fn delete_wallet_profile_cascades() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Add a draft so cascade has something to delete.
        insert_tx_draft(&conn, "d1", "p1", "send_hns", "", "{}", "{}").unwrap();
        assert!(get_tx_draft(&conn, "d1").unwrap().is_some());

        delete_wallet_profile(&conn, "p1").unwrap();
        assert!(get_wallet_profile(&conn, "p1").unwrap().is_none());
        // Draft should be gone via CASCADE.
        assert!(get_tx_draft(&conn, "d1").unwrap().is_none());
    }
}
