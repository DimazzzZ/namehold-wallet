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
            param_idx += 1;
        }
    }

    let valid_sort_cols = ["tld", "status", "is_staked", "category", "hns_received", "expires_at_height", "updated_at", "created_at"];
    let col = sort_by.filter(|c| valid_sort_cols.contains(&c)).unwrap_or("tld");
    let dir = if sort_dir == Some("desc") { "DESC" } else { "ASC" };
    sql.push_str(&format!(" ORDER BY {} {}", col, dir));

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok(Asset::from_row(row))
    })?;

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
    let sql = format!("UPDATE assets SET {} WHERE id = ?{}", sets.join(", "), param_idx);
    param_values.push(Box::new(id));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
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
    let rows = stmt.query_map([], |row| {
        Ok(Batch::from_row(row))
    })?;
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
        .query_map(params![batch_id], |row| Asset::from_row(row))?
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
    let sql = format!("UPDATE batches SET {} WHERE id = ?{}", sets.join(", "), param_idx);
    param_values.push(Box::new(id));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
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
    let staked: i64 = conn.query_row(
        "SELECT COUNT(*) FROM assets WHERE is_staked = 1",
        [],
        |r| r.get(0),
    )?;
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
        let result = conn.query_row(
            "SELECT * FROM assets WHERE tld = ?1",
            params![tld],
            |row| Asset::from_row(row),
        );
        match result {
            Ok(asset) => assets.push(asset),
            Err(_) => continue,
        }
    }
    Ok(assets)
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
        AS has_passphrase";

fn row_to_profile(
    row: &rusqlite::Row,
    active_id: &str,
) -> rusqlite::Result<WalletProfileSummary> {
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
    pub created_at: String,
}

const DRAFT_COLS: &str = "id, wallet_profile_id, action, unsigned_tx_hex, signed_tx_hex, \
     signing_inputs_json, summary_json, status, error_message, txid, created_at";

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
        created_at: row.get(10)?,
    })
}

impl TxDraftRow {
    /// Project to the frontend-facing summary (parsing `summary_json`).
    pub fn to_summary(&self) -> TxDraftSummary {
        let summary = serde_json::from_str(&self.summary_json)
            .unwrap_or(serde_json::Value::Null);
        TxDraftSummary {
            id: self.id.clone(),
            wallet_profile_id: self.wallet_profile_id.clone(),
            action: self.action.clone(),
            status: self.status.clone(),
            summary,
            error_message: self.error_message.clone(),
            txid: self.txid.clone(),
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

/// Fetch one draft, or `None`.
pub fn get_tx_draft(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<TxDraftRow>, AppError> {
    let sql = format!("SELECT {DRAFT_COLS} FROM wallet_tx_drafts WHERE id = ?1");
    let row = conn
        .query_row(&sql, params![id], row_to_draft)
        .optional()?;
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
    let mut stmt = conn.prepare(
        "SELECT n.name, n.state, n.height, n.renewal_height, n.owner_txid, n.owner_vout
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
        let owner = owner_txid.map(|hash| {
            serde_json::json!({ "hash": hash, "index": owner_vout.unwrap_or(0) })
        });
        Ok(serde_json::json!({
            "name": name,
            "state": state,
            "height": height,
            "renewal": renewal,
            "owner": owner,
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
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
             height, renewal_height, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(wallet_profile_id, name) DO UPDATE SET
            name_hash_hex  = excluded.name_hash_hex,
            state          = excluded.state,
            owner_txid     = excluded.owner_txid,
            owner_vout     = excluded.owner_vout,
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
pub fn read_owned_names_explorer(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT name, state, height, renewal_height, owner_txid, owner_vout
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
        let owner = owner_txid
            .map(|hash| serde_json::json!({ "hash": hash, "index": owner_vout.unwrap_or(0) }));
        Ok(serde_json::json!({
            "name": name,
            "state": state,
            "height": height,
            "renewal": renewal,
            "owner": owner,
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
    let mut stmt = conn
        .prepare("SELECT name FROM tracked_name_states WHERE wallet_profile_id = ?1")?;
    let rows = stmt.query_map(params![profile_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
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
        let mut stmt = conn
            .prepare("SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1")?;
        let rows = stmt.query_map(params![profile_id], |r| r.get::<_, String>(0))?;
        let mut s = HashSet::new();
        for r in rows {
            s.insert(r?);
        }
        s
    };
    let our_outpoints: HashSet<(String, i64)> = {
        let mut stmt = conn
            .prepare("SELECT txid, vout FROM tracked_utxos WHERE wallet_profile_id = ?1")?;
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
        let parsed: Option<serde_json::Value> =
            raw_json.as_deref().and_then(|s| serde_json::from_str(s).ok());

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

/// Find an unspent tracked UTXO at `address` with a given covenant type, with
/// its derivation path. Used to locate our BID coin (to reveal) or a losing
/// REVEAL coin (to redeem).
pub fn find_unspent_covenant_utxo(
    conn: &rusqlite::Connection,
    profile_id: &str,
    address: &str,
    covenant_type: i64,
) -> Result<Option<NameCoin>, AppError> {
    let row = conn
        .query_row(
            "SELECT u.txid, u.vout, u.value_doos, u.address, d.branch, d.child_index,
                    u.covenant_type, u.covenant_json, NULL
             FROM tracked_utxos u
             JOIN derived_addresses d
               ON d.wallet_profile_id = u.wallet_profile_id AND d.address = u.address
             WHERE u.wallet_profile_id = ?1 AND u.address = ?2
               AND u.covenant_type = ?3 AND u.spent_by_txid IS NULL
             LIMIT 1",
            params![profile_id, address, covenant_type],
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
}

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
    conn.execute(
        "INSERT INTO bid_commitments
            (wallet_profile_id, name, name_hash_hex, address, branch, child_index,
             bid_value_doos, lockup_value_doos, nonce_hex, blind_hex)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(wallet_profile_id, name, blind_hex) DO NOTHING",
        params![
            profile_id, name, name_hash_hex, address, branch, child_index,
            bid_value, lockup, nonce_hex, blind_hex
        ],
    )?;
    Ok(())
}

const BID_COLS: &str = "name, name_hash_hex, address, branch, child_index, \
     bid_value_doos, lockup_value_doos, nonce_hex, blind_hex, bid_txid, reveal_txid";

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
    })
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
    Ok(conn.query_row(&sql, params![profile_id, name], row_to_bid).optional()?)
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
        insert_wallet_profile(conn, id, "Primary", "mnemonic_hot", "regtest", "xpubFAKE", 0, false)
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
            &conn, "p2", "Watch", "watch_only_xpub", "regtest", "xpubW", 0, true,
        )
        .unwrap();
        assert_eq!(get_wallet_secret_meta(&conn, "p2").unwrap(), None);
        // No-passphrase wallets are marked kdf='none'.
        insert_wallet_secret(&conn, "p2", &[1, 2, 3], "none", "fp2").unwrap();
        assert_eq!(get_wallet_secret_meta(&conn, "p2").unwrap().unwrap().1, "none");
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

        update_tx_draft_signed(&conn, "d1", "0011aabb", r#"{"action":"send_hns","txid":"tx1"}"#)
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
        assert_eq!(addrs, vec!["rs1qrecv".to_string(), "rs1qchange".to_string()]);
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
}
