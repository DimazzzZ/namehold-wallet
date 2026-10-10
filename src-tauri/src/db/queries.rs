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

/// The active wallet profile's stored network string — the schema allows only
/// `"mainnet"`, `"testnet"` and `"regtest"` (`CHECK (network IN (...))`);
/// `"main"` and `"simnet"` are accepted defensively by the comparison.
/// `Ok(None)` when there is no active profile — e.g. during onboarding, before
/// any wallet exists — or when the active id points at a profile that no
/// longer exists. `Err` only for a real DB failure; callers decide whether
/// that is fatal (the connection probe) or a conservative skip (the read gate).
pub fn get_active_profile_network(conn: &rusqlite::Connection) -> Result<Option<String>, AppError> {
    let id = get_active_profile_id(conn)?;
    if id.is_empty() {
        return Ok(None);
    }
    Ok(get_wallet_profile(conn, &id)?.map(|p| p.network))
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

/// The `Network` of one *named* profile, or an error when the profile is
/// missing or its stored string does not parse. For user-triggered commands,
/// read models and background jobs, where guessing mainnet would compute the
/// wrong answer (a balance with the wrong coinbase maturity, a renewal window
/// aged by a wall clock regtest does not keep, a listing judged on another
/// chain's lockup) and CODING_STANDARDS says a DB failure is returned, not
/// swallowed. Here, below `commands`, so the commands
/// (`commands::active_profile::profile_network_from_conn`) and the sync jobs
/// share one implementation.
pub fn profile_network(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<crate::noncustodial::network::Network, AppError> {
    let profile = get_wallet_profile(conn, profile_id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {profile_id}")))?;
    crate::noncustodial::derivation::network_from_profile(&profile.network)
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
            purchase_lost_reason: None,
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
    insert_tx_draft_reserving_coins_in_tx(
        &tx,
        id,
        profile_id,
        action,
        unsigned_tx_hex,
        signing_inputs_json,
        summary_json,
        inputs,
    )?;
    tx.commit()?;
    Ok(())
}

/// The body of [`insert_tx_draft_reserving_coins`] without its own
/// transaction, for a caller that must commit further writes atomically with
/// the draft (e.g. the Shakedex purchase row). The caller opens the
/// transaction, and an error leaves it to roll back on drop.
#[allow(clippy::too_many_arguments)]
pub fn insert_tx_draft_reserving_coins_in_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    profile_id: &str,
    action: &str,
    unsigned_tx_hex: &str,
    signing_inputs_json: &str,
    summary_json: &str,
    inputs: &[(String, u32)],
) -> Result<(), AppError> {
    // Self-heal stale reservations first so an abandoned earlier draft never
    // blocks a legitimate new one.
    crate::noncustodial::send::release_stale_reservations(tx, profile_id)?;

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
            // The caller drops `tx` without commit(), which rolls back
            // everything above, including the draft insert.
            return Err(AppError::InvalidInput(
                "one or more coins for this transaction were just reserved by another \
                 pending draft (or already spent) — please try again"
                    .to_string(),
            ));
        }
    }
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

/// Whether a draft in `status` has reached, or may have reached, a node:
/// `broadcast_pending` is a transport-ambiguous attempt the node may hold.
/// Such a draft is never deleted (see [`delete_tx_draft`]).
pub fn may_have_reached_chain(status: &str) -> bool {
    REACHED_CHAIN_STATUSES.contains(&status)
}

/// The draft statuses [`may_have_reached_chain`] accepts.
pub const REACHED_CHAIN_STATUSES: [&str; 3] = ["broadcasted", "confirmed", "broadcast_pending"];

/// [`REACHED_CHAIN_STATUSES`] as an SQL list, for `status IN {..}` in the
/// queries that ask the same question of the database.
pub fn reached_chain_sql() -> String {
    sql_list(REACHED_CHAIN_STATUSES.iter().copied())
}

/// The draft statuses of a draft never sent: discarding it changes nothing
/// on chain. Disjoint from [`REACHED_CHAIN_STATUSES`].
pub const UNSENT_STATUSES: [&str; 2] = ["draft", "signed"];

/// Whether a draft in `status` was never sent ([`UNSENT_STATUSES`]).
pub fn never_sent(status: &str) -> bool {
    UNSENT_STATUSES.contains(&status)
}

/// The status of a draft the chain has mined.
pub const CONFIRMED_STATUS: &str = "confirmed";

/// The status of a draft whose broadcast hsd refused: dead ([`draft_alive`]).
const FAILED_STATUS: &str = "failed";

/// Rule one: whether a draft in `status` is alive — it may yet be sent
/// ([`never_sent`]), or it was sent and not given up
/// ([`may_have_reached_chain`]). A `dropped` or `failed` draft, or one whose
/// row is gone, is dead: it holds nothing (a Locking listing's name, R27) and
/// its transaction is read from the chain alone (R19).
pub fn draft_alive(status: &str) -> bool {
    never_sent(status) || may_have_reached_chain(status)
}

/// The draft statuses [`draft_alive`] accepts, as an SQL list, for the
/// queries that ask the same question of the database.
pub fn alive_sql() -> String {
    sql_list(
        UNSENT_STATUSES
            .iter()
            .chain(REACHED_CHAIN_STATUSES.iter())
            .copied(),
    )
}

/// Rule two: whether a draft in `status` may still be mined — alive
/// ([`draft_alive`]) and not mined yet. Until a lock TRANSFER is mined or
/// given up, a missing lock coin proves nothing (R19); a Cancel transfer in
/// such a status holds back the FINALIZE into the lock, which spends the
/// same coin.
pub fn draft_may_still_land(status: &str) -> bool {
    draft_alive(status) && status != CONFIRMED_STATUS
}

/// `items` quoted as an SQL list, `('a', 'b')`. Only for the fixed spellings
/// of this module's constants, never for user input.
fn sql_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let quoted: Vec<String> = items.map(|s| format!("'{s}'")).collect();
    format!("({})", quoted.join(", "))
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
    delete_tx_draft_in_tx(&tx, id)?;
    tx.commit()?;
    Ok(())
}

/// The body of [`delete_tx_draft`] without its own transaction, for a caller
/// that replaces the draft with another one atomically. An error leaves the
/// caller's transaction to roll back on drop.
pub fn delete_tx_draft_in_tx(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<(), AppError> {
    let status: Option<String> = tx
        .query_row(
            "SELECT status FROM wallet_tx_drafts WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let status = status.ok_or_else(|| AppError::NotFound(format!("draft {id}")))?;
    if may_have_reached_chain(&status) {
        return Err(AppError::InvalidInput(
            "cannot delete a draft that has already been broadcast".to_string(),
        ));
    }
    tx.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = NULL WHERE reserved_by_draft_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM wallet_tx_drafts WHERE id = ?1", params![id])?;
    // A Shakedex purchase that was never sent goes with its draft, so the
    // listing can be bought again at once (cancelled prompt, failed sign).
    // A purchase past `pending_send` is the chain's to decide.
    tx.execute(
        "DELETE FROM shakedex_purchases
         WHERE purchase_draft_id = ?1 AND state = ?2",
        params![id, PurchaseState::PendingSend],
    )?;
    tx.execute(
        "UPDATE shakedex_purchases SET finalize_draft_id = NULL, updated_at = datetime('now')
         WHERE finalize_draft_id = ?1",
        params![id],
    )?;
    // A draft never sent (`draft`, `signed`) leaves the chain untouched; a
    // `dropped` or `failed` one was broadcast and may still be mined, so the
    // listings below keep their state and the chain judges them.
    if never_sent(&status) {
        // A listing still Locking goes with its lock TRANSFER draft: the name
        // never left; its reserved addresses stay used.
        tx.execute(
            "DELETE FROM shakedex_listings WHERE lock_transfer_draft_id = ?1 AND state = ?2",
            params![id, ListingState::Locking],
        )?;
        // An unsent FINALIZE into the lock takes its steps with it: they were
        // signed over a coin that never existed.
        tx.execute(
            &format!(
                "UPDATE shakedex_listings SET {}
                 WHERE lock_finalize_draft_id = ?1 AND {}",
                revert_to_ready_set(),
                ListingWrite::RevertToReady.source_sql()
            ),
            params![id],
        )?;
        // A Cancel transfer aborts nothing, so its listing loses the link and
        // the cancel's txid (R19).
        tx.execute(
            "UPDATE shakedex_listings
             SET abort_draft_id = NULL, abort_txid = NULL, updated_at = datetime('now')
             WHERE abort_draft_id = ?1",
            params![id],
        )?;
        // An unsent cancel takes nothing out of the lock: its own listing is
        // Listed again (Restored without a file) and forgets it (R28). A sent
        // one (`failed`, `dropped`) keeps its link: it may still be mined.
        let cancelling: Option<(String, bool)> = tx
            .query_row(
                "SELECT id, listing_file_json IS NOT NULL FROM shakedex_listings
                 WHERE cancel_draft_id = ?1 AND state = ?2",
                params![id, ListingState::Cancelling],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((listing, has_file)) = cancelling {
            uncancel_listing(tx, &listing, ListingState::uncancel_target(has_file))?;
        }
        // An unsent FINALIZE of a mined cancel: its listing awaits it again.
        let w = ListingWrite::RevertCancelFinalize;
        tx.execute(
            &format!(
                "UPDATE shakedex_listings SET {REVERT_CANCEL_FINALIZE_SET}
                 WHERE cancel_finalize_draft_id = ?1 AND {}",
                w.source_sql()
            ),
            params![id, w.target()],
        )?;
    }
    Ok(())
}

/// Where a Shakedex purchase is (`shakedex_purchases.state`): `PendingSend`
/// until its draft is sent, then `Unconfirmed`, `AwaitingFinalize` once it is
/// mined, and `Owned` after the FINALIZE — or `Lost`. Stored by
/// [`PurchaseState::as_str`]; sent to the UI in camelCase
/// (`ShakedexNameState.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PurchaseState {
    PendingSend,
    Unconfirmed,
    AwaitingFinalize,
    Owned,
    Lost,
}

impl PurchaseState {
    /// The stored spelling, as the table's CHECK constraint lists it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingSend => "pending_send",
            Self::Unconfirmed => "unconfirmed",
            Self::AwaitingFinalize => "awaiting_finalize",
            Self::Owned => "owned",
            Self::Lost => "lost",
        }
    }
}

impl std::str::FromStr for PurchaseState {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, AppError> {
        Ok(match s {
            "pending_send" => Self::PendingSend,
            "unconfirmed" => Self::Unconfirmed,
            "awaiting_finalize" => Self::AwaitingFinalize,
            "owned" => Self::Owned,
            "lost" => Self::Lost,
            other => return Err(AppError::Other(format!("unknown purchase state '{other}'"))),
        })
    }
}

impl rusqlite::ToSql for PurchaseState {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl rusqlite::types::FromSql for PurchaseState {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: AppError| rusqlite::types::FromSqlError::Other(e.to_string().into()))
    }
}

/// Where a Shakedex listing is (`shakedex_listings.state`). Stored by
/// [`ListingState::as_str`], the spellings the table's CHECK lists; sent to
/// the UI in camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ListingState {
    Locking,
    ReadyToFinalize,
    Finalizing,
    Listed,
    SalePending,
    Sold,
    Cancelling,
    CancelAwaitingFinalize,
    CancelFinalizing,
    Cancelled,
    Aborted,
    Restored,
    Expired,
}

impl ListingState {
    pub const ALL: [ListingState; 13] = [
        Self::Locking,
        Self::ReadyToFinalize,
        Self::Finalizing,
        Self::Listed,
        Self::SalePending,
        Self::Sold,
        Self::Cancelling,
        Self::CancelAwaitingFinalize,
        Self::CancelFinalizing,
        Self::Cancelled,
        Self::Aborted,
        Self::Restored,
        Self::Expired,
    ];

    /// The states a listing ends in; the name is no longer locking or locked
    /// by it. `idx_shakedex_listings_open_name` lists the same spellings.
    pub const TERMINAL: [ListingState; 4] =
        [Self::Sold, Self::Cancelled, Self::Aborted, Self::Expired];

    /// The stored spelling, as the table's CHECK constraint lists it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Locking => "locking",
            Self::ReadyToFinalize => "ready_to_finalize",
            Self::Finalizing => "finalizing",
            Self::Listed => "listed",
            Self::SalePending => "sale_pending",
            Self::Sold => "sold",
            Self::Cancelling => "cancelling",
            Self::CancelAwaitingFinalize => "cancel_awaiting_finalize",
            Self::CancelFinalizing => "cancel_finalizing",
            Self::Cancelled => "cancelled",
            Self::Aborted => "aborted",
            Self::Restored => "restored",
            Self::Expired => "expired",
        }
    }

    pub fn is_terminal(self) -> bool {
        Self::TERMINAL.contains(&self)
    }

    /// The states in which the name's own Cancel transfer is still the
    /// listing's abort (R19): from day 0 until the FINALIZE into the lock is
    /// built, the owner coin is our TRANSFER to the lock and nothing else
    /// spends it.
    pub const CANCEL_ABORTABLE: [ListingState; 2] = [Self::Locking, Self::ReadyToFinalize];

    /// Whether this state is one of [`Self::CANCEL_ABORTABLE`].
    pub fn aborts_by_cancel_transfer(self) -> bool {
        Self::CANCEL_ABORTABLE.contains(&self)
    }

    /// [`Self::CANCEL_ABORTABLE`] as an SQL list, for `state IN {..}`.
    pub fn cancel_abortable_sql() -> String {
        sql_list(Self::CANCEL_ABORTABLE.iter().map(|s| s.as_str()))
    }

    /// The states the before-lock job reads
    /// ([`list_shakedex_listings_before_lock`]): [`Self::CANCEL_ABORTABLE`],
    /// and Aborted within the re-check window.
    pub const BEFORE_LOCK_JOB: [ListingState; 3] =
        [Self::Locking, Self::ReadyToFinalize, Self::Aborted];

    /// The states the after-lock job reads ([`list_shakedex_listings_after_lock`]):
    /// the FINALIZE into the lock built or mined (Finalizing, Listed), a
    /// purchase of the lock coin in the mempool (SalePending), a Restored
    /// lock, Sold within the re-check window, which a reorg may undo, and a
    /// cancel on its way (Cancelling, CancelAwaitingFinalize,
    /// CancelFinalizing), which a purchase may still beat or a reorg undo.
    /// Disjoint from [`Self::BEFORE_LOCK_JOB`].
    pub const AFTER_LOCK_JOB: [ListingState; 8] = [
        Self::Finalizing,
        Self::Listed,
        Self::SalePending,
        Self::Restored,
        Self::Sold,
        Self::Cancelling,
        Self::CancelAwaitingFinalize,
        Self::CancelFinalizing,
    ];

    /// The states of a listing whose cancel TRANSFER is mined (R28): its
    /// FINALIZE home is awaited or built.
    pub const CANCEL_MINED: [ListingState; 2] =
        [Self::CancelAwaitingFinalize, Self::CancelFinalizing];

    /// [`Self::CANCEL_MINED`] as an SQL list, for `state IN {..}`.
    pub fn cancel_mined_sql() -> String {
        sql_list(Self::CANCEL_MINED.iter().map(|s| s.as_str()))
    }

    /// Where a Cancelling listing goes back to when its cancel can no longer
    /// land (R28): Listed with its listing file, Restored without one (a
    /// lock restored by name, whose only action was Cancel). The one choice
    /// the job and the deletion of an unsent cancel both make.
    pub fn uncancel_target(has_listing_file: bool) -> ListingState {
        if has_listing_file {
            Self::Listed
        } else {
            Self::Restored
        }
    }

    /// The states whose listing file may leave the wallet (R23): its
    /// FINALIZE into the lock is mined and the listing is not over. Before
    /// that the steps are over a coin that may never exist; Sold, Cancelled,
    /// Aborted, Expired and Restored listings are refused with their own
    /// reason.
    pub const EXPORT_ALLOWED: [ListingState; 5] = [
        Self::Listed,
        Self::SalePending,
        Self::Cancelling,
        Self::CancelAwaitingFinalize,
        Self::CancelFinalizing,
    ];
}

/// Every guarded write that moves a listing from one state to another. Its
/// [`ListingWrite::transition`] is the one table of the states each write
/// moves from and to: every write's SQL takes its `state IN (..)` from it
/// ([`ListingWrite::source_sql`]), and a write whose target is an argument
/// refuses one the table does not list ([`ListingWrite::check_to`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingWrite {
    /// [`mark_listing_ready`]: the lockup is over.
    Ready,
    /// [`mark_listing_locking_again`]: a reorg undid the lockup.
    LockingAgain,
    /// [`mark_listing_finalizing_in_tx`]: Finalize & sign wrote the FINALIZE.
    Finalize,
    /// [`revert_listing_to_ready`], and deleting an unsent FINALIZE draft.
    RevertToReady,
    /// [`mark_listing_listed`]: the FINALIZE into the lock is mined.
    Listed,
    /// [`mark_listing_finalizing_again`]: a reorg took that FINALIZE away.
    FinalizingAgain,
    /// [`abort_shakedex_listing`].
    Abort,
    /// [`expire_shakedex_listing`]: the name expired before the lock.
    ExpireBeforeLock,
    /// [`adopt_lock_finalized_elsewhere`].
    AdoptElsewhere,
    /// [`unabort_shakedex_listing`].
    Unabort,
    /// [`mark_listing_sale_pending`].
    SalePending,
    /// [`sell_shakedex_listing`].
    Sell,
    /// [`sell_listing_through_proven_lock`].
    ProvenLockSale,
    /// [`resell_sold_listing`].
    Resell,
    /// [`unsell_shakedex_listing`].
    Unsell,
    /// [`expire_locked_listing`]: the name expired under the lock.
    ExpireLocked,
    /// [`unadopt_restored_lock`].
    Unadopt,
    /// [`upgrade_restored_lock`].
    Upgrade,
    /// [`mark_listing_cancelling_in_tx`]: the cancel command wrote its
    /// signed cancel.
    Cancel,
    /// [`uncancel_listing`]: the cancel can no longer land.
    Uncancel,
    /// [`mark_listing_cancel_mined`]: a TRANSFER out of the lock coin to an
    /// address of ours is mined.
    CancelMined,
    /// [`mark_listing_cancel_unmined`]: a reorg took that TRANSFER back.
    CancelUnmined,
    /// [`mark_listing_cancel_finalizing_in_tx`]: the cancel's FINALIZE draft.
    FinalizeCancel,
    /// [`revert_listing_cancel_finalize`], and deleting an unsent cancel
    /// FINALIZE draft.
    RevertCancelFinalize,
    /// [`mark_listing_cancelled`]: the name is home.
    CancelDone,
    /// [`lower_listing_price`]: a cheaper step, the state unchanged.
    LowerPrice,
    /// [`refresh_listing_expiry`]: the listing file's `expiresAt` rewritten
    /// before it lapses (R23), the state unchanged.
    RefreshExpiry,
}

impl ListingWrite {
    pub const ALL: [ListingWrite; 27] = [
        Self::Ready,
        Self::LockingAgain,
        Self::Finalize,
        Self::RevertToReady,
        Self::Listed,
        Self::FinalizingAgain,
        Self::Abort,
        Self::ExpireBeforeLock,
        Self::AdoptElsewhere,
        Self::Unabort,
        Self::SalePending,
        Self::Sell,
        Self::ProvenLockSale,
        Self::Resell,
        Self::Unsell,
        Self::ExpireLocked,
        Self::Unadopt,
        Self::Upgrade,
        Self::Cancel,
        Self::Uncancel,
        Self::CancelMined,
        Self::CancelUnmined,
        Self::FinalizeCancel,
        Self::RevertCancelFinalize,
        Self::CancelDone,
        Self::LowerPrice,
        Self::RefreshExpiry,
    ];

