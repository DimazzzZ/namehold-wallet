use crate::db;
use crate::error::AppError;
use crate::models::asset::ImportResult;
use crate::AppState;
use serde::Deserialize;
use tauri::State;

#[derive(Debug, Deserialize)]
pub(crate) struct CsvRow {
    #[serde(alias = "Name", alias = "name", alias = "TLD", alias = "tld")]
    pub(crate) tld: Option<String>,
    #[serde(alias = "Staked", alias = "staked", alias = "is_staked", alias = "IsStaked", default)]
    pub(crate) is_staked: Option<String>,
    #[serde(alias = "Category", alias = "category", default)]
    pub(crate) category: Option<String>,
    #[serde(alias = "Tag", alias = "tag", alias = "Tags", alias = "tags", default)]
    pub(crate) tags: Option<String>,
    #[serde(alias = "Notes", alias = "notes", alias = "Note", alias = "note", default)]
    pub(crate) notes: Option<String>,
    #[serde(alias = "has_sld", alias = "HasSld", alias = "has_sld", default)]
    pub(crate) has_sld: Option<String>,
    #[serde(alias = "Status", alias = "status", alias = "MigrationStatus", alias = "migration_status", default)]
    pub(crate) status: Option<String>,
}

pub(crate) fn parse_boolish(v: &str) -> bool {
    matches!(
        v.to_lowercase().trim(),
        "true" | "1" | "yes" | "y" | "staked"
    )
}

pub(crate) fn normalize_tld(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('.')
        .trim()
        .to_lowercase()
}

pub(crate) fn infer_status(is_staked: bool, status_hint: Option<&str>) -> &'static str {
    if is_staked {
        return "do_not_touch_staked";
    }
    if let Some(hint) = status_hint {
        let h = hint.to_lowercase().replace(' ', "_").replace('-', "_");
        match h.as_str() {
            "namebase_transfer_requested" => "namebase_transfer_requested",
            "waiting_transfer_tx" => "waiting_transfer_tx",
            "transfer_seen_on_chain" => "transfer_seen_on_chain",
            "waiting_finalize" => "waiting_finalize",
            "finalized_owned" => "finalized_owned",
            "failed_or_stuck" => "failed_or_stuck",
            "do_not_touch_staked" => "do_not_touch_staked",
            _ => "not_started",
        }
    } else {
        "not_started"
    }
}

#[tauri::command]
pub async fn import_csv(
    state: State<'_, AppState>,
    path: String,
) -> Result<ImportResult, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_path(&path)?;

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (i, result) in rdr.deserialize().enumerate() {
        let row: CsvRow = match result {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("Row {}: {}", i + 2, e));
                continue;
            }
        };

        let raw_tld = match &row.tld {
            Some(t) if !t.trim().is_empty() => t.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };

        let tld = normalize_tld(&raw_tld);
        if tld.is_empty() {
            skipped += 1;
            continue;
        }

        let is_staked = row
            .is_staked
            .as_deref()
            .map(parse_boolish)
            .unwrap_or(false);
        let status = infer_status(is_staked, row.status.as_deref());

        let tags_json = row.tags.as_deref().map(|t| {
            let items: Vec<&str> = t.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
        });

        let result = db.execute(
            "INSERT INTO assets (tld, is_staked, status, category, tags, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(tld) DO UPDATE SET
               is_staked = excluded.is_staked,
               status = CASE
                 WHEN assets.status = 'do_not_touch_staked' AND excluded.is_staked = 0 THEN 'not_started'
                 ELSE excluded.status
               END,
               category = COALESCE(excluded.category, assets.category),
               tags = COALESCE(excluded.tags, assets.tags),
               notes = CASE
                 WHEN excluded.notes IS NOT NULL AND excluded.notes != '' THEN excluded.notes
                 ELSE assets.notes
               END,
               updated_at = datetime('now')",
            rusqlite::params![
                tld,
                if is_staked { 1 } else { 0 },
                status,
                row.category,
                tags_json,
                row.notes
            ],
        );

        match result {
            Ok(_) => imported += 1,
            Err(e) => errors.push(format!("Row {}: {}", i + 2, e)),
        }
    }

    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('import_csv', ?1)",
        [serde_json::json!({
            "imported": imported,
            "skipped": skipped,
            "errors": errors.len()
        })
        .to_string()],
    )?;

    Ok(ImportResult {
        imported,
        skipped,
        errors,
    })
}

#[tauri::command]
pub async fn export_csv(
    state: State<'_, AppState>,
    path: String,
    status: Option<String>,
    is_staked: Option<bool>,
    search: Option<String>,
) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let assets = db::queries::list_assets(
        &db,
        status.as_deref(),
        is_staked,
        search.as_deref(),
        None,
        None,
    )?;

    let mut wtr = csv::WriterBuilder::new().from_path(&path)?;

    wtr.write_record([
        "Name",
        "Status",
        "Staked",
        "Category",
        "Tags",
        "Notes",
        "HNS Received",
        "Transfer TX",
        "Finalize TX",
        "Name State",
        "Expires At Height",
        "Last Synced",
        "Created",
        "Updated",
    ])?;

    for asset in &assets {
        let record: Vec<String> = vec![
            asset.tld.clone(),
            asset.status.as_str().to_string(),
            if asset.is_staked { "true" } else { "false" }.to_string(),
            asset.category.clone().unwrap_or_default(),
            serde_json::to_string(&asset.tags).unwrap_or_else(|_| "[]".to_string()),
            asset.notes.clone().unwrap_or_default(),
            asset.hns_received.map(|v| v.to_string()).unwrap_or_default(),
            asset.transfer_tx_hash.clone().unwrap_or_default(),
            asset.finalize_tx_hash.clone().unwrap_or_default(),
            asset.name_state.clone().unwrap_or_default(),
            asset.expires_at_height.map(|v| v.to_string()).unwrap_or_default(),
            asset.last_synced_at.clone().unwrap_or_default(),
            asset.created_at.clone(),
            asset.updated_at.clone(),
        ];
        wtr.write_record(&record)?;
    }

    wtr.flush()?;

    db.execute(
        "INSERT INTO audit_log (action, detail) VALUES ('export_csv', ?1)",
        [serde_json::json!({"path": path, "count": assets.len()}).to_string()],
    )?;

    Ok(assets.len())
}
