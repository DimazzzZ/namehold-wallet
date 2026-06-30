use crate::error::AppError;
use crate::models::asset::Asset;
use crate::models::batch::{Batch, BatchWithAssets};
use crate::models::settings::SettingsMap;
use rusqlite::params;

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