    /// The table: `(from, to)`, the states this write moves a listing from
    /// and the states it may move it to.
    ///
    /// - Ending a listing before the lock (Abort, ExpireBeforeLock,
    ///   AdoptElsewhere) takes the states whose abort is still the Cancel
    ///   transfer, and Finalizing only while its FINALIZE draft is dead
    ///   ([`ends_before_lock_sql`]).
    /// - A sale of the stored lock outpoint (SalePending, Sell) takes a
    ///   listing whose name is in our lock; ReadyToFinalize never carries an
    ///   outpoint, so its sale is ProvenLockSale's, which Locking is a source
    ///   for too. Sold is a source only for its own write, Resell.
    /// - A name expiring under the lock (ExpireLocked) ends a Listed,
    ///   SalePending or Restored listing, or one whose cancel is on its way;
    ///   Sold stays Sold, and a Finalizing listing is judged by its FINALIZE
    ///   first.
    /// - A cancel (Cancel ... CancelDone) follows R28: built from Listed or
    ///   Restored, mined from any state whose lock coin is ours to spend,
    ///   back to Cancelling on a reorg, Cancelled once the name is home;
    ///   Lower price rewrites a Listed listing's steps and file, its state
    ///   unchanged, and the `expiresAt` refresh likewise. A purchase mined
    ///   before our cancel is a sale all the same (Sell from Cancelling), and
    ///   a reorg may replace a purchase with our mined cancel (CancelMined
    ///   from Sold, its sold txid forgotten) or our mined cancel with a
    ///   purchase or another cancel (Sell and CancelMined from the two
    ///   cancel-mined states).
    pub const fn transition(self) -> (&'static [ListingState], &'static [ListingState]) {
        use ListingState as S;
        const BEFORE_LOCK_END: &[ListingState] = &[S::Locking, S::ReadyToFinalize, S::Finalizing];
        const SALE: &[ListingState] = &[S::Finalizing, S::Listed, S::SalePending, S::Restored];
        // A purchase mined before our cancel is a sale all the same (R28).
        // A reorg may replace our mined cancel with a purchase (R22).
        const SELL_FROM: &[ListingState] = &[
            S::Finalizing,
            S::Listed,
            S::SalePending,
            S::Restored,
            S::Cancelling,
            S::CancelAwaitingFinalize,
            S::CancelFinalizing,
        ];
        // A TRANSFER out of the lock coin to an address of ours: our cancel,
        // another device's, or our own purchase (R28).
        // Finalizing: its FINALIZE mined and then the cancel, or a purchase of
        // our own, before this device synced. Sold: a reorg replaced the
        // winning purchase with our cancel. The two cancel-mined states: a
        // reorg replaced the mined cancel with another one (a new txid).
        const CANCEL_MINED_FROM: &[ListingState] = &[
            S::Finalizing,
            S::Listed,
            S::SalePending,
            S::Sold,
            S::Restored,
            S::Cancelling,
            S::CancelAwaitingFinalize,
            S::CancelFinalizing,
        ];
        const CANCEL_MINED: &[ListingState] = &ListingState::CANCEL_MINED;
        const LOCKED_EXPIRABLE: &[ListingState] = &[
            S::Listed,
            S::SalePending,
            S::Restored,
            S::Cancelling,
            S::CancelAwaitingFinalize,
            S::CancelFinalizing,
        ];
        match self {
            Self::Ready => (&[S::Locking], &[S::ReadyToFinalize]),
            Self::LockingAgain => (&[S::ReadyToFinalize], &[S::Locking]),
            Self::Finalize => (&[S::ReadyToFinalize], &[S::Finalizing]),
            Self::RevertToReady => (&[S::Finalizing], &[S::ReadyToFinalize]),
            Self::Listed => (&[S::Finalizing], &[S::Listed]),
            Self::FinalizingAgain => (&[S::Listed], &[S::Finalizing]),
            Self::Abort => (BEFORE_LOCK_END, &[S::Aborted]),
            Self::ExpireBeforeLock => (BEFORE_LOCK_END, &[S::Expired]),
            Self::AdoptElsewhere => (BEFORE_LOCK_END, &[S::Restored]),
            Self::Unabort => (&[S::Aborted], &[S::Locking]),
            Self::SalePending => (SALE, &[S::SalePending]),
            Self::Sell => (SELL_FROM, &[S::Sold]),
            Self::ProvenLockSale => (&[S::Locking, S::ReadyToFinalize], &[S::Sold]),
            Self::Resell => (&[S::Sold], &[S::SalePending, S::Sold]),
            Self::Unsell => (
                &[S::SalePending, S::Sold],
                &[S::Listed, S::Finalizing, S::Restored],
            ),
            Self::ExpireLocked => (LOCKED_EXPIRABLE, &[S::Expired]),
            Self::Unadopt => (&[S::Restored], &[S::Locking]),
            Self::Upgrade => (&[S::Restored], &[S::Listed]),
            Self::Cancel => (&[S::Listed, S::Restored], &[S::Cancelling]),
            Self::Uncancel => (&[S::Cancelling], &[S::Listed, S::Restored]),
            Self::CancelMined => (CANCEL_MINED_FROM, &[S::CancelAwaitingFinalize]),
            Self::CancelUnmined => (CANCEL_MINED, &[S::Cancelling]),
            Self::FinalizeCancel => (&[S::CancelAwaitingFinalize], &[S::CancelFinalizing]),
            Self::RevertCancelFinalize => (&[S::CancelFinalizing], &[S::CancelAwaitingFinalize]),
            Self::CancelDone => (CANCEL_MINED, &[S::Cancelled]),
            Self::LowerPrice => (&[S::Listed], &[S::Listed]),
            Self::RefreshExpiry => (&[S::Listed], &[S::Listed]),
        }
    }

    /// The states this write moves a listing from.
    pub const fn from(self) -> &'static [ListingState] {
        self.transition().0
    }

    /// The states this write may move a listing to.
    pub const fn to(self) -> &'static [ListingState] {
        self.transition().1
    }

    /// `state IN (..)` of [`Self::from`], for the write's `WHERE`.
    fn source_sql(self) -> String {
        format!(
            "state IN {}",
            sql_list(self.from().iter().map(|s| s.as_str()))
        )
    }

    /// R25 (T6): the SET fragment that starts a listing's market bookkeeping
    /// over when this write moves it to `to` = Listed — the listing is back
    /// on the market set (a cancel that died, a reorg back, an upgrade, a
    /// lowered price, a refreshed expiry), and the market may have been told
    /// something else meanwhile, so the jobs announce it afresh. Empty for
    /// every other target, including the other targets of a write that may
    /// also go to Listed (Unsell to Finalizing or Restored, Uncancel to
    /// Restored): those listings are off the market set.
    fn market_reset_sql(self, to: ListingState) -> &'static str {
        debug_assert!(self.to().contains(&to), "{self:?} to {to:?}");
        if to == ListingState::Listed {
            MARKET_RESET_SQL
        } else {
            ""
        }
    }

    /// The target of a write that has one.
    fn target(self) -> ListingState {
        match self.to() {
            [one] => *one,
            _ => unreachable!("{self:?} has more than one target"),
        }
    }

    /// Refuse a target the table does not list for this write: a bug in the
    /// caller.
    fn check_to(self, to: ListingState) -> Result<(), AppError> {
        if self.to().contains(&to) {
            Ok(())
        } else {
            Err(AppError::Other(format!(
                "{self:?} cannot move a listing to {to:?}"
            )))
        }
    }
}

/// The SET fragment that starts a listing's market bookkeeping over
/// ([`ListingWrite::market_reset_sql`], [`mark_listing_cancel_unmined`]).
const MARKET_RESET_SQL: &str = ", market_status = NULL, market_retry_at = NULL, \
     market_attempts = 0, market_error = NULL";

/// The condition that the listing's cancel draft (`cancel_draft_id`) exists
/// and was never sent ([`UNSENT_STATUSES`]). An SQL condition on
/// `shakedex_listings`.
fn cancel_draft_unsent_sql() -> String {
    format!(
        "EXISTS (
             SELECT 1 FROM wallet_tx_drafts d
             WHERE d.id = shakedex_listings.cancel_draft_id AND d.status IN {})",
        sql_list(UNSENT_STATUSES.iter().copied())
    )
}

/// The `NOT EXISTS` condition of a write that reopens a listing: no other
/// listing of the same profile and name is open (not
/// [`ListingState::TERMINAL`]); `idx_shakedex_listings_open_name` allows one.
/// An SQL condition on `shakedex_listings`.
fn no_other_open_listing_sql() -> String {
    format!(
        "NOT EXISTS (
             SELECT 1 FROM shakedex_listings o
             WHERE o.wallet_profile_id = shakedex_listings.wallet_profile_id
               AND o.name = shakedex_listings.name AND o.id <> shakedex_listings.id
               AND o.state NOT IN {})",
        sql_list(ListingState::TERMINAL.iter().map(|s| s.as_str()))
    )
}

impl std::str::FromStr for ListingState {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, AppError> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == s)
            .ok_or_else(|| AppError::Other(format!("unknown listing state '{s}'")))
    }
}

impl rusqlite::ToSql for ListingState {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl rusqlite::types::FromSql for ListingState {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: AppError| rusqlite::types::FromSqlError::Other(e.to_string().into()))
    }
}

/// How a listing prices the name (`shakedex_listings.mode`). Stored by
/// [`ListingMode::as_str`]; the command argument and the UI use camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListingMode {
    BuyNow,
    ReverseAuction,
}

impl ListingMode {
    /// The stored spelling, as the table's CHECK constraint lists it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuyNow => "buy_now",
            Self::ReverseAuction => "reverse_auction",
        }
    }
}

impl std::str::FromStr for ListingMode {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, AppError> {
        Ok(match s {
            "buy_now" => Self::BuyNow,
            "reverse_auction" => Self::ReverseAuction,
            other => return Err(AppError::Other(format!("unknown listing mode '{other}'"))),
        })
    }
}

impl rusqlite::ToSql for ListingMode {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl rusqlite::types::FromSql for ListingMode {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: AppError| rusqlite::types::FromSqlError::Other(e.to_string().into()))
    }
}

/// Where a published listing stands on LearnHNS Market
/// (`shakedex_listings.market_status`; `None`: nothing told yet). Stored by
/// [`MarketStatus::as_str`]; serde sends camelCase to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarketStatus {
    /// The day-0 pending listing is posted (R23).
    Pending,
    /// Our current step is on the market as we uploaded it (R23, R25).
    Listed,
    /// Someone else's copy was on the market; ours was uploaded over it (R25).
    ReplacedReuploaded,
    /// The market gave no answer; the next try is at `market_retry_at` (R25).
    Retrying,
    /// The market answered no (its own 4xx JSON `error`); its words are in
    /// `market_error`. Not retried automatically (`market_retry_at` unset):
    /// tried again only once what would be sent changes — a Lower price, an
    /// expiry refresh, a reset after a revert ([`ListingWrite::market_reset_sql`]).
    Refused,
    /// A stored step does not verify over the lock coin hsd reports; nothing
    /// was uploaded (carried from T4).
    StepsUnverified,
    /// The market was told of the cancel or the sale, or answered that it
    /// lists nothing to withdraw (R28): the jobs are done with this listing.
    Reported,
}

impl MarketStatus {
    pub const ALL: [MarketStatus; 7] = [
        Self::Pending,
        Self::Listed,
        Self::ReplacedReuploaded,
        Self::Retrying,
        Self::Refused,
        Self::StepsUnverified,
        Self::Reported,
    ];

    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Listed => "listed",
            Self::ReplacedReuploaded => "replaced_reuploaded",
            Self::Retrying => "retrying",
            Self::Refused => "refused",
            Self::StepsUnverified => "steps_unverified",
            Self::Reported => "reported",
        }
    }
}

impl std::str::FromStr for MarketStatus {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, AppError> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == s)
            .ok_or_else(|| AppError::Other(format!("unknown market status '{s}'")))
    }
}

impl rusqlite::ToSql for MarketStatus {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl rusqlite::types::FromSql for MarketStatus {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: AppError| rusqlite::types::FromSqlError::Other(e.to_string().into()))
    }
}

/// One row of `shakedex_listings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShakedexListing {
    pub id: String,
    pub wallet_profile_id: String,
    pub name: String,
    pub mode: ListingMode,
    pub state: ListingState,
    pub lock_pubkey_hex: String,
    pub lock_transfer_draft_id: Option<String>,
    /// The FINALIZE into the lock (T3); its txid is `lock_txid`.
    pub lock_finalize_draft_id: Option<String>,
    pub lock_transfer_txid: Option<String>,
    pub lock_txid: Option<String>,
    pub lock_vout: Option<i64>,
    pub payment_address: Option<String>,
    pub cancel_address: Option<String>,
    /// Receive-branch index of `cancel_address`.
    pub cancel_child_index: Option<i64>,
    pub steps_json: String,
    pub listing_file_json: Option<String>,
    pub publish: bool,
    pub market_status: Option<MarketStatus>,
    /// RFC 3339 UTC: when the next market action on the listing is due.
    pub market_retry_at: Option<String>,
    /// Failed market tries since the last success: the backoff exponent.
    pub market_attempts: i64,
    /// The market's own refusal, or why there was no answer.
    pub market_error: Option<String>,
    pub expires_at: Option<i64>,
    pub abort_draft_id: Option<String>,
    /// The txid of the Cancel transfer `abort_draft_id` holds; kept when that
    /// draft is deleted after it was broadcast.
    pub abort_txid: Option<String>,
    pub sold_txid: Option<String>,
    pub cancel_txid: Option<String>,
    /// Our signed cancel draft (R28). `cancel_draft_id` and `cancel_txid` may
    /// name different transactions: `cancel_txid` is this draft's txid until
    /// a mined TRANSFER out of the lock (another device's cancel, or our own
    /// purchase) replaces it, and `cancel_draft_id` stays. Once `cancel_vout`
    /// is set, do not assume this draft produced `cancel_txid`.
    pub cancel_draft_id: Option<String>,
    /// The output of the mined cancel TRANSFER `cancel_txid`.
    pub cancel_vout: Option<i64>,
    /// The cancel's FINALIZE draft, which brings the name home.
    pub cancel_finalize_draft_id: Option<String>,
    /// Blocks left in the cancel's transfer lockup at the last sync (hsd
    /// judges the FINALIZE at tip + 1); 0 once it can be finalized.
    pub cancel_blocks_remaining: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

const SHAKEDEX_LISTING_COLS: &str = "id, wallet_profile_id, name, mode, state, lock_pubkey_hex, \
    lock_transfer_draft_id, lock_finalize_draft_id, lock_transfer_txid, lock_txid, lock_vout, \
    payment_address, cancel_address, cancel_child_index, steps_json, listing_file_json, publish, market_status, \
    market_retry_at, market_attempts, market_error, expires_at, abort_draft_id, abort_txid, sold_txid, cancel_txid, cancel_draft_id, cancel_vout, \
    cancel_finalize_draft_id, cancel_blocks_remaining, created_at, updated_at";

fn row_to_shakedex_listing(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShakedexListing> {
    Ok(ShakedexListing {
        id: row.get("id")?,
        wallet_profile_id: row.get("wallet_profile_id")?,
        name: row.get("name")?,
        mode: row.get("mode")?,
        state: row.get("state")?,
        lock_pubkey_hex: row.get("lock_pubkey_hex")?,
        lock_transfer_draft_id: row.get("lock_transfer_draft_id")?,
        lock_finalize_draft_id: row.get("lock_finalize_draft_id")?,
        lock_transfer_txid: row.get("lock_transfer_txid")?,
        lock_txid: row.get("lock_txid")?,
        lock_vout: row.get("lock_vout")?,
        payment_address: row.get("payment_address")?,
        cancel_address: row.get("cancel_address")?,
        cancel_child_index: row.get("cancel_child_index")?,
        steps_json: row.get("steps_json")?,
        listing_file_json: row.get("listing_file_json")?,
        publish: row.get::<_, i64>("publish")? != 0,
        market_status: row.get("market_status")?,
        market_retry_at: row.get("market_retry_at")?,
        market_attempts: row.get("market_attempts")?,
        market_error: row.get("market_error")?,
        expires_at: row.get("expires_at")?,
        abort_draft_id: row.get("abort_draft_id")?,
        abort_txid: row.get("abort_txid")?,
        sold_txid: row.get("sold_txid")?,
        cancel_txid: row.get("cancel_txid")?,
        cancel_draft_id: row.get("cancel_draft_id")?,
        cancel_vout: row.get("cancel_vout")?,
        cancel_finalize_draft_id: row.get("cancel_finalize_draft_id")?,
        cancel_blocks_remaining: row.get("cancel_blocks_remaining")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// A txid as `shakedex_listings` stores it: lowercase hex, as hsd writes
/// hashes. Every listing write normalises the txids it stores or compares
/// with this, so the queries and the jobs compare stored txids exactly.
fn listing_txid(txid: &str) -> String {
    txid.to_ascii_lowercase()
}

/// Insert a listing. The indexes refuse a second open listing of the name
/// and a lock outpoint another listing already tracks.
pub fn insert_shakedex_listing(
    conn: &rusqlite::Connection,
    l: &ShakedexListing,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO shakedex_listings
            (id, wallet_profile_id, name, mode, state, lock_pubkey_hex, lock_transfer_draft_id,
             lock_finalize_draft_id, lock_transfer_txid, lock_txid, lock_vout, payment_address,
             cancel_address, cancel_child_index, steps_json, listing_file_json, publish,
             market_status, market_retry_at, market_attempts, market_error, expires_at,
             abort_draft_id, abort_txid, sold_txid, cancel_txid, cancel_draft_id, cancel_vout,
             cancel_finalize_draft_id, cancel_blocks_remaining)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)",
        params![
            l.id,
            l.wallet_profile_id,
            l.name,
            l.mode,
            l.state,
            l.lock_pubkey_hex,
            l.lock_transfer_draft_id,
            l.lock_finalize_draft_id,
            l.lock_transfer_txid.as_deref().map(listing_txid),
            l.lock_txid.as_deref().map(listing_txid),
            l.lock_vout,
            l.payment_address,
            l.cancel_address,
            l.cancel_child_index,
            l.steps_json,
            l.listing_file_json,
            i64::from(l.publish),
            l.market_status,
            l.market_retry_at,
            l.market_attempts,
            l.market_error,
            l.expires_at,
            l.abort_draft_id,
            l.abort_txid.as_deref().map(listing_txid),
            l.sold_txid.as_deref().map(listing_txid),
            l.cancel_txid.as_deref().map(listing_txid),
            l.cancel_draft_id,
            l.cancel_vout,
            l.cancel_finalize_draft_id,
            l.cancel_blocks_remaining
        ],
    )?;
    Ok(())
}

/// Fetch one listing, or `None`.
pub fn get_shakedex_listing(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<ShakedexListing>, AppError> {
    let sql = format!("SELECT {SHAKEDEX_LISTING_COLS} FROM shakedex_listings WHERE id = ?1");
    let row = conn
        .query_row(&sql, params![id], row_to_shakedex_listing)
        .optional()?;
    Ok(row)
}

/// The listing of `name` that is still open (not in a terminal state), newest
/// first: while there is one, the name is locking or locked (R27).
pub fn open_shakedex_listing_for_name(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<ShakedexListing>, AppError> {
    let [t0, t1, t2, t3] = ListingState::TERMINAL;
    let sql = format!(
        "SELECT {SHAKEDEX_LISTING_COLS} FROM shakedex_listings
         WHERE wallet_profile_id = ?1 AND name = ?2 AND state NOT IN (?3, ?4, ?5, ?6)
         ORDER BY created_at DESC LIMIT 1"
    );
    let row = conn
        .query_row(
            &sql,
            params![profile_id, name, t0, t1, t2, t3],
            row_to_shakedex_listing,
        )
        .optional()?;
    Ok(row)
}

/// Whether any listing of the profile, in any state, holds the lock coin
/// `(txid, vout)`: a lock coin is tracked by one listing
/// (`idx_shakedex_listings_lock_outpoint`).
pub fn shakedex_listing_holds_lock_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    txid: &str,
    vout: u32,
) -> Result<bool, AppError> {
    let found = conn
        .query_row(
            "SELECT 1 FROM shakedex_listings
             WHERE wallet_profile_id = ?1 AND lock_txid = ?2 AND lock_vout = ?3
             LIMIT 1",
            params![profile_id, listing_txid(txid), i64::from(vout)],
            |_| Ok(()),
        )
        .optional()?;
    Ok(found.is_some())
}

/// The open listing that keeps `name`'s owner actions away (R27), if any.
/// A listing past Locking always does. A Locking one does only while its
/// lock TRANSFER draft is alive ([`draft_alive`]): a dropped,
/// failed or deleted lock draft does not freeze the name; the chain refresh
/// (T4) resolves the row, and a TRANSFER mined after all shows as a pending
/// transfer that Cancel transfer handles.
pub fn listing_blocking_owner_actions(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<ShakedexListing>, AppError> {
    let Some(listing) = open_shakedex_listing_for_name(conn, profile_id, name)? else {
        return Ok(None);
    };
    let status = lock_draft_status(conn, &listing)?;
    let blocks = listing_holds_the_name(listing.state, status.as_deref());
    Ok(blocks.then_some(listing))
}

/// R27: whether an open listing in `state`, its lock TRANSFER draft in
/// `lock_draft_status` (`None`: no draft, or its row gone), keeps the name's
/// owner actions away and shows on the name's row: past Locking always;
/// Locking only while that draft is alive ([`draft_alive`]).
pub fn listing_holds_the_name(state: ListingState, lock_draft_status: Option<&str>) -> bool {
    state != ListingState::Locking || lock_draft_status.is_some_and(draft_alive)
}

/// The `listing` object on an Owned Names row (`ShakedexNameListing` in the UI).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingNameState {
    pub listing_id: String,
    pub state: ListingState,
    /// Blocks left in the transfer lockup while Locking, from the lock
    /// TRANSFER's confirmation height and the wallet's sync height (hsd
    /// judges the FINALIZE at tip + 1); `None` before the TRANSFER is seen
    /// mined and in every other state.
    pub blocks_until_finalize: Option<i64>,
}

/// One listing an Owned Names row shows.
pub struct ListingNameRow {
    pub name: String,
    pub lock_pubkey_hex: String,
    pub listing: ListingNameState,
}

/// The listing each of the profile's names shows on its Owned Names row: its
/// open listing (a Locking one only while its lock TRANSFER draft is alive,
/// as [`listing_blocking_owner_actions`]), else a Sold one within the last
/// `sold_days`; newest first per name.
pub fn read_shakedex_listing_names(
    conn: &rusqlite::Connection,
    profile_id: &str,
    params_net: crate::noncustodial::network::NameParams,
    tip: i64,
    sold_days: u32,
) -> Result<Vec<ListingNameRow>, AppError> {
    let [t0, t1, t2, t3] = ListingState::TERMINAL;
    let mut stmt = conn.prepare(
        "SELECT l.id, l.name, l.state, l.lock_pubkey_hex, d.status, d.confirmation_height
         FROM shakedex_listings l
         LEFT JOIN wallet_tx_drafts d ON d.id = l.lock_transfer_draft_id
         WHERE l.wallet_profile_id = ?1
           AND (l.state NOT IN (?2, ?3, ?4, ?5)
                OR (l.state = ?6 AND l.updated_at >= datetime('now', ?7)))
         ORDER BY l.name, l.created_at DESC, l.id",
    )?;
    let rows = stmt.query_map(
        params![
            profile_id,
            t0,
            t1,
            t2,
            t3,
            ListingState::Sold,
            format!("-{sold_days} days")
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, ListingState>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        },
    )?;
    let mut out: Vec<ListingNameRow> = Vec::new();
    for row in rows {
        let (id, name, state, pubkey, status, confirmed) = row?;
        if out.iter().any(|r| r.name == name) {
            continue;
        }
        if !listing_holds_the_name(state, status.as_deref()) {
            continue;
        }
        // A display estimate: the lock TRANSFER draft's confirmation height
        // as the wallet's sync recorded it and the wallet's sync height. The
        // listing's state itself moves on hsd's `info.transfer` (the
        // before-lock job), which a reorg can change before the sync does.
        let blocks_until_finalize = match (state, status.as_deref(), confirmed) {
            (ListingState::Locking, Some(CONFIRMED_STATUS), Some(h)) => {
                Some(params_net.blocks_until_finalize(h, tip))
            }
            _ => None,
        };
        out.push(ListingNameRow {
            name,
            lock_pubkey_hex: pubkey,
            listing: ListingNameState {
                listing_id: id,
                state,
                blocks_until_finalize,
            },
        });
    }
    Ok(out)
}

/// The status of the listing's lock TRANSFER draft, `None` when the listing
/// has none or the draft row is gone.
pub fn lock_draft_status(
    conn: &rusqlite::Connection,
    listing: &ShakedexListing,
) -> Result<Option<String>, AppError> {
    let Some(id) = listing.lock_transfer_draft_id.as_deref() else {
        return Ok(None);
    };
    Ok(conn
        .query_row(
            "SELECT status FROM wallet_tx_drafts WHERE id = ?1 AND wallet_profile_id = ?2",
            params![id, listing.wallet_profile_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// R19: link a Cancel transfer draft to the listing it aborts: the open
/// listing of `name` whose abort is still the Cancel transfer
/// ([`ListingState::aborts_by_cancel_transfer`]) and whose lock TRANSFER is
/// the coin `(transfer_txid, transfer_vout)` the cancel spends. Any other
/// cancel links nothing. Returns how many listings were linked (0 or 1).
pub fn link_shakedex_listing_abort(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    transfer_txid: &str,
    transfer_vout: u32,
    draft_id: &str,
    cancel_txid: &str,
) -> Result<usize, AppError> {
    // The lock TRANSFER is output 0 of its draft (the plan's covenant output).
    if transfer_vout != 0 {
        return Ok(0);
    }
    let sql = format!(
        "UPDATE shakedex_listings
         SET abort_draft_id = ?1, abort_txid = ?5, updated_at = datetime('now')
         WHERE wallet_profile_id = ?2 AND name = ?3 AND lock_transfer_txid = ?4
           AND state IN {}",
        ListingState::cancel_abortable_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            draft_id,
            profile_id,
            name,
            listing_txid(transfer_txid),
            listing_txid(cancel_txid)
        ],
    )?)
}

/// The listings the chain may still move before the FINALIZE into the lock
/// (R19): those whose abort is still the Cancel transfer
/// ([`ListingState::CANCEL_ABORTABLE`]), and those Aborted within the last
/// `recheck_days`, which a reorg could still make Locking again.
pub fn list_shakedex_listings_before_lock(
    conn: &rusqlite::Connection,
    profile_id: &str,
    recheck_days: u32,
) -> Result<Vec<ShakedexListing>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_LISTING_COLS} FROM shakedex_listings
         WHERE wallet_profile_id = ?1
           AND (state IN {}
                OR (state = ?2 AND updated_at >= datetime('now', ?3)))
         ORDER BY created_at",
        ListingState::cancel_abortable_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            profile_id,
            ListingState::Aborted,
            format!("-{recheck_days} days")
        ],
        row_to_shakedex_listing,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// R19: the name left the lock TRANSFER before the FINALIZE into the lock
/// (a mined Cancel transfer, a REVOKE, or a lock TRANSFER that never
/// landed), so the listing is Aborted. Only a listing whose abort is still
/// the Cancel transfer, or a Finalizing one whose FINALIZE is dead, moves
/// ([`ends_before_lock_sql`]). Returns how many rows changed (0 or 1).
pub fn abort_shakedex_listing(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    end_listing_before_lock(conn, id, ListingWrite::Abort)
}

/// hsd reports no live state for the name (`getnameinfo` with `info: null`)
/// before the FINALIZE into the lock, so the listing is Expired. Only a
/// listing whose abort is still the Cancel transfer, or a Finalizing one
/// whose FINALIZE is dead, moves ([`ends_before_lock_sql`]). Returns how
/// many rows changed (0 or 1).
pub fn expire_shakedex_listing(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    end_listing_before_lock(conn, id, ListingWrite::ExpireBeforeLock)
}

fn end_listing_before_lock(
    conn: &rusqlite::Connection,
    id: &str,
    w: ListingWrite,
) -> Result<usize, AppError> {
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, lock_txid = NULL, lock_vout = NULL, {}, updated_at = datetime('now')
         WHERE id = ?1 AND {}",
        DROP_DEAD_FINALIZE_SET,
        ends_before_lock_sql(w)
    );
    Ok(conn.execute(&sql, params![id, w.target()])?)
}

/// The listings the chain may still end as before the FINALIZE into the
/// lock (R19, [`abort_shakedex_listing`], [`expire_shakedex_listing`],
/// [`adopt_lock_finalized_elsewhere`]): `w`'s source states
/// ([`ListingWrite::from`]), a Finalizing one only while its FINALIZE draft
/// is dead (not [`draft_alive`], or its row gone): that FINALIZE can no
/// longer be mined, so whatever spent the lock TRANSFER was something else.
/// An SQL condition on `shakedex_listings`.
fn ends_before_lock_sql(w: ListingWrite) -> String {
    let finalizing = ListingState::Finalizing;
    let others = w.from().iter().filter(|s| **s != finalizing);
    let dead_finalize = if w.from().contains(&finalizing) {
        format!(
            " OR (state = '{}' AND NOT EXISTS (
                 SELECT 1 FROM wallet_tx_drafts d
                 WHERE d.id = shakedex_listings.lock_finalize_draft_id AND d.status IN {}))",
            finalizing.as_str(),
            alive_sql()
        )
    } else {
        String::new()
    };
    format!(
        "(state IN {}{dead_finalize})",
        sql_list(others.map(|s| s.as_str()))
    )
}

/// What a listing ended from Finalizing loses of its dead FINALIZE: the
/// draft link, the steps and file signed over a lock coin that never
/// existed, and their expiry. Already so for a listing before the lock.
const DROP_DEAD_FINALIZE_SET: &str = "lock_finalize_draft_id = NULL, steps_json = '[]', \
     listing_file_json = NULL, expires_at = NULL";

/// R19: a reorg took an Aborted listing's abort out of the chain (its lock
/// TRANSFER is a coin or the owner again), so the listing is Locking again — unless another listing of the name is
/// open by now (`idx_shakedex_listings_open_name` allows one): that newer
/// listing stays the open one, and this one stays Aborted. Returns how many
/// rows changed (0 or 1).
pub fn unabort_shakedex_listing(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    let w = ListingWrite::Unabort;
    let sql = format!(
        "UPDATE shakedex_listings SET state = ?2, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND {}",
        w.source_sql(),
        no_other_open_listing_sql()
    );
    Ok(conn.execute(&sql, params![id, w.target()])?)
}

/// The columns a Finalizing listing loses when its FINALIZE never landed:
/// the lock outpoint, the steps and the file were signed over a coin that
/// does not exist. Shared by [`revert_listing_to_ready`] and the deletion of
/// an unsent FINALIZE draft, so the two cannot drift.
fn revert_to_ready_set() -> String {
    format!(
        "state = '{}', lock_finalize_draft_id = NULL, lock_txid = NULL, lock_vout = NULL, \
         steps_json = '[]', listing_file_json = NULL, expires_at = NULL, \
         updated_at = datetime('now')",
        ListingWrite::RevertToReady.target().as_str()
    )
}

/// What Finalize & sign writes on its listing (R19, R23).
pub struct FinalizingListing<'a> {
    pub finalize_draft_id: &'a str,
    /// The FINALIZE's txid and the index of its FINALIZE output: the lock
    /// coin.
    pub lock_txid: &'a str,
    pub lock_vout: u32,
    pub steps_json: &'a str,
    pub listing_file_json: &'a str,
    pub expires_at: u64,
}

/// R19: a listing ReadyToFinalize becomes Finalizing with its FINALIZE
/// draft, lock outpoint, steps and listing file, inside the caller's
/// transaction (the one that inserts the draft), so it leaves
/// `CANCEL_ABORTABLE` together with the draft. Returns 0 when the listing is
/// not ReadyToFinalize (changed meanwhile): the caller rolls back.
pub fn mark_listing_finalizing_in_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    f: &FinalizingListing,
) -> Result<usize, AppError> {
    let expires_at = i64::try_from(f.expires_at)
        .map_err(|_| AppError::Other("listing expiry out of range".into()))?;
    let w = ListingWrite::Finalize;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, lock_finalize_draft_id = ?3, lock_txid = ?4, lock_vout = ?8,
             steps_json = ?5, listing_file_json = ?6, expires_at = ?7,
             updated_at = datetime('now')
         WHERE id = ?1 AND {}",
        w.source_sql()
    );
    Ok(tx.execute(
        &sql,
        params![
            id,
            w.target(),
            f.finalize_draft_id,
            listing_txid(f.lock_txid),
            f.steps_json,
            f.listing_file_json,
            expires_at,
            i64::from(f.lock_vout)
        ],
    )?)
}

/// A Finalizing listing whose FINALIZE never landed goes back to
/// ReadyToFinalize, losing its lock outpoint, steps and file. Returns how
/// many rows changed (0 or 1).
pub fn revert_listing_to_ready(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    Ok(conn.execute(
        &format!(
            "UPDATE shakedex_listings SET {} WHERE id = ?1 AND {}",
            revert_to_ready_set(),
            ListingWrite::RevertToReady.source_sql()
        ),
        params![id],
    )?)
}

/// Write `w`, a state change alone, on listing `id`, only while it is in
/// one of `w`'s source states. Returns how many rows changed (0 or 1).
fn move_listing(conn: &rusqlite::Connection, id: &str, w: ListingWrite) -> Result<usize, AppError> {
    let sql = format!(
        "UPDATE shakedex_listings SET state = ?2, updated_at = datetime('now'){}
         WHERE id = ?1 AND {}",
        w.market_reset_sql(w.target()),
        w.source_sql()
    );
    Ok(conn.execute(&sql, params![id, w.target()])?)
}

/// The lockup is over (R19): Locking to ReadyToFinalize.
pub fn mark_listing_ready(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    move_listing(conn, id, ListingWrite::Ready)
}

/// A reorg undid the lock TRANSFER's lockup: ReadyToFinalize to Locking.
pub fn mark_listing_locking_again(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<usize, AppError> {
    move_listing(conn, id, ListingWrite::LockingAgain)
}

/// The FINALIZE into the lock is mined: Finalizing to Listed.
pub fn mark_listing_listed(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    move_listing(conn, id, ListingWrite::Listed)
}

/// A reorg took the FINALIZE out of the chain: Listed to Finalizing.
pub fn mark_listing_finalizing_again(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<usize, AppError> {
    move_listing(conn, id, ListingWrite::FinalizingAgain)
}

/// R19 (coordinator (b)): the name was finalized into this listing's lock by
/// a FINALIZE this device did not build (another device with the same seed):
/// the listing tracks that lock coin as a Restored lock, never Aborted. Only
/// from `CANCEL_ABORTABLE`, or from Finalizing once its own FINALIZE is dead
/// ([`ends_before_lock_sql`]), whose outpoint, steps and file it drops.
/// Returns how many rows changed (0 or 1).
pub fn adopt_lock_finalized_elsewhere(
    conn: &rusqlite::Connection,
    id: &str,
    lock_txid: &str,
    lock_vout: u32,
) -> Result<usize, AppError> {
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, lock_txid = ?3, lock_vout = ?4, abort_draft_id = NULL,
             abort_txid = NULL, {}, updated_at = datetime('now')
         WHERE id = ?1 AND {}",
        DROP_DEAD_FINALIZE_SET,
        ends_before_lock_sql(ListingWrite::AdoptElsewhere)
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            ListingWrite::AdoptElsewhere.target(),
            listing_txid(lock_txid),
            lock_vout
        ],
    )?)
}

/// The listings the after-lock job reads (R19, R22):
/// [`ListingState::AFTER_LOCK_JOB`], Sold only within the last
/// `sold_recheck_days` (a reorg can still take the purchase away).
pub fn list_shakedex_listings_after_lock(
    conn: &rusqlite::Connection,
    profile_id: &str,
    sold_recheck_days: u32,
) -> Result<Vec<ShakedexListing>, AppError> {
    let open = sql_list(
        ListingState::AFTER_LOCK_JOB
            .iter()
            .filter(|s| **s != ListingState::Sold)
            .map(|s| s.as_str()),
    );
    let sql = format!(
        "SELECT {SHAKEDEX_LISTING_COLS} FROM shakedex_listings
         WHERE wallet_profile_id = ?1
           AND (state IN {open} OR (state = ?2 AND updated_at >= datetime('now', ?3)))
         ORDER BY created_at, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            profile_id,
            ListingState::Sold,
            format!("-{sold_recheck_days} days")
        ],
        row_to_shakedex_listing,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// R22: a purchase of the lock coin `lock` is in the mempool as
/// `purchase_txid`. Only from [`ListingWrite::SalePending`]'s sources, and only when
/// `lock` is the listing's stored lock outpoint: a listing without one is
/// never moved (only [`sell_listing_through_proven_lock`] gives a listing an
/// outpoint on sale), and a `lock` that differs from it is another lock's
/// purchase. Returns how many rows changed.
pub fn mark_listing_sale_pending(
    conn: &rusqlite::Connection,
    id: &str,
    purchase_txid: &str,
    lock: (&str, u32),
) -> Result<usize, AppError> {
    sale_write(conn, id, ListingWrite::SalePending, purchase_txid, lock)
}

/// R22: the lock coin `lock` was bought by the mined `purchase_txid`. Only
/// from [`ListingWrite::Sell`]'s sources, under the same stored-outpoint rule as
/// [`mark_listing_sale_pending`]. A sale (or a purchase seen pending) forgets
/// a mined cancel's FINALIZE draft link and lockup count: a reorg replaced
/// that cancel, so its FINALIZE spends a TRANSFER in no block. Returns how
/// many rows changed.
pub fn sell_shakedex_listing(
    conn: &rusqlite::Connection,
    id: &str,
    purchase_txid: &str,
    lock: (&str, u32),
) -> Result<usize, AppError> {
    sale_write(conn, id, ListingWrite::Sell, purchase_txid, lock)
}

fn sale_write(
    conn: &rusqlite::Connection,
    id: &str,
    w: ListingWrite,
    purchase_txid: &str,
    lock: (&str, u32),
) -> Result<usize, AppError> {
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, sold_txid = ?3, cancel_finalize_draft_id = NULL,
             cancel_blocks_remaining = NULL, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND lock_txid = ?4 AND lock_vout = ?5",
        w.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            w.target(),
            listing_txid(purchase_txid),
            listing_txid(lock.0),
            i64::from(lock.1)
        ],
    )?)
}

/// R22: the lock coin `lock`, which the job proved to be this listing's
/// (a FINALIZE of the name at its lock address spending its lock TRANSFER,
/// `shakedex_jobs::finalize_into_lock`), was bought by the mined
/// `purchase_txid`. Only from [`ListingWrite::ProvenLockSale`]'s sources and
/// only while the listing has no lock outpoint; `lock` becomes it. Returns
/// how many rows changed.
pub fn sell_listing_through_proven_lock(
    conn: &rusqlite::Connection,
    id: &str,
    purchase_txid: &str,
    lock: (&str, u32),
) -> Result<usize, AppError> {
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, sold_txid = ?3, lock_txid = ?4, lock_vout = ?5,
             updated_at = datetime('now')
         WHERE id = ?1 AND {} AND lock_txid IS NULL AND lock_vout IS NULL",
        ListingWrite::ProvenLockSale.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            ListingWrite::ProvenLockSale.target(),
            listing_txid(purchase_txid),
            listing_txid(lock.0),
            i64::from(lock.1)
        ],
    )?)
}

/// R22, a reorg under a Sold listing: its purchase `purchase_txid` is back in
/// the mempool (`to` SalePending, hsd's `mempool._removeBlock`), or another
/// purchase of the same lock coin was mined instead (`to` Sold, the sale's
/// txid replaced). Only from Sold ([`ListingWrite::Resell`]; Sold is a
/// source for no other write) and
/// only for the listing's own lock outpoint `lock` (a Sold row always has
/// one). SalePending is an open listing again, so it is refused while
/// another listing of the name is open (`idx_shakedex_listings_open_name`
/// allows one). Returns how many rows changed (0 or 1).
pub fn resell_sold_listing(
    conn: &rusqlite::Connection,
    id: &str,
    to: ListingState,
    purchase_txid: &str,
    lock: (&str, u32),
) -> Result<usize, AppError> {
    let w = ListingWrite::Resell;
    w.check_to(to)?;
    let sql = format!(
        "UPDATE shakedex_listings SET state = ?2, sold_txid = ?3, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND lock_txid = ?4 AND lock_vout = ?5
           AND (?2 = state OR {})",
        w.source_sql(),
        no_other_open_listing_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            to,
            listing_txid(purchase_txid),
            listing_txid(lock.0),
            i64::from(lock.1)
        ],
    )?)
}

/// R22, a reorg: the purchase is no longer in the chain, so a SalePending or
/// Sold listing goes back to `to` (Listed, Finalizing or Restored) and
/// forgets the purchase — unless another listing of the name is open by now
/// (`idx_shakedex_listings_open_name` allows one), which stays the open one.
/// Going back to Listed needs the listing file the steps are in. The
/// re-check window is not applied here: the caller got the row from
/// [`list_shakedex_listings_after_lock`], which applies it. Returns how many
/// rows changed (0 or 1).
pub fn unsell_shakedex_listing(
    conn: &rusqlite::Connection,
    id: &str,
    to: ListingState,
) -> Result<usize, AppError> {
    let w = ListingWrite::Unsell;
    w.check_to(to)?;
    let sql = format!(
        "UPDATE shakedex_listings SET state = ?2, sold_txid = NULL, updated_at = datetime('now'){}
         WHERE id = ?1 AND {} AND {}
           AND (?2 <> ?3 OR listing_file_json IS NOT NULL)",
        w.market_reset_sql(to),
        w.source_sql(),
        no_other_open_listing_sql()
    );
    Ok(conn.execute(&sql, params![id, to, ListingState::Listed])?)
}

/// The name expired, or expired and was opened again, under a locked
/// listing ([`ListingWrite::ExpireLocked`]'s sources): Expired, keeping its lock
/// outpoint. Returns how many rows changed.
pub fn expire_locked_listing(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    move_listing(conn, id, ListingWrite::ExpireLocked)
}

/// A reorg took the FINALIZE a Restored lock was adopted from out of every
/// block and mempool (its lock TRANSFER is a coin again): Locking, without
/// the outpoint; the before-lock job takes it from there. Only a Restored
/// lock whose lock TRANSFER is known (one adopted from this device's
/// listing); a lock restored by name has none. Returns how many rows changed.
pub fn unadopt_restored_lock(conn: &rusqlite::Connection, id: &str) -> Result<usize, AppError> {
    let w = ListingWrite::Unadopt;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, lock_txid = NULL, lock_vout = NULL, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND lock_transfer_txid IS NOT NULL",
        w.source_sql()
    );
    Ok(conn.execute(&sql, params![id, w.target()])?)
}

/// What a Restored lock gains from its own listing file (R32).
pub struct UpgradedListing<'a> {
    pub mode: ListingMode,
    pub payment_address: &'a str,
    pub steps_json: &'a str,
    pub listing_file_json: &'a str,
    pub expires_at: Option<i64>,
}

/// R32: a Restored lock tracking `(lock_txid, lock_vout)` with the lock key
/// `lock_pubkey_hex` becomes Listed with its file's details. Returns how many
/// rows changed (0 when the listing is no longer that Restored lock). The
/// caller must also check that the file's payment address is one of our
/// derived addresses: this write does not.
pub fn upgrade_restored_lock(
    conn: &rusqlite::Connection,
    id: &str,
    lock_txid: &str,
    lock_vout: u32,
    lock_pubkey_hex: &str,
    u: &UpgradedListing,
) -> Result<usize, AppError> {
    let w = ListingWrite::Upgrade;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, mode = ?3, payment_address = ?4, steps_json = ?5,
             listing_file_json = ?6, expires_at = ?7, updated_at = datetime('now'){}
         WHERE id = ?1 AND {} AND lock_txid = ?8 AND lock_vout = ?9
           AND lock_pubkey_hex = ?10",
        w.market_reset_sql(w.target()),
        w.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            w.target(),
            u.mode,
            u.payment_address,
            u.steps_json,
            u.listing_file_json,
            u.expires_at,
            listing_txid(lock_txid),
            i64::from(lock_vout),
            lock_pubkey_hex
        ],
    )?)
}

/// What the cancel command writes on its listing with its signed cancel
/// draft (R28).
pub struct CancellingListing<'a> {
    pub cancel_draft_id: &'a str,
    /// The signed cancel's txid.
    pub cancel_txid: &'a str,
    /// The lock coin the cancel spends: the listing's stored outpoint.
    pub lock: (&'a str, u32),
    /// The reserved address the cancel commits to, and its receive index.
    pub cancel_address: &'a str,
    pub cancel_child_index: u32,
}

/// R28: a Listed or Restored listing becomes Cancelling with its signed
/// cancel draft, inside the caller's transaction (the one that inserts the
/// draft), only while it still holds the lock coin the cancel spends and
/// the cancel address and index the cancel commits to. Returns 0 when it
/// changed meanwhile: the caller rolls back.
pub fn mark_listing_cancelling_in_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    c: &CancellingListing,
) -> Result<usize, AppError> {
    let w = ListingWrite::Cancel;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, cancel_draft_id = ?3, cancel_txid = ?4, cancel_vout = NULL,
             cancel_blocks_remaining = NULL, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND lock_txid = ?5 AND lock_vout = ?6
           AND cancel_address = ?7 AND cancel_child_index = ?8",
        w.source_sql()
    );
    Ok(tx.execute(
        &sql,
        params![
            id,
            w.target(),
            c.cancel_draft_id,
            listing_txid(c.cancel_txid),
            listing_txid(c.lock.0),
            i64::from(c.lock.1),
            c.cancel_address,
            i64::from(c.cancel_child_index)
        ],
    )?)
}

/// What a listing loses when its cancel can no longer land: the draft link
/// and the cancel's txid and output. Shared by [`uncancel_listing`] and the
/// deletion of an unsent cancel draft (which calls it); back to Listed it
/// also starts the market bookkeeping over
/// ([`ListingWrite::market_reset_sql`]). `?2` is the target state.
const UNCANCEL_SET: &str = "state = ?2, cancel_draft_id = NULL, cancel_txid = NULL, \
     cancel_vout = NULL, cancel_blocks_remaining = NULL, updated_at = datetime('now')";

/// R28: a Cancelling listing whose cancel can no longer land goes back to
/// `to` (Listed, or Restored; [`ListingState::uncancel_target`]). Going
/// back to Listed needs the listing file the steps are in. Returns how many
/// rows changed (0 or 1).
pub fn uncancel_listing(
    conn: &rusqlite::Connection,
    id: &str,
    to: ListingState,
) -> Result<usize, AppError> {
    let w = ListingWrite::Uncancel;
    w.check_to(to)?;
    let sql = format!(
        "UPDATE shakedex_listings SET {UNCANCEL_SET}{}
         WHERE id = ?1 AND {} AND (?2 <> ?3 OR listing_file_json IS NOT NULL)",
        w.market_reset_sql(to),
        w.source_sql()
    );
    Ok(conn.execute(&sql, params![id, to, ListingState::Listed])?)
}

/// R28: the lock coin `lock`, the listing's stored outpoint, was spent by
/// the mined TRANSFER `cancel` (txid, output) of the name at the lock
/// address committing to an address of ours: our cancel, another same-seed
/// device's, or a purchase of our own. CancelAwaitingFinalize with that
/// outpoint, from [`ListingWrite::CancelMined`]'s sources only; a FINALIZE
/// draft of a cancel this one replaced (a reorg) is unlinked, as it spends a
/// TRANSFER that is not mined. Returns how many rows changed (0 or 1).
pub fn mark_listing_cancel_mined(
    conn: &rusqlite::Connection,
    id: &str,
    cancel: (&str, u32),
    lock: (&str, u32),
) -> Result<usize, AppError> {
    let w = ListingWrite::CancelMined;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, cancel_txid = ?3, cancel_vout = ?4, cancel_blocks_remaining = NULL,
             cancel_finalize_draft_id = NULL, sold_txid = NULL, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND lock_txid = ?5 AND lock_vout = ?6",
        w.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            w.target(),
            listing_txid(cancel.0),
            i64::from(cancel.1),
            listing_txid(lock.0),
            i64::from(lock.1)
        ],
    )?)
}

/// R28, a reorg: hsd shows the mined cancel's TRANSFER back in the mempool,
/// or the lock coin a coin again. Cancelling, the mined output, the lockup
/// count and the FINALIZE draft link forgotten (that FINALIZE cannot be
/// mined before the cancel is again, and its lockup starts over then).
/// Only for the cancel the listing stores (`cancel_txid`). When the
/// listing's own cancel draft is unsent ([`UNSENT_STATUSES`]: the mined
/// cancel was another device's, or this one's before a send we never
/// recorded), the listing is back among [`list_listings_kept_on_market`], so
/// its market bookkeeping starts over (a `Reported` left from the mined
/// cancel would stop the jobs); with the cancel sent it stays. Returns how
/// many rows changed (0 or 1).
pub fn mark_listing_cancel_unmined(
    conn: &rusqlite::Connection,
    id: &str,
    cancel_txid: &str,
) -> Result<usize, AppError> {
    let w = ListingWrite::CancelUnmined;
    let unsent = cancel_draft_unsent_sql();
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, cancel_vout = NULL, cancel_blocks_remaining = NULL,
             cancel_finalize_draft_id = NULL, updated_at = datetime('now'),
             market_status = CASE WHEN {unsent} THEN NULL ELSE market_status END,
             market_retry_at = CASE WHEN {unsent} THEN NULL ELSE market_retry_at END,
             market_attempts = CASE WHEN {unsent} THEN 0 ELSE market_attempts END,
             market_error = CASE WHEN {unsent} THEN NULL ELSE market_error END
         WHERE id = ?1 AND {} AND cancel_txid = ?3",
        w.source_sql()
    );
    Ok(conn.execute(&sql, params![id, w.target(), listing_txid(cancel_txid)])?)
}

/// R28: a CancelAwaitingFinalize listing becomes CancelFinalizing with the
/// FINALIZE draft out of the cancel TRANSFER `cancel`, inside the caller's
/// transaction (the one that inserts the draft), only while `cancel` is
/// still its mined cancel. Returns 0 when it changed meanwhile.
pub fn mark_listing_cancel_finalizing_in_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    finalize_draft_id: &str,
    cancel: (&str, u32),
) -> Result<usize, AppError> {
    let w = ListingWrite::FinalizeCancel;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, cancel_finalize_draft_id = ?3, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND cancel_txid = ?4 AND cancel_vout = ?5",
        w.source_sql()
    );
    Ok(tx.execute(
        &sql,
        params![
            id,
            w.target(),
            finalize_draft_id,
            listing_txid(cancel.0),
            i64::from(cancel.1)
        ],
    )?)
}

/// What a CancelFinalizing listing loses when its FINALIZE draft never
/// lands (deleted unsent, or dead while the cancel TRANSFER is a coin).
/// Shared by [`revert_listing_cancel_finalize`] and the draft deletion.
/// `?2` is the target state.
const REVERT_CANCEL_FINALIZE_SET: &str =
    "state = ?2, cancel_finalize_draft_id = NULL, updated_at = datetime('now')";

/// R28: CancelFinalizing back to CancelAwaitingFinalize, only for the
/// FINALIZE draft the listing links. Returns how many rows changed (0 or 1).
pub fn revert_listing_cancel_finalize(
    conn: &rusqlite::Connection,
    id: &str,
    finalize_draft_id: &str,
) -> Result<usize, AppError> {
    let w = ListingWrite::RevertCancelFinalize;
    let sql = format!(
        "UPDATE shakedex_listings SET {REVERT_CANCEL_FINALIZE_SET}
         WHERE id = ?1 AND {} AND cancel_finalize_draft_id = ?3",
        w.source_sql()
    );
    Ok(conn.execute(&sql, params![id, w.target(), finalize_draft_id])?)
}

/// R28: the name is home: a mined FINALIZE spends the cancel TRANSFER
/// `cancel` into an address of ours. Cancelled, from
/// [`ListingWrite::CancelDone`]'s sources, only for the listing's own
/// mined cancel. Returns how many rows changed (0 or 1).
pub fn mark_listing_cancelled(
    conn: &rusqlite::Connection,
    id: &str,
    cancel: (&str, u32),
) -> Result<usize, AppError> {
    let w = ListingWrite::CancelDone;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, cancel_blocks_remaining = NULL, updated_at = datetime('now')
         WHERE id = ?1 AND {} AND cancel_txid = ?3 AND cancel_vout = ?4",
        w.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![id, w.target(), listing_txid(cancel.0), i64::from(cancel.1)],
    )?)
}

/// What Lower price writes (R26).
pub struct LoweredPrice<'a> {
    /// The lock coin the new step is signed over: the stored outpoint.
    pub lock: (&'a str, u32),
    /// The steps the new one was added to, as read before signing.
    pub old_steps_json: &'a str,
    pub steps_json: &'a str,
    pub listing_file_json: &'a str,
    pub expires_at: i64,
}

/// R26: a Listed listing's steps and listing file gain the cheaper step, its
/// state unchanged ([`ListingWrite::LowerPrice`]), only while it still holds
/// that lock coin and the steps it was read with. Returns 0 when it changed
/// meanwhile.
pub fn lower_listing_price(
    conn: &rusqlite::Connection,
    id: &str,
    p: &LoweredPrice,
) -> Result<usize, AppError> {
    let w = ListingWrite::LowerPrice;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, steps_json = ?3, listing_file_json = ?4, expires_at = ?5,
             updated_at = datetime('now'){}
         WHERE id = ?1 AND {} AND lock_txid = ?6 AND lock_vout = ?7 AND steps_json = ?8",
        w.market_reset_sql(w.target()),
        w.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            w.target(),
            p.steps_json,
            p.listing_file_json,
            p.expires_at,
            listing_txid(p.lock.0),
            i64::from(p.lock.1),
            p.old_steps_json
        ],
    )?)
}

/// What the `expiresAt` refresh writes (R23, T6): the stored listing file
/// with its new expiry, over the lock coin and the file the job read.
pub struct RefreshedExpiry<'a> {
    pub lock: (&'a str, u32),
    pub old_file: &'a str,
    pub listing_file_json: &'a str,
    pub expires_at: i64,
}

/// R23: a Listed listing's file with its `expiresAt` moved ahead, only while
/// it is still Listed on that lock coin with the file the job read
/// ([`ListingWrite::RefreshExpiry`]); its market bookkeeping starts over, so
/// the jobs upload the new file. Returns how many rows changed.
pub fn refresh_listing_expiry(
    conn: &rusqlite::Connection,
    id: &str,
    r: &RefreshedExpiry,
) -> Result<usize, AppError> {
    let w = ListingWrite::RefreshExpiry;
    let sql = format!(
        "UPDATE shakedex_listings
         SET state = ?2, listing_file_json = ?3, expires_at = ?4,
             updated_at = datetime('now'){}
         WHERE id = ?1 AND {} AND lock_txid = ?5 AND lock_vout = ?6 AND listing_file_json = ?7",
        w.market_reset_sql(w.target()),
        w.source_sql()
    );
    Ok(conn.execute(
        &sql,
        params![
            id,
            w.target(),
            r.listing_file_json,
            r.expires_at,
            listing_txid(r.lock.0),
            i64::from(r.lock.1),
            r.old_file
        ],
    )?)
}

/// The listing as a market job read it: its result is written only over
/// that row ([`record_market_result`]).
pub struct MarketSeen<'a> {
    pub state: ListingState,
    pub steps_json: &'a str,
    pub listing_file_json: Option<&'a str>,
}

/// A market job's result (R23, R25, R28).
#[derive(Debug, Clone, Copy)]
pub struct MarketUpdate<'a> {
    pub status: MarketStatus,
    /// RFC 3339 UTC: when the next market action is due; `None`: none due.
    pub retry_at: Option<&'a str>,
    pub attempts: i64,
    pub error: Option<&'a str>,
}

/// Write a market job's result. Not a state write (no `ListingWrite`): the
/// listing's state is untouched, and the row changes only while it is still
/// the one the job read (`seen`: state, steps, file), so a Lower price or a
/// state change made meanwhile is never overwritten with a stale result.
///
/// A round trip back to the same state, steps and file (Listed, a cancel
/// built and its unsent draft deleted, Listed again with the bookkeeping
/// reset) passes this guard, and the stale result replaces the reset. That
/// result is still what the market holds unless another market call ran in
/// between, and only the market jobs call the market: they run in
/// `commands::sync::run_sync_steps` under the profile's `sync_lock`, one
/// sync of a profile at a time. The lock is not strict: the app takes a
/// daemon's lock (`sync_lock::acquire_for_app`), and the daemon's sync in
/// flight runs to its end (`commands::sync::spawn_lock_heartbeat` drops the
/// lost-lock answer of `refresh_heartbeat`), so an app job and a daemon job
/// may overlap then.
///
/// Returns how many rows changed.
pub fn record_market_result(
    conn: &rusqlite::Connection,
    id: &str,
    seen: &MarketSeen,
    u: &MarketUpdate,
) -> Result<usize, AppError> {
    Ok(conn.execute(
        "UPDATE shakedex_listings
         SET market_status = ?2, market_retry_at = ?3, market_attempts = ?4,
             market_error = ?5, updated_at = datetime('now')
         WHERE id = ?1 AND state = ?6 AND steps_json = ?7 AND listing_file_json IS ?8",
        params![
            id,
            u.status,
            u.retry_at,
            u.attempts,
            u.error,
            seen.state,
            seen.steps_json,
            seen.listing_file_json
        ],
    )?)
}

/// Every transaction with a coin of ours at `address` the wallet's sync has
/// recorded, spent or not, with the height the coin was last seen at (-1 in
/// the mempool), most recently mined first. R22 looks for the purchase among
/// them: the purchase pays our listing's payment address. A row may come from
/// a transaction no longer in any block or mempool (sync's `'spent'` sentinel,
/// a stale height), so the caller must find the transaction on chain before
/// it counts as payment.
pub fn own_coins_at(
    conn: &rusqlite::Connection,
    profile_id: &str,
    address: &str,
) -> Result<Vec<(String, Option<i64>)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT txid, MAX(height) FROM tracked_utxos
         WHERE wallet_profile_id = ?1 AND address = ?2
         GROUP BY txid ORDER BY MAX(height) DESC, txid",
    )?;
    let rows = stmt.query_map(params![profile_id, address], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Whether the wallet has recorded a coin of ours at `address` in
/// transaction `txid`, spent or not.
pub fn own_coin_in_tx(
    conn: &rusqlite::Connection,
    profile_id: &str,
    txid: &str,
    address: &str,
) -> Result<bool, AppError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tracked_utxos
                       WHERE wallet_profile_id = ?1 AND txid = ?2 AND address = ?3)",
        params![profile_id, txid, address],
        |r| r.get(0),
    )?)
}

/// `(profile, name, lock transfer txid)` of every listing that waits for
/// Finalize & sign, across profiles (the `listing_ready` reminder):
/// ReadyToFinalize, or Finalizing with its FINALIZE draft not sent yet
/// ([`UNSENT_STATUSES`]) — signed, it locks nothing in until it is sent.
pub fn list_listings_ready_to_finalize(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, String, String)>, AppError> {
    let sql = format!(
        "SELECT wallet_profile_id, name, lock_transfer_txid FROM shakedex_listings
         WHERE lock_transfer_txid IS NOT NULL
           AND (state = ?1
                OR (state = ?2 AND EXISTS (
                    SELECT 1 FROM wallet_tx_drafts d
                    WHERE d.id = shakedex_listings.lock_finalize_draft_id
                      AND d.status IN {})))
         ORDER BY wallet_profile_id, name",
        sql_list(UNSENT_STATUSES.iter().copied())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![ListingState::ReadyToFinalize, ListingState::Finalizing],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// R28, a cancel that can never land: listing `id`'s lock coin went to a
/// mined purchase or to another mined cancel. `spender_txid` is the txid of
/// the transaction hsd shows as having mined that spend; it is evidence, not
/// a hint. Nothing happens (`Ok(false)`) unless the listing is in a state a
/// mined purchase or another cancel has already put it in (Sold, or one of
/// [`ListingState::CANCEL_MINED`]) and `spender_txid` is not our cancel (the
/// draft's own txid, else the listing's `cancel_txid`), so a cancel that did land
/// (or a lock coin still unspent) never loses its coins.
///
/// Then its cancel draft's reserved coins are released and the draft, unless
/// mined or already failed, is `dropped` with `reason`, so it is never sent
/// and Activity says why. Returns whether a draft was released (`false`: no
/// evidence, no cancel draft, its row gone, or mined).
///
/// Its callers (the after-lock job's purchase-beats-cancel and other-cancel
/// paths) pass the spender they read from hsd, never a txid taken from our
/// own rows.
pub fn release_losing_cancel(
    conn: &rusqlite::Connection,
    id: &str,
    spender_txid: &str,
    reason: &str,
) -> Result<bool, AppError> {
    release_cancel(conn, id, spender_txid, reason, false)
}

/// [`release_losing_cancel`] for a mined cancel a reorg replaced:
/// `spender_txid` is a MINED spender of the listing's stored lock coin that
/// is not our cancel, read from hsd, so our cancel, which spends that same
/// coin, is in no block, and a `confirmed` status the draft tracker has not
/// reverted yet is stale: such a draft is released too.
pub fn release_replaced_cancel(
    conn: &rusqlite::Connection,
    id: &str,
    spender_txid: &str,
    reason: &str,
) -> Result<bool, AppError> {
    release_cancel(conn, id, spender_txid, reason, true)
}

fn release_cancel(
    conn: &rusqlite::Connection,
    id: &str,
    spender_txid: &str,
    reason: &str,
    confirmed_is_stale: bool,
) -> Result<bool, AppError> {
    let row: Option<(ListingState, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT state, cancel_txid, cancel_draft_id FROM shakedex_listings WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((state, cancel_txid, Some(draft))) = row else {
        return Ok(false);
    };
    let reached =
        state == ListingWrite::Sell.target() || ListingState::CANCEL_MINED.contains(&state);
    if !reached {
        return Ok(false);
    }
    let Some(row) = get_tx_draft(conn, &draft)? else {
        return Ok(false);
    };
    // Our cancel's txid is the draft's own; the listing's `cancel_txid` is it
    // only until a mined cancel of another device replaces it, so it is read
    // as ours only when the draft knows no txid.
    let ours = row.txid.clone().or(cancel_txid);
    if (row.status == CONFIRMED_STATUS && !confirmed_is_stale)
        || ours.map(|t| listing_txid(&t)) == Some(listing_txid(spender_txid))
    {
        return Ok(false);
    }
    release_reserved_utxos_for_draft(conn, &draft)?;
    if row.status != FAILED_STATUS {
        update_tx_draft_status(conn, &draft, "dropped", Some(reason), None)?;
    }
    Ok(true)
}

/// The listings the market jobs keep on LearnHNS (R24, R25: the jobs that
/// re-upload and step listings read this): the
/// published (`publish`) ones that are Listed, or Cancelling while their
/// cancel draft is not sent yet (`draft`, `signed`): R28 stops the jobs once
/// a cancel is broadcast. The mainnet rule (R23) is the jobs' own.
pub fn list_listings_kept_on_market(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<ShakedexListing>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_LISTING_COLS} FROM shakedex_listings
         WHERE wallet_profile_id = ?1 AND publish = 1
           AND (state = ?2 OR (state = ?3 AND {}))
         ORDER BY created_at, id",
        cancel_draft_unsent_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![profile_id, ListingState::Listed, ListingState::Cancelling],
        row_to_shakedex_listing,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// R23 day 0: the published listings to announce as pending — Locking once
/// its lock TRANSFER draft has been sent ([`REACHED_CHAIN_STATUSES`]),
/// ReadyToFinalize and Finalizing — that the market has not taken yet
/// (`market_status` unset, or retrying after no answer; a refused one waits
/// for a change, see [`MarketStatus::Refused`]). The mainnet rule is the
/// job's own.
pub fn list_listings_to_announce(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<ShakedexListing>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_LISTING_COLS} FROM shakedex_listings
         WHERE wallet_profile_id = ?1 AND publish = 1
           AND (market_status IS NULL OR market_status = ?2)
           AND (state IN (?3, ?4)
                OR (state = ?5 AND EXISTS (
                    SELECT 1 FROM wallet_tx_drafts d
                    WHERE d.id = shakedex_listings.lock_transfer_draft_id
                      AND d.status IN {})))
         ORDER BY created_at, id",
        reached_chain_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            profile_id,
            MarketStatus::Retrying,
            ListingState::ReadyToFinalize,
            ListingState::Finalizing,
            ListingState::Locking
        ],
        row_to_shakedex_listing,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The profile's receive-branch address at `child_index` under `account`,
/// as `derived_addresses` holds it, or `None` (a cancel commits only to one
/// of these, R21/R28).
pub fn receive_address_at(
    conn: &rusqlite::Connection,
    profile_id: &str,
    account: u32,
    child_index: u32,
) -> Result<Option<String>, AppError> {
    use crate::noncustodial::derivation::BRANCH_RECEIVE;
    Ok(conn
        .query_row(
            "SELECT address FROM derived_addresses
             WHERE wallet_profile_id = ?1 AND account_index = ?2 AND branch = ?3
               AND child_index = ?4",
            params![
                profile_id,
                i64::from(account),
                i64::from(BRANCH_RECEIVE),
                i64::from(child_index)
            ],
            |r| r.get(0),
        )
        .optional()?)
}

/// R21/R28: a lock restored by name gets the cancel address the cancel
/// commits to when it is cancelled (its restore reserved none). Only a
/// Restored row without one; its state is unchanged. Returns how many rows
/// changed (0 or 1).
pub fn set_restored_lock_cancel_address(
    conn: &rusqlite::Connection,
    id: &str,
    address: &str,
    child_index: u32,
) -> Result<usize, AppError> {
    Ok(conn.execute(
        "UPDATE shakedex_listings
         SET cancel_address = ?2, cancel_child_index = ?3, updated_at = datetime('now')
         WHERE id = ?1 AND state = ?4 AND cancel_address IS NULL AND cancel_child_index IS NULL",
        params![id, address, i64::from(child_index), ListingState::Restored],
    )?)
}

/// What the after-lock job read of a mined cancel's lockup (R28, the
/// reminder): blocks left until its FINALIZE is valid at tip + 1. Only a
/// listing in [`ListingState::CANCEL_MINED`], and only when the count
/// changed. Returns how many rows changed (0 or 1).
pub fn set_cancel_blocks_remaining(
    conn: &rusqlite::Connection,
    id: &str,
    blocks: i64,
) -> Result<usize, AppError> {
    Ok(conn.execute(
        &format!(
            "UPDATE shakedex_listings SET cancel_blocks_remaining = ?2
             WHERE id = ?1 AND state IN {} AND cancel_blocks_remaining IS NOT ?2",
            ListingState::cancel_mined_sql()
        ),
        params![id, blocks],
    )?)
}

/// `(profile, name, cancel txid)` of every mined cancel whose FINALIZE can
/// be sent now, across profiles (the `cancel_finalize` reminder, R14's
/// pattern): its lockup over at the last sync and no FINALIZE draft of it
/// that may have reached the chain.
///
/// A `failed` or `dropped` FINALIZE draft counts as not sent here, unlike the
/// sibling [`list_listings_ready_to_finalize`] (whose draft is the one it
/// waits to be sent): that draft will never land, so a new FINALIZE is needed
/// and the reminder must fire again.
pub fn list_cancels_ready_to_finalize(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, String, String)>, AppError> {
    let sql = format!(
        "SELECT wallet_profile_id, name, cancel_txid FROM shakedex_listings
         WHERE state IN {} AND cancel_blocks_remaining = 0 AND cancel_txid IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM wallet_tx_drafts d
               WHERE d.id = shakedex_listings.cancel_finalize_draft_id AND d.status IN {})
         ORDER BY wallet_profile_id, name",
        ListingState::cancel_mined_sql(),
        reached_chain_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// A purchase's state with the chain facts tracked alongside it; written
/// together by [`update_shakedex_purchase_state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurchaseProgress {
    pub state: PurchaseState,
    pub purchase_height: Option<i64>,
    pub blocks_remaining: Option<i64>,
    pub missing_since_height: Option<i64>,
    pub rebroadcast_count: i64,
    pub lost_reason: Option<String>,
}

/// One row of `shakedex_purchases`: a name bought from a Shakedex listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShakedexPurchase {
    pub id: String,
    pub wallet_profile_id: String,
    pub name: String,
    pub listing_json: String,
    pub lock_txid: String,
    pub lock_vout: i64,
    pub price_doos: i64,
    pub purchase_draft_id: String,
    pub purchase_txid: String,
    pub destination_address: String,
    pub state: PurchaseState,
    pub purchase_height: Option<i64>,
    pub blocks_remaining: Option<i64>,
    pub missing_since_height: Option<i64>,
    pub rebroadcast_count: i64,
    pub lost_reason: Option<String>,
    pub finalize_draft_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl ShakedexPurchase {
    /// The price this purchase pays the seller, which every broadcast and
    /// rebroadcast re-checks against the listing (R10). SQLite keeps it as
    /// an `i64`; a negative one was not written by this wallet.
    pub fn paid_doos(&self) -> Result<u64, AppError> {
        u64::try_from(self.price_doos).map_err(|_| {
            AppError::Other(format!(
                "the purchase's recorded price {} is unreadable",
                self.price_doos
            ))
        })
    }
}

const SHAKEDEX_PURCHASE_COLS: &str = "id, wallet_profile_id, name, listing_json, lock_txid, \
    lock_vout, price_doos, purchase_draft_id, purchase_txid, destination_address, state, \
    purchase_height, blocks_remaining, missing_since_height, rebroadcast_count, lost_reason, \
    finalize_draft_id, created_at, updated_at";

fn row_to_shakedex_purchase(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShakedexPurchase> {
    Ok(ShakedexPurchase {
        id: row.get("id")?,
        wallet_profile_id: row.get("wallet_profile_id")?,
        name: row.get("name")?,
        listing_json: row.get("listing_json")?,
        lock_txid: row.get("lock_txid")?,
        lock_vout: row.get("lock_vout")?,
        price_doos: row.get("price_doos")?,
        purchase_draft_id: row.get("purchase_draft_id")?,
        purchase_txid: row.get("purchase_txid")?,
        destination_address: row.get("destination_address")?,
        state: row.get("state")?,
        purchase_height: row.get("purchase_height")?,
        blocks_remaining: row.get("blocks_remaining")?,
        missing_since_height: row.get("missing_since_height")?,
        rebroadcast_count: row.get("rebroadcast_count")?,
        lost_reason: row.get("lost_reason")?,
        finalize_draft_id: row.get("finalize_draft_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Insert a purchase. The unique partial index refuses a second open
/// (`pending_send`/`unconfirmed`) purchase of the same lock outpoint.
pub fn insert_shakedex_purchase(
    conn: &rusqlite::Connection,
    p: &ShakedexPurchase,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO shakedex_purchases
            (id, wallet_profile_id, name, listing_json, lock_txid, lock_vout, price_doos,
             purchase_draft_id, purchase_txid, destination_address, state, purchase_height,
             blocks_remaining, missing_since_height, rebroadcast_count, lost_reason,
             finalize_draft_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            p.id,
            p.wallet_profile_id,
            p.name,
            p.listing_json,
            p.lock_txid,
            p.lock_vout,
            p.price_doos,
            p.purchase_draft_id,
            p.purchase_txid,
            p.destination_address,
            p.state,
            p.purchase_height,
            p.blocks_remaining,
            p.missing_since_height,
            p.rebroadcast_count,
            p.lost_reason,
            p.finalize_draft_id
        ],
    )?;
    Ok(())
}

/// A profile's purchases that are not yet `owned` or `lost`, oldest first.
pub fn list_open_shakedex_purchases(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<ShakedexPurchase>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_PURCHASE_COLS} FROM shakedex_purchases
         WHERE wallet_profile_id = ?1 AND state NOT IN (?2, ?3)
         ORDER BY created_at ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![profile_id, PurchaseState::Owned, PurchaseState::Lost],
            row_to_shakedex_purchase,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A profile's purchases marked `lost` within the last `days` days while
/// they paid nothing: the purchase draft is `dropped` or `failed` (or was
/// deleted, which only those statuses allow). Such a purchase can still be
/// mined later (see `shakedex_jobs`).
pub fn list_recent_unpaid_lost_shakedex_purchases(
    conn: &rusqlite::Connection,
    profile_id: &str,
    days: u32,
) -> Result<Vec<ShakedexPurchase>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_PURCHASE_COLS} FROM shakedex_purchases
         WHERE wallet_profile_id = ?1 AND state = ?2
           AND updated_at > datetime('now', ?3)
           AND NOT EXISTS (
               SELECT 1 FROM wallet_tx_drafts d
                WHERE d.id = purchase_draft_id AND d.status NOT IN ('dropped','failed'))
         ORDER BY created_at ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![profile_id, PurchaseState::Lost, format!("-{days} days")],
            row_to_shakedex_purchase,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Purchases of `profile_id` that became `owned` within the last `days`
/// days: a reorg can still take their FINALIZE away (R13).
pub fn list_recent_owned_shakedex_purchases(
    conn: &rusqlite::Connection,
    profile_id: &str,
    days: u32,
) -> Result<Vec<ShakedexPurchase>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_PURCHASE_COLS} FROM shakedex_purchases
         WHERE wallet_profile_id = ?1 AND state = ?2
           AND updated_at > datetime('now', ?3)
         ORDER BY created_at ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![profile_id, PurchaseState::Owned, format!("-{days} days")],
            row_to_shakedex_purchase,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The `shakedex` object on an Owned Names row (`ShakedexNameState` in the UI).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseNameState {
    pub state: PurchaseState,
    pub blocks_remaining: Option<i64>,
    pub purchase_id: String,
}

/// A profile's purchased names still on their way (`unconfirmed` or
/// `awaiting_finalize`), shaped like [`read_cached_names`] rows so Owned Names
/// can list them (the chain fields null, since the name is not ours yet, and
/// no `claimed`), plus a [`PurchaseNameState`] under `shakedex`. A purchase
/// already sent (its draft [`may_have_reached_chain`]) is listed as `unconfirmed` before the purchase job has
/// looked at it: that job runs only against an authoritative node, and the
/// name appears from the moment it is sent (R14).
pub fn read_shakedex_purchase_names(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT p.id, p.name, p.state, p.blocks_remaining, p.destination_address
         FROM shakedex_purchases p
         WHERE p.wallet_profile_id = ?1
           AND (p.state IN (?2, ?3)
                OR (p.state = ?4 AND EXISTS (
                    SELECT 1 FROM wallet_tx_drafts d
                    WHERE d.id = p.purchase_draft_id
                      AND d.status IN {})))
         ORDER BY p.name, p.created_at, p.id",
        reached_chain_sql()
    ))?;
    let rows = stmt.query_map(
        params![
            profile_id,
            PurchaseState::Unconfirmed,
            PurchaseState::AwaitingFinalize,
            PurchaseState::PendingSend
        ],
        |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let state = match row.get::<_, PurchaseState>(2)? {
                // Sent, not yet seen by the purchase job.
                PurchaseState::PendingSend => PurchaseState::Unconfirmed,
                s => s,
            };
            let blocks_remaining: Option<i64> = row.get(3)?;
            let destination: String = row.get(4)?;
            let shakedex = PurchaseNameState {
                state,
                blocks_remaining,
                purchase_id: id,
            };
            Ok(serde_json::json!({
                "name": name,
                "state": serde_json::Value::Null,
                "height": serde_json::Value::Null,
                "renewal": serde_json::Value::Null,
                "owner": serde_json::Value::Null,
                "owner_address": destination,
                "registered": true,
                "expired": None::<bool>,
                "stats": serde_json::Value::Null,
                "shakedex": shakedex,
            }))
        },
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Every purchase, across all profiles, whose transfer lockup is over and
/// whose finalize is not sent yet: the name can be finalized now. A purchase
/// whose finalize draft [`may_have_reached_chain`] stays `awaiting_finalize`
/// until the job sees it mined, and is left out. `(profile id, name,
/// purchase txid)`.
pub fn list_purchases_ready_to_finalize(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, String, String)>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT p.wallet_profile_id, p.name, p.purchase_txid FROM shakedex_purchases p
         WHERE p.state = ?1 AND p.blocks_remaining = 0
           AND NOT EXISTS (
               SELECT 1 FROM wallet_tx_drafts d
               WHERE d.id = p.finalize_draft_id
                 AND d.status IN {})
         ORDER BY p.wallet_profile_id, p.name, p.purchase_txid",
        reached_chain_sql()
    ))?;
    let rows = stmt
        .query_map(params![PurchaseState::AwaitingFinalize], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Fetch one purchase, or `None`.
pub fn get_shakedex_purchase(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<ShakedexPurchase>, AppError> {
    let sql = format!("SELECT {SHAKEDEX_PURCHASE_COLS} FROM shakedex_purchases WHERE id = ?1");
    let row = conn
        .query_row(&sql, params![id], row_to_shakedex_purchase)
        .optional()?;
    Ok(row)
}

/// The purchase a purchase draft would send, or `None`.
pub fn get_shakedex_purchase_by_draft(
    conn: &rusqlite::Connection,
    purchase_draft_id: &str,
) -> Result<Option<ShakedexPurchase>, AppError> {
    let sql = format!(
        "SELECT {SHAKEDEX_PURCHASE_COLS} FROM shakedex_purchases WHERE purchase_draft_id = ?1"
    );
    let row = conn
        .query_row(&sql, params![purchase_draft_id], row_to_shakedex_purchase)
        .optional()?;
    Ok(row)
}

/// Move a purchase to `progress.state`, replacing its tracking fields.
pub fn update_shakedex_purchase_state(
    conn: &rusqlite::Connection,
    id: &str,
    progress: &PurchaseProgress,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE shakedex_purchases
            SET state = ?2, purchase_height = ?3, blocks_remaining = ?4,
                missing_since_height = ?5, rebroadcast_count = ?6, lost_reason = ?7,
                updated_at = datetime('now')
         WHERE id = ?1",
        params![
            id,
            progress.state,
            progress.purchase_height,
            progress.blocks_remaining,
            progress.missing_since_height,
            progress.rebroadcast_count,
            progress.lost_reason
        ],
    )?;
    Ok(())
}

/// Record the draft that finalizes the purchased name.
pub fn set_shakedex_purchase_finalize_draft(
    conn: &rusqlite::Connection,
    id: &str,
    draft_id: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE shakedex_purchases
            SET finalize_draft_id = ?2, updated_at = datetime('now')
         WHERE id = ?1",
        params![id, draft_id],
    )?;
    Ok(())
}

/// Delete a purchase row.
pub fn delete_shakedex_purchase(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM shakedex_purchases WHERE id = ?1", params![id])?;
    Ok(())
}

/// Delete a purchase whose draft was abandoned before it was sent, together
/// with that draft, releasing the draft's coin reservations — all in one
/// transaction. Only when the draft is still `draft`/`signed` and older than
/// `ttl_secs` (checked in the DELETE itself, so a draft that moved on since the
/// caller read it is left alone): a signed draft left behind without its
/// purchase row could still be broadcast and send money no purchase tracks.
/// Returns whether anything was deleted.
pub fn delete_abandoned_shakedex_purchase(
    conn: &rusqlite::Connection,
    purchase_id: &str,
    draft_id: &str,
    ttl_secs: i64,
) -> Result<bool, AppError> {
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        &format!(
            "DELETE FROM shakedex_purchases
             WHERE id = ?1 AND purchase_draft_id = ?2
               AND EXISTS (SELECT 1 FROM wallet_tx_drafts
                           WHERE id = ?2 AND status IN ('draft','signed')
                             AND created_at < datetime('now', '-{ttl_secs} seconds'))"
        ),
        params![purchase_id, draft_id],
    )?;
    if n == 0 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = NULL WHERE reserved_by_draft_id = ?1",
        params![draft_id],
    )?;
    tx.execute(
        "DELETE FROM wallet_tx_drafts WHERE id = ?1 AND status IN ('draft','signed')",
        params![draft_id],
    )?;
    tx.commit()?;
    Ok(true)
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

/// `dropped` and `failed` drafts last updated within `max_age_secs`: their
/// coins were released, but a transaction other nodes still hold can be mined
/// after the verdict, and the confirmation poll keeps looking for it that long.
pub fn list_released_drafts_to_watch(
    conn: &rusqlite::Connection,
    profile_id: &str,
    max_age_secs: i64,
) -> Result<Vec<TxDraftRow>, AppError> {
    let sql = format!(
        "SELECT {DRAFT_COLS} FROM wallet_tx_drafts
         WHERE wallet_profile_id = ?1
           AND status IN ('dropped', 'failed')
           AND signed_tx_hex IS NOT NULL
           AND (julianday('now') - julianday(updated_at)) * 86400 < ?2
         ORDER BY created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id, max_age_secs], row_to_draft)?;
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

/// One receive-branch address row for the "all addresses" list UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveAddressRow {
    /// BIP44 child index within the receive branch (branch = 0).
    pub index: u32,
    pub address: String,
    /// True when the address is marked used (sync saw coins there, or it was
    /// reserved) or is referenced by any tracked UTXO, bid commitment or
    /// Shakedex purchase destination. Mirrors the "used" test in
    /// `derivation::next_unused_receive_address`, so what the list marks as
    /// used is exactly what address allocation skips over.
    pub used: bool,
    /// ISO 8601 timestamp of when this address was first derived
    /// (`derived_addresses.created_at`).
    pub first_seen_at: String,
}

/// List every derived RECEIVE-branch address for a profile, oldest index
/// first, each tagged with whether it has been used (the `used` flag is set,
/// or a tracked UTXO, a bid commitment or a Shakedex purchase destination
/// points at it). Change-branch addresses are intentionally excluded —
/// they are wallet-internal and never handed out.
pub fn list_receive_addresses(
    conn: &rusqlite::Connection,
    profile_id: &str,
    account_index: u32,
) -> Result<Vec<ReceiveAddressRow>, AppError> {
    // Use the canonical branch constant + the shared `used` SQL fragment so
    // this query can never drift from what address allocation
    // (`next_unused_receive_address`) skips over.
    use crate::noncustodial::derivation::{ADDRESS_USED_PREDICATE, BRANCH_RECEIVE};
    let sql = format!(
        "SELECT d.child_index, d.address, d.created_at,
                {ADDRESS_USED_PREDICATE} AS used
         FROM derived_addresses d
         WHERE d.wallet_profile_id = ?1 AND d.account_index = ?2 AND d.branch = ?3
         ORDER BY d.child_index"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![profile_id, account_index as i64, BRANCH_RECEIVE as i64],
        |row| {
            Ok(ReceiveAddressRow {
                index: row.get::<_, i64>(0)? as u32,
                address: row.get::<_, String>(1)?,
                first_seen_at: row.get::<_, String>(2)?,
                used: row.get::<_, i64>(3)? != 0,
            })
        },
    )?;
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
    let mut lost = conn.prepare(
        "SELECT purchase_draft_id, lost_reason FROM shakedex_purchases
         WHERE wallet_profile_id = ?1 AND state = ?2",
    )?;
    let lost: std::collections::HashMap<String, Option<String>> = lost
        .query_map(params![profile_id, PurchaseState::Lost], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for r in rows {
        let mut d = r?.to_summary();
        d.purchase_lost_reason = lost.get(&d.id).cloned().flatten();
        out.push(d);
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

/// Every action this wallet has BROADCAST for `name` that the chain has not
/// mined yet — `"open"`, `"reveal"` and so on — newest first.
///
/// This is the gap the UI has to narrate. Between broadcast and the next block
/// the chain still reports the name's previous state, so every phase-derived
/// label says nothing happened, while the user has just watched a confirmation
/// dialog and a txid go by. On a chain that mines on demand (regtest) the gap
/// is indefinite.
///
/// Only `broadcast_pending`/`broadcasted` count: a `draft` or `signed` row has
/// not left the device, and `confirmed`/`dropped`/`failed` are settled.
///
/// More than one can be in flight at once — a register and a redeem on the
/// same name spend different coins and are independent — so a caller asking
/// "is a transaction of this kind in flight?" must look at all of them rather
/// than at the first. `created_at` has second resolution, so two drafts made
/// in the same second order arbitrarily between themselves.
pub fn pending_broadcast_actions_for_name(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Vec<String>, AppError> {
    let sql = format!(
        "SELECT {DRAFT_COLS} FROM wallet_tx_drafts
         WHERE wallet_profile_id = ?1
           AND status IN ('broadcast_pending','broadcasted')
         ORDER BY created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![profile_id], row_to_draft)?;
    let mut out = Vec::new();
    for r in rows {
        let row = r?;
        if draft_summary_covers_name(&row.summary_json, name) {
            out.push(row.action);
        }
    }
    Ok(out)
}

/// The most recent of [`pending_broadcast_actions_for_name`], or `None` when
/// nothing this wallet sent for `name` is still waiting for a block.
///
/// "Most recent" is the one the user just pressed, which is what a single
/// "waiting for a block" label should name. Ordering is by `created_at`, which
/// has second resolution, so two drafts made in the same second pick between
/// themselves arbitrarily — a caller that must not miss one of several in
/// flight wants the plural form instead.
pub fn pending_broadcast_action_for_name(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<String>, AppError> {
    Ok(pending_broadcast_actions_for_name(conn, profile_id, name)?
        .into_iter()
        .next())
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
/// Spendable liquid coins map to `confirmed`; name-bound value (control +
/// lockup) maps to `locked_confirmed`. Immature coinbase value maps to
/// `unconfirmed`: it is the wallet's, but not yet usable — the closest of the
/// four buckets, and better than counting it as confirmed-and-spendable. We
/// still don't split a mempool bucket.
pub fn read_cached_balance(
    conn: &rusqlite::Connection,
    profile_id: &str,
    network: crate::noncustodial::network::Network,
) -> Result<serde_json::Value, AppError> {
    let b = crate::noncustodial::sync::compute_balances(conn, profile_id, network)?;
    Ok(serde_json::json!({
        "confirmed": b.liquid,
        "unconfirmed": b.immature,
        "locked_confirmed": b.name_control + b.name_lockup,
        "locked_unconfirmed": 0,
    }))
}

/// Whether a name was claimed (a reserved name's CLAIM), from a cached
/// `getnameinfo` reply (`{"info": {"claimed": n, ..}}`, `upsert_name_state`).
/// `None` when the cache does not say: an explorer row, or no `info`.
pub fn claimed_from_name_info(raw_json: &str) -> Option<bool> {
    serde_json::from_str::<serde_json::Value>(raw_json)
        .ok()?
        .get("info")?
        .get("claimed")?
        .as_u64()
        .map(|c| c > 0)
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
                n.owner_address, n.raw_json
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
        let raw_json: Option<String> = row.get(8)?;
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
            "claimed": raw_json.as_deref().and_then(claimed_from_name_info),
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

/// Owned names for a profile, shaped like the frontend `HsdName`, safe to
/// serve whether or not a node sync has populated `tracked_utxos`.
///
/// Ownership is proved by EITHER of two signals:
///
/// * `owner_address IS NOT NULL` — the row was written by the
///   explorer-verified path (`upsert_owned_name`), which is reached only after
///   `resolve_owner_via_history` confirmed the outpoint pays a wallet address.
/// * A matching UNSPENT wallet `name_control` UTXO exists in `tracked_utxos` —
///   the same rule `read_cached_names` enforces for node-synced wallets.
///
/// The node-discovery path (`upsert_name_state`) also stamps `owner_txid`
/// (from `getnameinfo`) for every name the wallet holds any covenant coin
/// on, INCLUDING bid `name_lockup` coins whose covenant name hash matches a
/// name someone else owns. Those rows have `owner_address = NULL` and no
/// wallet `name_control` UTXO, so this gate correctly excludes them —
/// closing the "bid-only names leak into Owned Names" bug.
///
/// Also extracts `registered` and `expired` from the persisted `raw_json`
/// when available, so the frontend has accurate registration-status metadata
/// even when the local node is not fully synced.
pub fn read_owned_names_explorer(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(
        // A name is "owned" only when its recorded owner outpoint is real AND
        // provably belongs to the wallet. Two write paths populate owner_txid:
        //
        //   1. The explorer-verified path (`upsert_owned_name`) is reached only
        //      after `resolve_owner_via_history` confirmed `owned_by_wallet`,
        //      and it ALWAYS stores a non-null `owner_address`. Those rows are
        //      trusted directly (a node sync may never have filled
        //      `tracked_utxos`, which is the whole reason this explorer read
        //      exists).
        //
        //   2. The node-discovery path (`upsert_name_state`) copies the
        //      on-chain `owner.hash` from `getnameinfo` for EVERY name the
        //      wallet holds any covenant coin on — including BID `name_lockup`
        //      coins — WITHOUT checking wallet ownership, and leaves
        //      `owner_address` NULL. A name the wallet only bid on therefore
        //      leaks in through this path. For these rows we require a matching
        //      UNSPENT wallet `name_control` UTXO (the actual owner coin, never
        //      a bid lockup), mirroring `read_cached_names`.
        //
        // Either way we reject the empty / all-zeros (hsd ZERO_HASH) owner hash.
        "SELECT name, state, height, renewal_height, owner_txid, owner_vout, raw_json, owner_address
         FROM tracked_name_states
         WHERE wallet_profile_id = ?1
           AND owner_txid IS NOT NULL
           AND owner_txid <> ''
           AND owner_txid <> '0000000000000000000000000000000000000000000000000000000000000000'
           AND (
               owner_address IS NOT NULL
               OR EXISTS (
                   SELECT 1 FROM tracked_utxos u
                   WHERE u.wallet_profile_id = tracked_name_states.wallet_profile_id
                     AND u.txid = tracked_name_states.owner_txid
                     AND u.vout = tracked_name_states.owner_vout
                     AND u.spent_by_txid IS NULL
                     AND u.spend_class = 'name_control'
               )
           )
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
    /// `days_until_expire` the same way `read_renewals` does
    /// (`NameParams::expiry_end` of the renewal height vs. a persisted height
    /// estimate) instead of
    /// leaving the expiry alarm silent for lack of live node stats.
    pub renewal_height: Option<i64>,
    /// The block the name's TRANSFER was recorded in
    /// (`getnameinfo().info.transfer`), or `None`/0 when none is pending.
    /// hsd refuses a FINALIZE until `transfer + transfer_lockup` blocks have
    /// passed, so the capability gate needs it to avoid offering one the node
    /// will throw away.
    pub transfer_height: Option<i64>,
}

/// Resolve the name for a given nameHash (hex) under a profile. Returns None if
/// the name is not tracked by this wallet (e.g. a name being bid on by another
/// party, or a name not yet tracked). Used by Ledger signing to populate
/// on-device name markers for covenant actions.
pub fn get_name_by_hash(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name_hash_hex: &str,
) -> Result<Option<String>, AppError> {
    let name = conn
        .query_row(
            "SELECT name FROM tracked_name_states
             WHERE wallet_profile_id = ?1 AND name_hash_hex = ?2",
            params![profile_id, name_hash_hex],
            |row| row.get(0),
        )
        .optional()?;
    Ok(name)
}

/// Fetch the tracked-name-state row for `name` under `profile_id`, if one exists.
pub fn get_tracked_name_state(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
) -> Result<Option<TrackedNameRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT name, state, owner_address, raw_json, renewal_height, transfer_height
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
                    transfer_height: row.get(5)?,
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

/// True when the profile holds an UNSPENT, UNCONFIRMED covenant coin of
/// `covenant_type` for `name_hash_hex` — one this wallet broadcast that is
/// still sitting in the mempool (hsd reports `-1`/absent as the height of a
/// mempool coin; see `NodeCoin::height`).
///
/// This is the "still in flight" question, which is not the same as
/// [`find_unspent_covenant_utxos_by_name_hash`]'s "we hold one". An OPEN coin,
/// for instance, is a zero-value marker nobody ever spends, so once it confirms
/// the wallet holds it forever — long after that auction has ended. Asking the
/// broader question to mean "in flight" made a name whose auction had lapsed
/// look like it was still opening.
pub fn has_unconfirmed_covenant_utxo_by_name_hash(
    conn: &rusqlite::Connection,
    profile_id: &str,
    covenant_type: i64,
    name_hash_hex: &str,
) -> Result<bool, AppError> {
    let mut stmt = conn.prepare(
        "SELECT u.covenant_json
         FROM tracked_utxos u
         WHERE u.wallet_profile_id = ?1
           AND u.covenant_type = ?2
           AND u.spent_by_txid IS NULL
           AND (u.height IS NULL OR u.height < 0)",
    )?;
    let want = name_hash_hex.to_ascii_lowercase();
    let mut rows = stmt.query(params![profile_id, covenant_type])?;
    while let Some(row) = rows.next()? {
        let covenant_json: Option<String> = row.get(0)?;
        if covenant_name_hash_hex(covenant_json.as_deref()).as_deref() == Some(want.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
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
    /// OPEN height of the auction this bid was placed in (030). `None` for a
    /// commitment recovered from the chain, where the auction is unknown.
    pub name_start_height: Option<i64>,
}

impl BidCommitmentRow {
    /// Whether this commitment belongs to the auction that opened at
    /// `auction_start`.
    ///
    /// A name can be auctioned many times: one nobody reveals in lapses and the
    /// name becomes available again, so a commitment from a dead auction must
    /// not count as a bid on the live one.
    ///
    /// Two unknowns are deliberately permissive. A commitment recovered from
    /// the chain rather than built here has no recorded auction (migration
    /// 030), and a caller that could not resolve the auction's start passes
    /// `None`; in both cases counting one that may be dead is a wrong number on
    /// screen, while hiding a live one is a bid the user is never told to
    /// reveal. Only a recorded mismatch excludes.
    pub fn belongs_to_auction(&self, auction_start: Option<i64>) -> bool {
        match (auction_start, self.name_start_height) {
            (Some(start), Some(placed)) => placed == start,
            (Some(_), None) => true,
            (None, _) => true,
        }
    }
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
     reveal_end_height, name_start_height";

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
        name_start_height: row.get(12)?,
    })
}

/// Persist the auction a bid commitment belongs to, and the reveal-window-close
/// height derived from it, for the commitment `build_bid_draft` just inserted —
/// the only caller with the live auction `start` height in hand (see
/// `014_reveal_end_height.sql` and `030_bid_commitment_auction.sql`).
pub fn set_auction_heights(
    conn: &rusqlite::Connection,
    profile_id: &str,
    blind_hex: &str,
    name_start_height: i64,
    reveal_end_height: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE bid_commitments
            SET reveal_end_height = ?3, name_start_height = ?4
          WHERE wallet_profile_id = ?1 AND blind_hex = ?2",
        params![profile_id, blind_hex, reveal_end_height, name_start_height],
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
    blind_hex: &str,
    txid: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE bid_commitments SET reveal_txid = ?4
         WHERE wallet_profile_id = ?1 AND name = ?2 AND blind_hex = ?3",
        params![profile_id, name, blind_hex, txid],
    )?;
    Ok(())
}

/// Whether a wallet profile row exists.
///
/// Node-config resolution (ADR-001) treats a missing profile as a hard error
/// rather than a fallback to global settings, so it has to ask this before it
/// merges anything.
pub fn wallet_profile_exists(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<bool, AppError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) > 0 FROM wallet_profiles WHERE id = ?1",
        [profile_id],
        |row| row.get(0),
    )?)
}

/// All `profile_settings` rows for one profile, as a key/value map.
///
/// These are the per-profile overrides of ADR-001. An absent key means "no
/// choice made here", which resolution reads as "fall back to global".
pub fn get_profile_settings(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<std::collections::HashMap<String, String>, AppError> {
    let mut stmt = conn.prepare("SELECT key, value FROM profile_settings WHERE profile_id = ?1")?;
    let rows = stmt.query_map([profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (k, v) = row?;
        map.insert(k, v);
    }
    Ok(map)
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

    /// Seed an unspent wallet `name_control` UTXO (the owner coin) at the given
    /// outpoint, so `read_owned_names_explorer`'s ownership gate is satisfied.
    fn seed_name_control_utxo(
        conn: &Connection,
        profile_id: &str,
        txid: &str,
        vout: i64,
        address: &str,
    ) {
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES (?1, ?2, ?3, ?4, '00', 1000, 6, NULL, 'name_control', NULL)",
            params![txid, vout, profile_id, address],
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
    fn active_profile_network_is_none_without_an_active_profile() {
        let conn = db();
        // No profile at all.
        assert_eq!(get_active_profile_network(&conn).unwrap(), None);
        // A profile exists but nothing is marked active.
        seed_profile(&conn, "p1");
        assert_eq!(get_active_profile_network(&conn).unwrap(), None);
    }

    #[test]
    fn active_profile_network_returns_the_stored_string() {
        let conn = db();
        seed_profile(&conn, "p1"); // network = "regtest"
        set_active_profile(&conn, "p1").unwrap();
        assert_eq!(
            get_active_profile_network(&conn).unwrap().as_deref(),
            Some("regtest")
        );
    }

    #[test]
    fn active_profile_network_is_none_when_active_id_points_at_a_deleted_profile() {
        let conn = db();
        seed_profile(&conn, "p1");
        set_active_profile(&conn, "p1").unwrap();
        delete_wallet_profile(&conn, "p1").unwrap();
        // The dangling active id must not error — it is simply "no profile".
        assert_eq!(get_active_profile_network(&conn).unwrap(), None);
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

    #[test]
    fn list_receive_addresses_returns_receive_only_with_used_flag() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Two receive addresses (idx 0 and 1) and one change address (must
        // be excluded). idx 0 is referenced by a tracked UTXO → used=true.
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex, created_at)
             VALUES ('p1',0,0,0,'rs1qrecv0','0014','02','2026-01-01T10:00:00'),
                    ('p1',0,0,1,'rs1qrecv1','0014','02','2026-01-01T10:01:00'),
                    ('p1',0,1,0,'rs1qchange','0014','02','2026-01-01T10:02:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES ('aa',0,'p1','rs1qrecv0','0014',1000,0,'liquid_hns')",
            [],
        )
        .unwrap();

        let rows = list_receive_addresses(&conn, "p1", 0).unwrap();
        // Change branch excluded; receive rows ordered by child_index.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].index, 0);
        assert_eq!(rows[0].address, "rs1qrecv0");
        assert!(rows[0].used, "recv0 has a UTXO");
        assert_eq!(rows[0].first_seen_at, "2026-01-01T10:00:00");
        assert_eq!(rows[1].index, 1);
        assert_eq!(rows[1].address, "rs1qrecv1");
        assert!(!rows[1].used, "recv1 has no UTXO or bid");
        assert_eq!(rows[1].first_seen_at, "2026-01-01T10:01:00");
    }

    #[test]
    fn list_receive_addresses_marks_bid_commitment_as_used() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex, created_at)
             VALUES ('p1',0,0,0,'rs1qbidder','0014','02','2026-01-01T10:00:00')",
            [],
        )
        .unwrap();
        // Seed a bid commitment referencing that address.
        conn.execute(
            "INSERT INTO bid_commitments
                (wallet_profile_id, name, name_hash_hex, address,
                 branch, child_index,
                 bid_value_doos, lockup_value_doos, nonce_hex, blind_hex)
             VALUES ('p1','name','aa','rs1qbidder',0,0,
                     1000,2000,'00','bb')",
            [],
        )
        .unwrap();

        let rows = list_receive_addresses(&conn, "p1", 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].used, "bid commitment counts as used");
        assert_eq!(rows[0].first_seen_at, "2026-01-01T10:00:00");
    }

    #[test]
    fn list_receive_addresses_empty_for_unknown_profile() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(list_receive_addresses(&conn, "does-not-exist", 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn next_unused_receive_address_returns_index_after_used() {
        use crate::noncustodial::derivation::{
            derive_one, next_unused_receive_address, BRANCH_RECEIVE,
        };
        use crate::noncustodial::hd::{seed_from_mnemonic, ExtendedPrivKey, ExtendedPubKey};
        use crate::noncustodial::network::Network;

        let conn = db();
        seed_profile(&conn, "p1");

        // Derive the test xpub (same as derivation tests).
        let seed = seed_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
        )
        .unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        let xpub = ExtendedPubKey::from_priv(&master);

        // Manually insert address at index 0 and mark it as used by a UTXO.
        let addr0 = derive_one(Network::Main, &xpub, BRANCH_RECEIVE, 0).unwrap();
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1', 0, 0, 0, ?1, ?2, ?3)",
            rusqlite::params![
                &addr0.address,
                &addr0.script_pubkey_hex,
                &addr0.public_key_hex
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES ('tx0', 0, 'p1', ?1, ?2, 1000, 0, 'liquid_hns')",
            rusqlite::params![&addr0.address, &addr0.script_pubkey_hex],
        )
        .unwrap();

        // With max(used) = 0, next_unused_receive_address should derive at index 1.
        let next = next_unused_receive_address(&conn, "p1", 0, Network::Main, &xpub).unwrap();
        assert_eq!(next.child_index, 1, "should derive at max(used)+1 = 1");
        assert_eq!(next.branch, BRANCH_RECEIVE);

        // Verify the new address was persisted to derived_addresses (sync window
        // is implicitly extended since get_profile_addresses reads this table).
        let persisted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM derived_addresses
                 WHERE wallet_profile_id = 'p1' AND child_index = 1 AND branch = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            persisted, 1,
            "new address must be in derived_addresses for sync"
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
        crate::tests::command_helpers::own_address(conn, "p1", "rs1qrecv");
    }

    #[test]
    fn cached_balance_maps_liquid_and_locked() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_utxo(&conn, "aa", 0, 1_000_000, "liquid_hns", 0);
        insert_utxo(&conn, "bb", 0, 3_000_000, "name_control", 6);
        insert_utxo(&conn, "cc", 0, 2_000_000, "name_lockup", 3);
        let bal =
            read_cached_balance(&conn, "p1", crate::noncustodial::network::Network::Main).unwrap();
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

    /// Only a transaction that has actually left the device and has not
    /// settled counts as "waiting for a block" — that is the window the UI has
    /// to narrate, because the chain still reports the name's old state
    /// throughout it.
    #[test]
    fn pending_broadcast_action_tracks_only_the_in_flight_window() {
        let conn = db();
        seed_profile(&conn, "p1");
        let pending = |name: &str| pending_broadcast_action_for_name(&conn, "p1", name).unwrap();

        insert_tx_draft(
            &conn,
            "d1",
            "p1",
            "reveal",
            "",
            "{}",
            r#"{"action":"reveal","name":"alpha"}"#,
        )
        .unwrap();
        // Built but not sent — nothing is in flight.
        assert_eq!(pending("alpha"), None);

        update_tx_draft_status(&conn, "d1", "signed", None, None).unwrap();
        assert_eq!(pending("alpha"), None, "signed is still on this device");

        update_tx_draft_status(&conn, "d1", "broadcasted", None, None).unwrap();
        assert_eq!(pending("alpha"), Some("reveal".to_string()));
        // Scoped to the name.
        assert_eq!(pending("beta"), None);

        update_tx_draft_status(&conn, "d1", "confirmed", None, None).unwrap();
        assert_eq!(pending("alpha"), None, "confirmed has settled");

        update_tx_draft_status(&conn, "d1", "dropped", None, None).unwrap();
        assert_eq!(pending("alpha"), None, "dropped has settled");
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
        // A name is considered owned only when the recorded owner outpoint is
        // an unspent wallet `name_control` UTXO. Seed the matching coin.
        seed_name_control_utxo(&conn, "p1", "txid1", 0, "rs1qaddr1");

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
        seed_name_control_utxo(&conn, "p1", "txid2", 1, "rs1qaddr2");
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
        seed_name_control_utxo(&conn, "p1", "txid1", 0, "rs1qoriginal");

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
    fn read_owned_names_explorer_excludes_bid_only_names() {
        // Regression test for the batch-bid owned-names leak:
        // `read_owned_names_explorer` should only return names whose recorded
        // owner outpoint is an unspent wallet `name_control` UTXO (the actual
        // owner coin). A BID places funds in a `name_lockup` coin, and node
        // discovery stamps the on-chain owner.hash (which is NOT the wallet's
        // owner coin) into tracked_name_states.owner_txid. Without the
        // ownership gate, a name the wallet only bid on leaks into "Owned Names".
        let conn = db();
        seed_profile(&conn, "p1");

        // Name A: actually owned. Insert a name_control UTXO at ("txid_owned", 0).
        let name_a = crate::hsd::types::HsdName {
            name: "owned.name".into(),
            name_hash: Some("hash_a".into()),
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
        upsert_owned_name(&conn, "p1", &name_a, "txid_owned", 0, "rs1qowned").unwrap();
        seed_name_control_utxo(&conn, "p1", "txid_owned", 0, "rs1qowned");

        // Name B: bid-only. Insert a name_lockup UTXO at ("txid_bid", 0)
        // (the BID coin), then upsert the name with owner_txid = "txid_bid".
        // This simulates what happens when node discovery processes a BID coin:
        // the wallet holds the name_lockup UTXO, but the on-chain owner is
        // someone else's outpoint (not in tracked_utxos).
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('txid_bid', 0, 'p1', 'rs1qbidder', '00', 500, 3, NULL, 'name_lockup', NULL)",
            [],
        )
        .unwrap();
        // Simulate what upsert_name_state does: record the on-chain owner
        // (which is NOT the wallet's owner coin, but some other address's coin).
        let bid_name_info = serde_json::json!({
            "info": {
                "name": "bid.name",
                "nameHash": "hash_b",
                "state": "BIDDING",
                "height": 100,
                "owner": {
                    "hash": "txid_other_owner",
                    "index": 0
                }
            }
        });
        crate::noncustodial::sync::upsert_name_state(&conn, "p1", "bid.name", &bid_name_info)
            .unwrap();

        // Call read_owned_names_explorer: should return ONLY name_a (owned),
        // NOT name_b (bid-only). The gate requires a matching unspent
        // name_control UTXO, which bid.name doesn't have.
        let names = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1, "Should return only the actually-owned name");
        assert_eq!(names[0]["name"], "owned.name");
        assert_eq!(names[0]["owner"]["hash"], "txid_owned");
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

    // --- Coverage: reachable branches flagged uncovered in Phase 4 ----------

    /// `insert_tx_draft_reserving_coins`: a coin already reserved by a *different*
    /// live draft cannot be stolen — the conditional UPDATE claims 0 rows, so
    /// the whole transaction rolls back with `InvalidInput` and draft B never
    /// persists.
    #[test]
    fn insert_tx_draft_reserving_coins_conflict_rolls_back() {
        let conn = db();
        seed_profile(&conn, "p1");
        // A spendable coin at (txid, vout).
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class, spent_by_txid)
             VALUES ('coinA', 0, 'p1', 'rs1qa', '00', 1000, 0, 'liquid_hns', NULL)",
            [],
        )
        .unwrap();
        let inputs = [("coinA".to_string(), 0u32)];
        // Draft A reserves the coin — succeeds.
        insert_tx_draft_reserving_coins(&conn, "dA", "p1", "send_hns", "", "[]", "{}", &inputs)
            .unwrap();
        // Draft B tries to claim the same coin — must fail and roll back.
        let err =
            insert_tx_draft_reserving_coins(&conn, "dB", "p1", "send_hns", "", "[]", "{}", &inputs)
                .unwrap_err();
        match err {
            AppError::InvalidInput(msg) => {
                assert!(msg.contains("reserved by another"), "got: {msg}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        // Draft B row was rolled back — it must not exist.
        assert!(get_tx_draft(&conn, "dB").unwrap().is_none());
        // Draft A still holds the reservation.
        assert!(get_tx_draft(&conn, "dA").unwrap().is_some());
    }

    /// Item 2 (queries.rs:525): no snapshot rows → `Ok(None)`.
    #[test]
    fn get_latest_wallet_snapshot_none_on_empty() {
        let conn = db();
        assert!(get_latest_wallet_snapshot(&conn).unwrap().is_none());
    }

    /// Item 3 (queries.rs:158): `update_asset` with every field `None` makes
    /// `sets` empty, so it early-returns `Ok(())` without issuing an UPDATE.
    #[test]
    fn update_asset_no_fields_is_noop() {
        let conn = db();
        seed_profile(&conn, "p1");
        // A no-op update against a non-existent id still succeeds because the
        // early return fires before any SQL runs.
        update_asset(&conn, 999, None, None, None, None, None, None, None).unwrap();
    }

    /// Item 4 (queries.rs:2113): `covenant_item_hex` returns `None` when the
    /// requested item is an empty string.
    #[test]
    fn covenant_item_hex_none_on_empty_item() {
        let cov = serde_json::json!({ "items": ["", "aa"] }).to_string();
        assert_eq!(covenant_item_hex(Some(&cov), 0), None);
        // Sanity: a non-empty sibling still decodes.
        assert_eq!(covenant_item_hex(Some(&cov), 1).as_deref(), Some("aa"));
    }

    /// Item 5 (queries.rs:2307): a name-covenant coin whose covenant_json has
    /// no items[0] (no name hash) is skipped via `None => continue`.
    #[test]
    fn list_unspent_wallet_name_hashes_skips_hashless_coin() {
        let conn = db();
        seed_profile(&conn, "p1");
        // A name_control coin with a covenant that has an empty items array —
        // covenant_name_hash_hex returns None, so the row is skipped.
        let cov = serde_json::json!({ "type": 6, "action": "REGISTER", "items": [] }).to_string();
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('t1', 0, 'p1', 'rs1qa', '00', 1000, 6, ?1, 'name_control', NULL)",
            params![cov],
        )
        .unwrap();
        let out = list_unspent_wallet_name_hashes(&conn, "p1").unwrap();
        assert!(out.is_empty(), "hashless coin must be skipped: {out:?}");
    }

    /// Item 6 (queries.rs:2325): rawName dedup — when a hashless-rawName coin
    /// (REVEAL) is seen FIRST and a rawName-carrying coin (BID) for the same
    /// hash is seen SECOND, the `and_modify` branch upgrades the stored entry.
    #[test]
    fn list_unspent_wallet_name_hashes_upgrades_rawname_later() {
        let conn = db();
        seed_profile(&conn, "p1");
        let raw = "6e616d65686f6c64"; // "namehold"
                                      // REVEAL first (items[2] is a nonce, not read as rawName → None).
        let reveal = serde_json::json!({
            "type": 4, "action": "REVEAL", "items": ["hashA", "64000000", "nonce"],
        })
        .to_string();
        // BID second, carrying rawName at items[2].
        let bid = serde_json::json!({
            "type": 3, "action": "BID", "items": ["hashA", "64000000", raw, "blind"],
        })
        .to_string();
        // Insert order controls scan order: t1 (REVEAL) < t2 (BID) by txid.
        for (txid, cov_type, cov) in [("t1", 4i64, &reveal), ("t2", 3i64, &bid)] {
            conn.execute(
                "INSERT INTO tracked_utxos
                    (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                     value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
                 VALUES (?1, 0, 'p1', 'rs1qa', '00', 1000, ?2, ?3, 'name_lockup', NULL)",
                params![txid, cov_type, cov],
            )
            .unwrap();
        }
        let out = list_unspent_wallet_name_hashes(&conn, "p1").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name_hash_hex, "hasha");
        // The rawName from the later BID was merged in over the earlier None.
        assert_eq!(out[0].raw_name_hex.as_deref(), Some(raw));
    }

    /// Item 7a (queries.rs:2183): `find_unspent_covenant_utxo` ignores a coin
    /// belonging to a *different* name hash (`Some(_) => {}`), returning None
    /// when nothing matches the requested hash.
    #[test]
    fn find_unspent_covenant_utxo_ignores_other_name() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,0,'rs1qa','0014','02')",
            [],
        )
        .unwrap();
        // Coin at rs1qa carries name hash "otherhash", not the "wanthash" we ask for.
        let cov = serde_json::json!({
            "type": 3, "action": "BID", "items": ["otherhash", "64000000", "72", "bl"],
        })
        .to_string();
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('t1', 0, 'p1', 'rs1qa', '00', 1000, 3, ?1, 'name_lockup', NULL)",
            params![cov],
        )
        .unwrap();
        let got = find_unspent_covenant_utxo(&conn, "p1", "rs1qa", 3, "want", "wanthash").unwrap();
        assert!(got.is_none(), "other-name coin must be ignored: {got:?}");
    }

    /// Item 7b (queries.rs:2188): the lone-unknown-covenant fallback — a single
    /// candidate whose covenant_json is NULL (unreadable) is returned as-is.
    #[test]
    fn find_unspent_covenant_utxo_lone_unknown_fallback() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,0,'rs1qa','0014','02')",
            [],
        )
        .unwrap();
        // Single coin at rs1qa with covenant_type 3 but NULL covenant_json —
        // covenant_name_hash_hex returns None → it lands in `unknown`.
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('t1', 0, 'p1', 'rs1qa', '00', 1000, 3, NULL, 'name_lockup', NULL)",
            [],
        )
        .unwrap();
        let got = find_unspent_covenant_utxo(&conn, "p1", "rs1qa", 3, "want", "wanthash").unwrap();
        assert!(
            got.is_some(),
            "lone unknown-covenant coin should be returned"
        );
        assert_eq!(got.unwrap().txid, "t1");
    }

    /// Item 8 (queries.rs:1553): `draft_summary_covers_name` returns false when
    /// the summary JSON is unparseable.
    #[test]
    fn draft_summary_covers_name_false_on_bad_json() {
        assert!(!draft_summary_covers_name("{not valid json", "foo"));
        // Sanity: valid JSON with a matching name field returns true.
        assert!(draft_summary_covers_name(r#"{"name":"foo"}"#, "foo"));
    }

    // ── Phase 5: Additional coverage tests to reach ~100% ──────────────────

    /// Coverage: read_cached_transactions — "other" direction (no our addr, no our spend)
    #[test]
    fn read_cached_transactions_other_direction() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Cache a transaction with neither our address nor our UTXO
        cache_transaction(
            &conn,
            "p1",
            "other_tx",
            Some(102),
            None,
            r#"{"outputs":[{"value":100000,"address":"rs1qother"}],"inputs":[]}"#,
        )
        .unwrap();

        let txs = read_cached_transactions(&conn, "p1").unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0]["direction"], "other");
        assert_eq!(txs[0]["value"], 0);
        assert_eq!(txs[0]["address"], "");
    }

    /// Coverage: read_cached_transactions — unconfirmed (NULL height)
    #[test]
    fn read_cached_transactions_unconfirmed() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,0,'rs1qaddr','0014','02')",
            [],
        )
        .unwrap();
        cache_transaction(
            &conn,
            "p1",
            "unconf_tx",
            None,
            None,
            r#"{"outputs":[{"value":500000,"address":"rs1qaddr"}],"inputs":[]}"#,
        )
        .unwrap();

        let txs = read_cached_transactions(&conn, "p1").unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0]["confirmed"], false);
        assert_eq!(txs[0]["height"], serde_json::Value::Null);
        assert_eq!(txs[0]["direction"], "receive");
    }

    /// Coverage: read_cached_transactions — send with multiple foreign outputs
    #[test]
    fn read_cached_transactions_send_multiple_outputs() {
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
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES ('prevtx', 0, 'p1', 'rs1qmine', '0014', 500000, 0, 'liquid_hns')",
            [],
        )
        .unwrap();
        // Send with multiple foreign outputs — sent_outputs sums all non-ours
        cache_transaction(
            &conn,
            "p1",
            "multi_send",
            Some(200),
            None,
            r#"{"outputs":[{"value":100000,"address":"rs1qfirst"},{"value":50000,"address":"rs1qsecond"},{"value":340000,"address":"rs1qmine"}],"inputs":[{"prevout":{"hash":"prevtx","index":0}}]}"#,
        )
        .unwrap();

        let txs = read_cached_transactions(&conn, "p1").unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0]["direction"], "send");
        // sent_outputs = 100000 + 50000 = 150000 (only non-ours outputs)
        assert_eq!(txs[0]["value"], 150000);
        assert_eq!(txs[0]["address"], "rs1qfirst"); // first non-ours address
    }

    /// Coverage: row_to_profile with has_passphrase=1 branch, receive_address set, last_synced fields
    #[test]
    fn row_to_profile_with_passphrase_and_fields() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Insert a secret row with kdf != "none" → has_passphrase = true
        conn.execute(
            "INSERT INTO wallet_secrets
                (wallet_profile_id, kdf, kdf_salt_hex, nonce_hex, ciphertext_hex, public_fingerprint)
             VALUES ('p1', 'pbkdf2', 'salt', 'nonce', 'aabb', 'fp')",
            [],
        )
        .unwrap();
        // Populate receive_address and last_synced fields
        conn.execute(
            "UPDATE wallet_profiles SET receive_address = 'rs1qreceive', last_synced_at = datetime('now'), last_synced_height = 100 WHERE id = 'p1'",
            [],
        )
        .unwrap();
        set_active_profile(&conn, "p1").unwrap();

        let profile = get_wallet_profile(&conn, "p1").unwrap().unwrap();
        assert_eq!(profile.id, "p1");
        assert_eq!(profile.receive_address.as_deref(), Some("rs1qreceive"));
        assert_eq!(profile.last_synced_height, Some(100));
        assert!(profile.last_synced_at.is_some());
        assert!(profile.has_passphrase);
        assert!(profile.active);
    }

    /// Coverage: list_wallet_profiles exercises row_to_profile on multiple rows
    #[test]
    fn list_wallet_profiles_multiple() {
        let conn = db();
        seed_profile(&conn, "p1");
        seed_profile(&conn, "p2");
        set_active_profile(&conn, "p2").unwrap();

        let profiles = list_wallet_profiles(&conn).unwrap();
        assert_eq!(profiles.len(), 2);
        let p2 = profiles.iter().find(|p| p.id == "p2").unwrap();
        assert!(p2.active);
        let p1 = profiles.iter().find(|p| p.id == "p1").unwrap();
        assert!(!p1.active);
    }

    /// Coverage: read_owned_names_explorer — both ownership signals
    #[test]
    fn read_owned_names_explorer_both_signals() {
        let conn = db();
        seed_profile(&conn, "p1");

        // Name A: owner_address IS NOT NULL (explorer-verified path, no UTXO needed)
        let name_a = crate::hsd::types::HsdName {
            name: "explorer_owned".into(),
            name_hash: Some("hash_a".into()),
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
        upsert_owned_name(&conn, "p1", &name_a, "txid_a", 0, "rs1qexplorer").unwrap();

        // Name B: owner_address IS NULL but matching unspent name_control UTXO
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address)
             VALUES ('p1', 'node_owned', 'hash_b', 'CLOSED', 'txid_b', 0, NULL)",
            [],
        )
        .unwrap();
        seed_name_control_utxo(&conn, "p1", "txid_b", 0, "rs1qnode");

        let names = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names.len(), 2);
        let by_name = |n: &str| names.iter().find(|x| x["name"] == n).unwrap().clone();
        assert_eq!(by_name("explorer_owned")["owner_address"], "rs1qexplorer");
        // node_owned has NULL owner_address
        assert_eq!(
            by_name("node_owned")["owner_address"],
            serde_json::Value::Null
        );
    }

    /// Coverage: read_owned_names_explorer — registered/expired extraction from raw_json
    #[test]
    fn read_owned_names_explorer_registered_expired_from_raw_json() {
        let conn = db();
        seed_profile(&conn, "p1");

        // raw_json with explicit registered=true, expired=false
        let raw_json = serde_json::json!({
            "name": "testname",
            "state": "CLOSED",
            "registered": true,
            "expired": false,
            "renewal": 500
        })
        .to_string();
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
                 owner_address, height, renewal_height, raw_json)
             VALUES ('p1', 'testname', 'hash1', 'CLOSED', 'txid1', 0, 'rs1qowner', 100, 500, ?1)",
            params![raw_json],
        )
        .unwrap();

        let names = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0]["registered"], true);
        assert_eq!(names[0]["expired"], false);
    }

    /// Coverage: read_owned_names_explorer — derived registered from CLOSED state + renewal
    #[test]
    fn read_owned_names_explorer_derived_registered() {
        let conn = db();
        seed_profile(&conn, "p1");

        // raw_json WITHOUT registered field but CLOSED state + renewal > 0 → derived registered = true
        let raw_json = serde_json::json!({
            "name": "derived_reg",
            "state": "CLOSED",
            "renewal": 500
        })
        .to_string();
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
                 owner_address, height, renewal_height, raw_json)
             VALUES ('p1', 'derived_reg', 'hash2', 'CLOSED', 'txid2', 0, 'rs1qowner', 100, 500, ?1)",
            params![raw_json],
        )
        .unwrap();

        let names = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0]["registered"], true);
    }

    /// Coverage: read_cached_names — registered branch (covenant_type >= 6)
    #[test]
    fn read_cached_names_registered_branch() {
        let conn = db();
        seed_profile(&conn, "p1");

        upsert_name_state(
            &conn,
            "p1",
            "registered_name",
            &serde_json::json!({"info":{"name":"registered_name","state":"CLOSED","height":100,"renewal":200}}),
        )
        .unwrap();

        // Insert a name_control UTXO with covenant_type = 6 (>= 6 → registered)
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class, spent_by_txid)
             VALUES ('txid_reg', 0, 'p1', 'rs1qaddr', '0014', 1000, 6, 'name_control', NULL)",
            [],
        )
        .unwrap();

        conn.execute(
            "UPDATE tracked_name_states SET owner_txid = 'txid_reg', owner_vout = 0 WHERE name = 'registered_name'",
            [],
        )
        .unwrap();

        let names = read_cached_names(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0]["name"], "registered_name");
        assert_eq!(names[0]["registered"], true);
    }

    /// Coverage: read_cached_names — not-registered branch (covenant_type < 6)
    #[test]
    fn read_cached_names_not_registered_branch() {
        let conn = db();
        seed_profile(&conn, "p1");

        upsert_name_state(
            &conn,
            "p1",
            "won_name",
            &serde_json::json!({"info":{"name":"won_name","state":"CLOSED","height":100,"renewal":200}}),
        )
        .unwrap();

        // Insert a REVEAL UTXO with covenant_type = 4 (< 6 → not registered)
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class, spent_by_txid)
             VALUES ('txid_rev', 0, 'p1', 'rs1qaddr', '0014', 1000, 4, 'name_control', NULL)",
            [],
        )
        .unwrap();

        conn.execute(
            "UPDATE tracked_name_states SET owner_txid = 'txid_rev', owner_vout = 0 WHERE name = 'won_name'",
            [],
        )
        .unwrap();

        let names = read_cached_names(&conn, "p1").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0]["registered"], false);
    }

    /// Coverage: get_recent_audit_log after triggering audit writes
    #[test]
    fn get_recent_audit_log_after_writes() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('a','not_started')",
            [],
        )
        .unwrap();
        let id1 = conn.last_insert_rowid();
        update_asset(
            &conn,
            id1,
            Some("finalized_owned"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        bulk_update_status(&conn, &[id1], "waiting_finalize").unwrap();

        let log = get_recent_audit_log(&conn, 10).unwrap();
        assert!(log.len() >= 2);
        // Most recent first
        assert_eq!(log[0]["action"], "bulk_status_change");
        assert_eq!(log[1]["action"], "asset_update");
    }

    /// Coverage: get_name_coin returns Some with all fields populated
    #[test]
    fn get_name_coin_returns_some() {
        let conn = db();
        seed_profile(&conn, "p1");

        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,5,'rs1qname','0014','02')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class, spent_by_txid)
             VALUES ('nametx', 0, 'p1', 'rs1qname', '0014', 2000, 6, 'name_control', NULL)",
            [],
        )
        .unwrap();

        upsert_name_state(
            &conn,
            "p1",
            "myname",
            &serde_json::json!({"info":{"name":"myname","state":"CLOSED","height":100}}),
        )
        .unwrap();

        conn.execute(
            "UPDATE tracked_name_states SET owner_txid = 'nametx', owner_vout = 0 WHERE name = 'myname'",
            [],
        )
        .unwrap();

        let coin = get_name_coin(&conn, "p1", "myname").unwrap().unwrap();
        assert_eq!(coin.txid, "nametx");
        assert_eq!(coin.vout, 0);
        assert_eq!(coin.value, 2000);
        assert_eq!(coin.address, "rs1qname");
        assert_eq!(coin.branch, 0);
        assert_eq!(coin.child_index, 5);
        assert_eq!(coin.covenant_type, 6);
        assert_eq!(coin.name_height, Some(100));
    }

    /// Coverage: get_wallet_snapshots ordering (newest first) and fields
    #[test]
    fn get_wallet_snapshots_ordering_and_fields() {
        let conn = db();
        insert_wallet_snapshot(&conn, "w1", 100, Some("rs1qa"), 1, None).unwrap();
        insert_wallet_snapshot(&conn, "w1", 200, None, 2, None).unwrap();
        insert_wallet_snapshot(&conn, "w1", 300, Some("rs1qc"), 3, None).unwrap();

        let snaps = get_wallet_snapshots(&conn, 10).unwrap();
        assert_eq!(snaps.len(), 3);
        // Newest first (highest id)
        assert_eq!(snaps[0]["balance"], 300);
        assert_eq!(snaps[0]["address"], "rs1qc");
        assert_eq!(snaps[0]["name_count"], 3);
        assert_eq!(snaps[1]["balance"], 200);
        assert_eq!(snaps[1]["address"], serde_json::Value::Null);
        assert_eq!(snaps[2]["balance"], 100);
    }

    /// Coverage: get_latest_wallet_snapshot returns most recent with fields
    #[test]
    fn get_latest_wallet_snapshot_with_fields() {
        let conn = db();
        insert_wallet_snapshot(&conn, "w1", 100, Some("rs1qa"), 1, None).unwrap();
        insert_wallet_snapshot(&conn, "w2", 200, Some("rs1qb"), 2, None).unwrap();

        let snap = get_latest_wallet_snapshot(&conn).unwrap().unwrap();
        assert_eq!(snap["wallet_name"], "w2");
        assert_eq!(snap["balance"], 200);
        assert_eq!(snap["address"], "rs1qb");
        assert_eq!(snap["name_count"], 2);
    }

    /// Coverage: row_to_name_coin via find_unspent_covenant_utxo with a match
    #[test]
    fn row_to_name_coin_via_find_unspent_match() {
        let conn = db();
        seed_profile(&conn, "p1");

        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,1,3,'rs1qcoin','0014','02')",
            [],
        )
        .unwrap();

        let cov = serde_json::json!({
            "type": 3, "action": "BID",
            "items": ["namehash1", "64000000", "72", "blind"],
        })
        .to_string();

        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES ('bidtx', 2, 'p1', 'rs1qcoin', '00', 5000, 3, ?1, 'name_lockup', NULL)",
            params![cov],
        )
        .unwrap();

        let coin = find_unspent_covenant_utxo(&conn, "p1", "rs1qcoin", 3, "name", "namehash1")
            .unwrap()
            .unwrap();
        assert_eq!(coin.txid, "bidtx");
        assert_eq!(coin.vout, 2);
        assert_eq!(coin.value, 5000);
        assert_eq!(coin.branch, 1);
        assert_eq!(coin.child_index, 3);
        assert_eq!(coin.covenant_type, 3);
        assert!(coin.covenant_json.as_deref().is_some());
    }

    /// Coverage: list_receive_addresses with used/unused detection
    #[test]
    fn list_receive_addresses_used_unused() {
        let conn = db();
        seed_profile(&conn, "p1");

        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,0,'rs1qused','0014','02'),
                    ('p1',0,0,1,'rs1qfresh','0014','02')",
            [],
        )
        .unwrap();

        // Mark first address as used by a UTXO
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES ('tx0', 0, 'p1', 'rs1qused', '0014', 1000, 0, 'liquid_hns')",
            [],
        )
        .unwrap();

        let addrs = list_receive_addresses(&conn, "p1", 0).unwrap();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].index, 0);
        assert_eq!(addrs[0].address, "rs1qused");
        assert!(addrs[0].used);
        assert_eq!(addrs[1].index, 1);
        assert_eq!(addrs[1].address, "rs1qfresh");
        assert!(!addrs[1].used);
    }

    /// Coverage: get_tracked_name_state returns Some with all fields
    #[test]
    fn get_tracked_name_state_returns_some() {
        let conn = db();
        seed_profile(&conn, "p1");

        upsert_name_state(
            &conn,
            "p1",
            "tracked",
            &serde_json::json!({"info":{"name":"tracked","state":"CLOSED","renewal":200}}),
        )
        .unwrap();

        let row = get_tracked_name_state(&conn, "p1", "tracked")
            .unwrap()
            .unwrap();
        assert_eq!(row.name, "tracked");
        assert_eq!(row.state.as_deref(), Some("CLOSED"));
        assert_eq!(row.renewal_height, Some(200));
    }

    /// Coverage: get_tracked_name_state returns None for missing
    #[test]
    fn get_tracked_name_state_returns_none() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(get_tracked_name_state(&conn, "p1", "nonexistent")
            .unwrap()
            .is_none());
    }

    /// Coverage: delete_tx_draft releases coins and refuses broadcasted
    #[test]
    fn delete_tx_draft_releases_and_refuses_broadcasted() {
        let conn = db();
        seed_profile(&conn, "p1");

        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES ('coin1', 0, 'p1', 'rs1q', '00', 1000, 0, 'liquid_hns')",
            [],
        )
        .unwrap();

        let inputs = [("coin1".to_string(), 0u32)];
        insert_tx_draft_reserving_coins(&conn, "d1", "p1", "send_hns", "", "[]", "{}", &inputs)
            .unwrap();

        // Can delete a draft-status draft
        delete_tx_draft(&conn, "d1").unwrap();
        assert!(get_tx_draft(&conn, "d1").unwrap().is_none());

        // Coin reservation released
        let reserved: Option<String> = conn
            .query_row(
                "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = 'coin1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reserved, None);

        // Cannot delete a broadcasted draft
        insert_tx_draft(&conn, "d2", "p1", "send_hns", "", "{}", "{}").unwrap();
        update_tx_draft_status(&conn, "d2", "broadcasted", None, Some("txid1")).unwrap();
        let err = delete_tx_draft(&conn, "d2").unwrap_err();
        match err {
            AppError::InvalidInput(msg) => assert!(msg.contains("broadcast")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    /// Coverage: list_assets with sort by various columns and desc direction
    #[test]
    fn list_assets_sort_columns_and_desc() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('zzz','not_started')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('aaa','finalized_owned')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('mmm','waiting_finalize')",
            [],
        )
        .unwrap();

        // Sort by tld ascending
        let by_tld = list_assets(&conn, None, None, None, Some("tld"), Some("asc")).unwrap();
        assert_eq!(by_tld[0].tld, "aaa");
        assert_eq!(by_tld[2].tld, "zzz");

        // Sort by tld descending
        let by_tld_desc = list_assets(&conn, None, None, None, Some("tld"), Some("desc")).unwrap();
        assert_eq!(by_tld_desc[0].tld, "zzz");
        assert_eq!(by_tld_desc[2].tld, "aaa");

        // Sort by status
        let by_status = list_assets(&conn, None, None, None, Some("status"), None).unwrap();
        assert_eq!(by_status.len(), 3);
    }

    /// Coverage: list_assets with search + status + staked combined
    #[test]
    fn list_assets_combined_filters() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld, status, is_staked, notes) VALUES ('match','finalized_owned',1,'target note')", []).unwrap();
        conn.execute("INSERT INTO assets (tld, status, is_staked, notes) VALUES ('nomatch','not_started',0,'target note')", []).unwrap();

        let results = list_assets(
            &conn,
            Some("finalized_owned"),
            Some(true),
            Some("target"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tld, "match");
    }

    /// Coverage: update_asset with all Option fields Some
    #[test]
    fn update_asset_all_fields_some() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('test','not_started')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();

        update_asset(
            &conn,
            id,
            Some("finalized_owned"),
            Some("premium"),
            Some(r#"["tag1","tag2"]"#),
            Some("a note"),
            Some(100),
            Some("txhash1"),
            Some("txhash2"),
        )
        .unwrap();

        let asset = get_asset(&conn, id).unwrap();
        assert_eq!(asset.status.as_str(), "finalized_owned");
        assert_eq!(asset.category.as_deref(), Some("premium"));
        assert_eq!(asset.tags, vec!["tag1".to_string(), "tag2".to_string()]);
        assert_eq!(asset.notes.as_deref(), Some("a note"));
        assert_eq!(asset.hns_received, Some(100));
    }

    /// Coverage: update_batch with empty fields (early return)
    #[test]
    fn update_batch_empty_noop() {
        let conn = db();
        let batch_id = create_batch(&conn, "batch1", None, &[]).unwrap();
        // All None → early return Ok(())
        update_batch(&conn, batch_id, None, None, None).unwrap();
        // Verify name unchanged
        let name: String = conn
            .query_row(
                "SELECT name FROM batches WHERE id = ?1",
                params![batch_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "batch1");
    }

    /// Coverage: update_batch with all fields populated
    #[test]
    fn update_batch_all_fields() {
        let conn = db();
        let batch_id = create_batch(&conn, "batch1", Some("desc1"), &[]).unwrap();

        update_batch(
            &conn,
            batch_id,
            Some("new_name"),
            Some("new_desc"),
            Some("in_progress"),
        )
        .unwrap();

        let (name, desc, status): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT name, description, status FROM batches WHERE id = ?1",
                params![batch_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "new_name");
        assert_eq!(desc.as_deref(), Some("new_desc"));
        assert_eq!(status.as_deref(), Some("in_progress"));
    }

    /// Coverage: bulk_update_status writes audit log
    #[test]
    fn bulk_update_status_audit_log() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('a','not_started')",
            [],
        )
        .unwrap();
        let id1 = conn.last_insert_rowid();

        bulk_update_status(&conn, &[id1], "waiting_finalize").unwrap();

        let log = get_recent_audit_log(&conn, 10).unwrap();
        assert!(log.iter().any(|e| e["action"] == "bulk_status_change"));
    }

    /// Coverage: bulk_update_tags writes audit log
    #[test]
    fn bulk_update_tags_audit_log() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('a')", [])
            .unwrap();
        let id1 = conn.last_insert_rowid();

        bulk_update_tags(&conn, &[id1], "newtag").unwrap();

        let log = get_recent_audit_log(&conn, 10).unwrap();
        assert!(log.iter().any(|e| e["action"] == "bulk_tag_change"));
    }

    /// Coverage: get_batch_with_assets with multiple assets in order
    #[test]
    fn get_batch_with_assets_order() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('aaa')", [])
            .unwrap();
        let id1 = conn.last_insert_rowid();
        conn.execute("INSERT INTO assets (tld) VALUES ('bbb')", [])
            .unwrap();
        let id2 = conn.last_insert_rowid();
        conn.execute("INSERT INTO assets (tld) VALUES ('ccc')", [])
            .unwrap();
        let id3 = conn.last_insert_rowid();

        let batch_id = create_batch(&conn, "ordered", None, &[id3, id1, id2]).unwrap();

        let batch = get_batch_with_assets(&conn, batch_id).unwrap();
        assert_eq!(batch.assets.len(), 3);
        // Order matches insertion order (sort_order)
        assert_eq!(batch.assets[0].tld, "ccc");
        assert_eq!(batch.assets[1].tld, "aaa");
        assert_eq!(batch.assets[2].tld, "bbb");
    }

    /// Coverage: add_to_batch respects sort_order continuation
    #[test]
    fn add_to_batch_sort_order() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('a')", [])
            .unwrap();
        let id1 = conn.last_insert_rowid();
        conn.execute("INSERT INTO assets (tld) VALUES ('b')", [])
            .unwrap();
        let id2 = conn.last_insert_rowid();
        conn.execute("INSERT INTO assets (tld) VALUES ('c')", [])
            .unwrap();
        let id3 = conn.last_insert_rowid();

        let batch_id = create_batch(&conn, "batch", None, &[id1]).unwrap();
        add_to_batch(&conn, batch_id, &[id2, id3]).unwrap();

        let batch = get_batch_with_assets(&conn, batch_id).unwrap();
        assert_eq!(batch.assets.len(), 3);
        assert_eq!(batch.assets[0].tld, "a");
        assert_eq!(batch.assets[1].tld, "b");
        assert_eq!(batch.assets[2].tld, "c");
    }

    /// Coverage: remove_from_batch removes specific assets
    #[test]
    fn remove_from_batch_specific() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('a')", [])
            .unwrap();
        let id1 = conn.last_insert_rowid();
        conn.execute("INSERT INTO assets (tld) VALUES ('b')", [])
            .unwrap();
        let id2 = conn.last_insert_rowid();

        let batch_id = create_batch(&conn, "batch", None, &[id1, id2]).unwrap();
        let removed = remove_from_batch(&conn, batch_id, &[id1]).unwrap();
        assert_eq!(removed, 1);

        let batch = get_batch_with_assets(&conn, batch_id).unwrap();
        assert_eq!(batch.assets.len(), 1);
        assert_eq!(batch.assets[0].tld, "b");
    }

    /// Coverage: list_pending_reveal_deadlines with revealed bids excluded
    /// Stamping a reveal must mark the ONE bid it revealed.
    ///
    /// The column was keyed by name, from when a wallet could hold only one bid
    /// per name. Once several were allowed, revealing one marked them all — and
    /// `list_pending_reveal_deadlines` filters on `reveal_txid IS NULL`, so the
    /// bids that were NOT revealed stopped being warned about, right up to the
    /// block where their lockup became unreclaimable.
    #[test]
    fn set_bid_reveal_txid_marks_only_its_own_commitment() {
        let conn = db();
        seed_profile(&conn, "p1");
        for (blind, lockup) in [("b1", 200), ("b2", 300), ("b3", 400)] {
            insert_bid_commitment(
                &conn, "p1", "multi", "h", "rs1q", 0, 0, 100, lockup, "n", blind,
            )
            .unwrap();
            set_auction_heights(&conn, "p1", blind, 100, 121).unwrap();
        }

        set_bid_reveal_txid(&conn, "p1", "multi", "b2", "revealtx").unwrap();

        let revealed: Vec<String> = list_bid_commitments(&conn, "p1")
            .unwrap()
            .into_iter()
            .filter(|b| b.reveal_txid.is_some())
            .map(|b| b.blind_hex)
            .collect();
        assert_eq!(revealed, vec!["b2".to_string()], "only the revealed bid");

        // The other two must still be chased by the deadline scanner.
        let pending = list_pending_reveal_deadlines(&conn).unwrap();
        assert_eq!(
            pending.len(),
            2,
            "the unrevealed bids must keep their deadline warning"
        );
    }

    #[test]
    fn list_pending_reveal_deadlines_excludes_revealed() {
        let conn = db();
        seed_profile(&conn, "p1");

        insert_bid_commitment(
            &conn,
            "p1",
            "unrevealed",
            "h1",
            "rs1q",
            0,
            0,
            100,
            200,
            "n1",
            "b1",
        )
        .unwrap();
        insert_bid_commitment(
            &conn, "p1", "revealed", "h2", "rs1q", 0, 0, 100, 200, "n2", "b2",
        )
        .unwrap();

        set_auction_heights(&conn, "p1", "b1", 0, 500).unwrap();
        set_auction_heights(&conn, "p1", "b2", 0, 600).unwrap();

        // Mark one as revealed
        set_bid_reveal_txid(&conn, "p1", "revealed", "b2", "reveal_txid").unwrap();

        let deadlines = list_pending_reveal_deadlines(&conn).unwrap();
        assert_eq!(deadlines.len(), 1);
        assert_eq!(deadlines[0].1, "unrevealed");
    }

    /// Coverage: list_bid_commitments with bid_txid and reveal_txid populated
    #[test]
    fn list_bid_commitments_with_txids() {
        let conn = db();
        seed_profile(&conn, "p1");

        insert_bid_commitment(
            &conn, "p1", "name1", "hash1", "rs1q", 0, 0, 100, 200, "nonce", "blind1",
        )
        .unwrap();
        set_bid_txid(&conn, "p1", "blind1", "bid_txid_1").unwrap();
        set_bid_reveal_txid(&conn, "p1", "name1", "blind1", "reveal_txid_1").unwrap();

        let bids = list_bid_commitments(&conn, "p1").unwrap();
        assert_eq!(bids.len(), 1);
        assert_eq!(bids[0].bid_txid.as_deref(), Some("bid_txid_1"));
        assert_eq!(bids[0].reveal_txid.as_deref(), Some("reveal_txid_1"));
    }

    /// Coverage: auction_position_names includes draft-based names
    #[test]
    fn auction_position_names_includes_drafts() {
        let conn = db();
        seed_profile(&conn, "p1");

        // Insert a broadcasted bid draft
        insert_tx_draft(
            &conn,
            "d1",
            "p1",
            "bid",
            "",
            "{}",
            r#"{"action":"bid","name":"bidname"}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("txid1")).unwrap();

        // Insert a signed open draft
        insert_tx_draft(
            &conn,
            "d2",
            "p1",
            "open",
            "",
            "{}",
            r#"{"action":"open","name":"openname"}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "d2", "signed", None, None).unwrap();

        // Insert a draft-status (not in-flight) — should be excluded
        insert_tx_draft(
            &conn,
            "d3",
            "p1",
            "bid",
            "",
            "{}",
            r#"{"action":"bid","name":"draftonly"}"#,
        )
        .unwrap();

        let positions = auction_position_names(&conn, "p1").unwrap();
        assert!(positions.contains(&"bidname".to_string()));
        assert!(positions.contains(&"openname".to_string()));
        assert!(!positions.contains(&"draftonly".to_string()));
    }

    /// Regression: a `dropped` open draft (broadcast, evicted/reorg'd out, then
    /// judged `dropped`) must NOT appear as an active auction position, while a
    /// name with a live in-flight open draft plus a confirmed bid still does.
    ///
    /// Mirrors an observed regtest sequence: `vmp3rt4`'s open tx was broadcast
    /// but never confirmed, so its draft settled to `dropped`; `vmp3rt3`'s open
    /// confirmed and its bid confirmed. Only `vmp3rt3` should show "In Auction".
    #[test]
    fn auction_position_names_excludes_dropped_open_draft() {
        let conn = db();
        seed_profile(&conn, "p1");

        // vmp3rt3: open draft reaches `confirmed` (in-flight status) — kept.
        insert_tx_draft(
            &conn,
            "open3",
            "p1",
            "open",
            "",
            "",
            r#"{"action":"open","name":"vmp3rt3"}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "open3", "confirmed", None, Some("txid_open3")).unwrap();

        // vmp3rt3: bid draft also confirmed — reinforces the position.
        insert_tx_draft(
            &conn,
            "bid3",
            "p1",
            "bid",
            "",
            "{}",
            r#"{"action":"bid","name":"vmp3rt3"}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "bid3", "confirmed", None, Some("txid_bid3")).unwrap();

        // vmp3rt4: open draft was broadcast then judged `dropped` (evicted /
        // reorg'd out, never landed). Not an in-flight status → must be excluded.
        insert_tx_draft(
            &conn,
            "open4",
            "p1",
            "open",
            "",
            "{}",
            r#"{"action":"open","name":"vmp3rt4"}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "open4", "dropped", None, Some("txid_open4")).unwrap();

        let positions = auction_position_names(&conn, "p1").unwrap();
        assert!(
            positions.contains(&"vmp3rt3".to_string()),
            "vmp3rt3 has an in-flight open + confirmed bid and must show as a position"
        );
        assert!(
            !positions.contains(&"vmp3rt4".to_string()),
            "vmp3rt4's only draft is `dropped`; it must not show as an active auction"
        );
    }

    /// Regression guard: every non-in-flight draft status is excluded from
    /// auction positions, so a fix here can never silently start surfacing
    /// abandoned/failed/queued drafts.
    #[test]
    fn auction_position_names_excludes_all_non_in_flight_statuses() {
        // The `status` CHECK constraint permits only these values; of them,
        // `draft`/`dropped`/`failed` are the non-in-flight ones.
        for (idx, status) in ["draft", "dropped", "failed"].iter().enumerate() {
            let conn = db();
            seed_profile(&conn, "p1");
            let id = format!("d{idx}");
            let name = format!("name{idx}");
            insert_tx_draft(
                &conn,
                &id,
                "p1",
                "open",
                "",
                "{}",
                &format!(r#"{{"action":"open","name":"{name}"}}"#),
            )
            .unwrap();
            // `draft` is the default; the others are set explicitly.
            if *status != "draft" {
                update_tx_draft_status(&conn, &id, status, None, Some("txid")).unwrap();
            }
            let positions = auction_position_names(&conn, "p1").unwrap();
            assert!(
                !positions.contains(&name),
                "status `{status}` is not in-flight and must be excluded"
            );
        }
    }

    /// Coverage: count_repair_candidates
    #[test]
    fn count_repair_candidates_matches_list() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute("INSERT INTO assets (tld) VALUES ('a')", [])
            .unwrap();
        conn.execute("INSERT INTO assets (tld) VALUES ('b')", [])
            .unwrap();

        let count = count_repair_candidates(&conn, "p1", 12).unwrap();
        let list = list_repair_candidates(&conn, "p1", 100, 12).unwrap();
        assert_eq!(count as usize, list.len());
    }

    /// Coverage: insert_tx_draft_reserving_coins with multiple inputs
    #[test]
    fn insert_tx_draft_reserving_multiple_coins() {
        let conn = db();
        seed_profile(&conn, "p1");

        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class)
             VALUES ('coin1', 0, 'p1', 'rs1q', '00', 1000, 0, 'liquid_hns'),
                    ('coin2', 1, 'p1', 'rs1q', '00', 2000, 0, 'liquid_hns')",
            [],
        )
        .unwrap();

        let inputs = vec![("coin1".to_string(), 0u32), ("coin2".to_string(), 1u32)];
        insert_tx_draft_reserving_coins(&conn, "d1", "p1", "send_hns", "", "[]", "{}", &inputs)
            .unwrap();

        let r1: Option<String> = conn
            .query_row(
                "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = 'coin1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let r2: Option<String> = conn
            .query_row(
                "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = 'coin2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(r1.as_deref(), Some("d1"));
        assert_eq!(r2.as_deref(), Some("d1"));
    }

    /// Coverage: create_batch with asset_ids
    #[test]
    fn create_batch_with_assets() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('a')", [])
            .unwrap();
        let id1 = conn.last_insert_rowid();
        conn.execute("INSERT INTO assets (tld) VALUES ('b')", [])
            .unwrap();
        let id2 = conn.last_insert_rowid();

        let batch_id = create_batch(&conn, "test_batch", Some("desc"), &[id1, id2]).unwrap();

        let batch = get_batch_with_assets(&conn, batch_id).unwrap();
        assert_eq!(batch.name, "test_batch");
        assert_eq!(batch.description.as_deref(), Some("desc"));
        assert_eq!(batch.assets.len(), 2);
    }

    /// Coverage: read_cached_transactions with NULL raw_json
    #[test]
    fn read_cached_transactions_null_raw_json() {
        let conn = db();
        seed_profile(&conn, "p1");
        // Insert directly with NULL raw_json
        conn.execute(
            "INSERT INTO wallet_transactions_cache (wallet_profile_id, txid, height, time, raw_json)
             VALUES ('p1', 'nulltx', 100, NULL, NULL)",
            [],
        )
        .unwrap();

        let txs = read_cached_transactions(&conn, "p1").unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0]["hash"], "nulltx");
        assert_eq!(txs[0]["direction"], "other");
        assert_eq!(txs[0]["value"], 0);
    }

    /// Coverage: list_assets with empty search string (should not filter)
    #[test]
    fn list_assets_empty_search() {
        let conn = db();
        conn.execute("INSERT INTO assets (tld) VALUES ('a')", [])
            .unwrap();
        conn.execute("INSERT INTO assets (tld) VALUES ('b')", [])
            .unwrap();

        let results = list_assets(&conn, None, None, Some(""), None, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    /// Coverage: get_bid_commitment returns most recent
    #[test]
    fn get_bid_commitment_most_recent() {
        let conn = db();
        seed_profile(&conn, "p1");

        insert_bid_commitment(
            &conn, "p1", "name1", "hash1", "rs1q", 0, 0, 100, 200, "nonce1", "blind1",
        )
        .unwrap();
        // Manually set created_at to an older time so the second insert is newer
        conn.execute(
            "UPDATE bid_commitments SET created_at = '2020-01-01 00:00:00' WHERE blind_hex = 'blind1'",
            [],
        )
        .unwrap();
        // Second commitment (newer created_at)
        insert_bid_commitment(
            &conn, "p1", "name1", "hash1", "rs1q", 0, 0, 200, 400, "nonce2", "blind2",
        )
        .unwrap();

        let bid = get_bid_commitment(&conn, "p1", "name1").unwrap().unwrap();
        // Most recent (by created_at DESC) is blind2
        assert_eq!(bid.blind_hex, "blind2");
        assert_eq!(bid.bid_value_doos, 200);
    }

    /// Coverage: get_bid_commitment returns None for missing
    #[test]
    fn get_bid_commitment_none_for_missing() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(get_bid_commitment(&conn, "p1", "nonexistent")
            .unwrap()
            .is_none());
    }

    /// Coverage: has_pending_bid_draft_for_name — the `||` short-circuit means
    /// we need a case where the first `has_pending_draft_for_name` call returns
    /// false (no single-bid draft) but the second (batch-bid) returns true.
    #[test]
    fn has_pending_bid_draft_for_name_checks_batch_bid_when_no_single_bid() {
        let conn = db();
        seed_profile(&conn, "p1");

        // No single-bid draft for "name1"
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "name1").unwrap());

        // Insert a batch-bid draft for "name1"
        insert_tx_draft(
            &conn,
            "batch_d1",
            "p1",
            "batch-bid",
            "",
            "{}",
            r#"{"action":"batch-bid","nameList":["name1","name2"]}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "batch_d1", "signed", None, None).unwrap();

        // Now the check should return true (batch-bid draft exists)
        assert!(has_pending_bid_draft_for_name(&conn, "p1", "name1").unwrap());
    }

    /// Coverage: auction_position_names — the for-loop over bid commitments
    /// and the if-branch checking `get_name_coin(...).is_none()`.
    #[test]
    fn auction_position_names_includes_bid_commitments_and_filters_owned() {
        let conn = db();
        seed_profile(&conn, "p1");

        // Seed a bid commitment for "unowned_bid"
        insert_bid_commitment(
            &conn,
            "p1",
            "unowned_bid",
            "hash_unowned",
            "rs1q_unowned",
            0,
            0,
            100,
            200,
            "nonce",
            "blind_unowned",
        )
        .unwrap();

        // Seed a bid commitment for "owned_bid" + a full owner coin
        // (`get_name_coin` requires a joinable tracked_name_states row with
        // owner_txid/vout pointing at an unspent tracked_utxos row whose
        // address is registered in derived_addresses).
        insert_bid_commitment(
            &conn,
            "p1",
            "owned_bid",
            "hash_owned",
            "rs1q_owned",
            0,
            0,
            100,
            200,
            "nonce",
            "blind_owned",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,0,7,'rs1q_owned','0014','02')",
            [],
        )
        .unwrap();
        seed_name_control_utxo(&conn, "p1", "owner_txid", 0, "rs1q_owned");
        upsert_name_state(
            &conn,
            "p1",
            "owned_bid",
            &serde_json::json!({"info":{"name":"owned_bid","state":"CLOSED","height":100}}),
        )
        .unwrap();
        conn.execute(
            "UPDATE tracked_name_states SET owner_txid = 'owner_txid', owner_vout = 0
             WHERE name = 'owned_bid'",
            [],
        )
        .unwrap();

        let positions = auction_position_names(&conn, "p1").unwrap();
        // unowned_bid should be included (no owner coin)
        assert!(positions.contains(&"unowned_bid".to_string()));
        // owned_bid should be excluded (has owner coin)
        assert!(!positions.contains(&"owned_bid".to_string()));
    }

    /// Coverage: list_drafts_awaiting_confirmation — the for-loop body when
    /// at least one draft matches the status criteria.
    #[test]
    fn list_drafts_awaiting_confirmation_includes_broadcasted() {
        let conn = db();
        seed_profile(&conn, "p1");

        // Insert a broadcasted draft (should be included)
        insert_tx_draft(&conn, "d1", "p1", "bid", "", "{}", r#"{"action":"bid"}"#).unwrap();
        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("txid1")).unwrap();

        // Insert a draft-status draft (should NOT be included)
        insert_tx_draft(&conn, "d2", "p1", "bid", "", "{}", r#"{"action":"bid"}"#).unwrap();

        let awaiting = list_drafts_awaiting_confirmation(&conn, "p1", 1000, 10).unwrap();
        assert_eq!(
            awaiting.len(),
            1,
            "only broadcasted draft should be included"
        );
        assert_eq!(awaiting[0].id, "d1");
    }

    /// Coverage: list_pending_reveal_deadlines — the for-loop body when at
    /// least one bid commitment has a reveal deadline.
    #[test]
    fn list_pending_reveal_deadlines_includes_unrevealed_with_deadline() {
        let conn = db();
        seed_profile(&conn, "p1");

        // Bid commitment with reveal deadline (should be included)
        insert_bid_commitment(
            &conn,
            "p1",
            "name_with_deadline",
            "hash1",
            "rs1q1",
            0,
            0,
            100,
            200,
            "nonce",
            "blind1",
        )
        .unwrap();
        conn.execute(
            "UPDATE bid_commitments SET reveal_end_height = 5000 WHERE name = 'name_with_deadline'",
            [],
        )
        .unwrap();

        // Bid commitment without reveal deadline (should NOT be included)
        insert_bid_commitment(
            &conn,
            "p1",
            "name_no_deadline",
            "hash2",
            "rs1q2",
            0,
            0,
            100,
            200,
            "nonce",
            "blind2",
        )
        .unwrap();

        let deadlines = list_pending_reveal_deadlines(&conn).unwrap();
        assert_eq!(deadlines.len(), 1);
        assert_eq!(deadlines[0].0, "p1");
        assert_eq!(deadlines[0].1, "name_with_deadline");
        assert_eq!(deadlines[0].2, 5000);
    }

    /// Coverage: row_to_profile — the `watch_only` and `has_passphrase` flag
    /// decoding with the `!= 0` and `unwrap_or` patterns.
    #[test]
    fn row_to_profile_decodes_watch_only_and_passphrase_flags() {
        let conn = db();
        // Insert a watch-only profile with a passphrase. The `kind` column
        // has a CHECK constraint restricting values to a known set — use the
        // real `watch_only_xpub` kind, which is what the app writes.
        insert_wallet_profile(
            &conn,
            "watch_p1",
            "Watch Only",
            "watch_only_xpub",
            "mainnet",
            "xpubWATCH",
            0,
            true,
        )
        .unwrap();
        insert_wallet_secret(&conn, "watch_p1", &[0xaa, 0xbb], "argon2id", "fp456").unwrap();

        // Insert a non-watch profile without passphrase
        insert_wallet_profile(
            &conn,
            "hot_p1",
            "Hot Wallet",
            "mnemonic_hot",
            "mainnet",
            "xpubHOT",
            0,
            false,
        )
        .unwrap();

        let profiles = list_wallet_profiles(&conn).unwrap();
        let watch_p = profiles.iter().find(|p| p.id == "watch_p1").unwrap();
        let hot_p = profiles.iter().find(|p| p.id == "hot_p1").unwrap();

        assert!(watch_p.watch_only, "watch_p1 should be watch_only");
        assert!(watch_p.has_passphrase, "watch_p1 should have passphrase");
        assert!(!hot_p.watch_only, "hot_p1 should NOT be watch_only");
        assert!(!hot_p.has_passphrase, "hot_p1 should NOT have passphrase");
    }
}
