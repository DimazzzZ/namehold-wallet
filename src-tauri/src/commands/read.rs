//! Provider-aware read commands.
//!
//! These commands resolve the active read provider (local hsd, remote hsd, or
//! an external read-only explorer) and serve normalized read models regardless
//! of the backend. Writes are never routed through here.

use crate::db::queries;
use crate::error::AppError;
use crate::providers::{resolve_read_context, ReadContext};
use crate::AppState;
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

/// Resolve and return the current read context (active provider, health,
/// write permissions, fallback state). Cheap enough to poll from the UI.
#[tauri::command]
pub async fn get_read_context(state: State<'_, AppState>) -> Result<ReadContext, AppError> {
    let resolution = resolve_read_context(&state).await?;
    Ok(resolution.context)
}

/// Provider-aware balance. Uses hsd when available, otherwise aggregates the
/// configured watch addresses via the external provider.
#[tauri::command]
pub async fn read_balance(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let resolution = resolve_read_context(&state).await?;
    if let Some(hsd) = resolution.hsd.as_ref() {
        let balance = hsd.get_balance().await?;
        return Ok(serde_json::to_value(&balance)?);
    }
    if let Some(external) = resolution.external.as_ref() {
        let balance = external.get_balance(&resolution.watch_addresses).await?;
        return Ok(serde_json::to_value(&balance)?);
    }
    Err(AppError::Other(
        "No read provider is available.".to_string(),
    ))
}

/// Provider-aware name list.
#[tauri::command]
pub async fn read_names(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let resolution = resolve_read_context(&state).await?;
    if let Some(hsd) = resolution.hsd.as_ref() {
        let names = hsd.get_names().await?;
        return Ok(serde_json::to_value(&names)?);
    }
    if let Some(external) = resolution.external.as_ref() {
        let names = external
            .get_names(&resolution.watch_addresses, &resolution.watch_names)
            .await?;
        return Ok(serde_json::to_value(&names)?);
    }
    Err(AppError::Other(
        "No read provider is available.".to_string(),
    ))
}

/// Provider-aware single name lookup.
#[tauri::command]
pub async fn read_name_info(
    state: State<'_, AppState>,
    name: String,
) -> Result<serde_json::Value, AppError> {
    let resolution = resolve_read_context(&state).await?;
    if let Some(hsd) = resolution.hsd.as_ref() {
        let info = hsd.get_name_info(&name).await?;
        return Ok(serde_json::to_value(&info)?);
    }
    if let Some(external) = resolution.external.as_ref() {
        let info = external.get_name_info(&name).await?;
        return Ok(serde_json::to_value(&info)?);
    }
    Err(AppError::Other(
        "No read provider is available.".to_string(),
    ))
}

/// Provider-aware transaction history.
#[tauri::command]
pub async fn read_transactions(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let resolution = resolve_read_context(&state).await?;
    if let Some(hsd) = resolution.hsd.as_ref() {
        return Ok(hsd.get_transactions().await?);
    }
    if let Some(external) = resolution.external.as_ref() {
        return Ok(external.get_transactions(&resolution.watch_addresses).await?);
    }
    Err(AppError::Other(
        "No read provider is available.".to_string(),
    ))
}

/// Aggregate read model combining the resolved context with balance, names and
/// transactions in a single round-trip. Mirrors the frontend `WalletReadModel`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletReadModel {
    pub context: ReadContext,
    pub watch_addresses: Vec<String>,
    pub balance: serde_json::Value,
    pub names: serde_json::Value,
    pub transactions: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_reason: Option<String>,
}

#[tauri::command]
pub async fn get_wallet_read_model(
    state: State<'_, AppState>,
) -> Result<WalletReadModel, AppError> {
    let resolution = resolve_read_context(&state).await?;
    let context = resolution.context.clone();

    let (balance, names, transactions) = if let Some(hsd) = resolution.hsd.as_ref() {
        (
            serde_json::to_value(hsd.get_balance().await.ok()).unwrap_or(serde_json::Value::Null),
            serde_json::to_value(hsd.get_names().await.unwrap_or_default())?,
            hsd.get_transactions()
                .await
                .unwrap_or(serde_json::Value::Array(vec![])),
        )
    } else if let Some(external) = resolution.external.as_ref() {
        (
            serde_json::to_value(external.get_balance(&resolution.watch_addresses).await.ok())
                .unwrap_or(serde_json::Value::Null),
            serde_json::to_value(
                external
                    .get_names(&resolution.watch_addresses, &resolution.watch_names)
                    .await
                    .unwrap_or_default(),
            )?,
            external
                .get_transactions(&resolution.watch_addresses)
                .await
                .unwrap_or(serde_json::Value::Array(vec![])),
        )
    } else {
        return Err(AppError::Other(
            "No read provider is available.".to_string(),
        ));
    };

    let read_only_reason = if context.write_allowed {
        None
    } else {
        context
            .write_reason
            .clone()
            .or_else(|| context.active_read_provider.reason.clone())
            .or_else(|| Some("Writes are not available in the current mode.".to_string()))
    };

    Ok(WalletReadModel {
        context,
        watch_addresses: resolution.watch_addresses,
        balance,
        names,
        transactions,
        read_only_reason,
    })
}

/// Result of comparing the locally tracked inventory (assets table) against the
/// names reported by the active read provider.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryComparison {
    pub provider_kind: String,
    pub provider_label: String,
    /// Names present both locally and at the provider.
    pub matched: Vec<String>,
    /// Names tracked locally but not reported by the provider.
    pub missing_at_provider: Vec<String>,
    /// Names reported by the provider but not tracked locally.
    pub extra_at_provider: Vec<String>,
}

#[tauri::command]
pub async fn compare_inventory_with_provider(
    state: State<'_, AppState>,
) -> Result<InventoryComparison, AppError> {
    let resolution = resolve_read_context(&state).await?;

    // Names the provider reports.
    let provider_names: Vec<String> = if let Some(hsd) = resolution.hsd.as_ref() {
        hsd.get_names()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.name)
            .collect()
    } else if let Some(external) = resolution.external.as_ref() {
        external
            .get_names(&resolution.watch_addresses, &resolution.watch_names)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.name)
            .collect()
    } else {
        return Err(AppError::Other(
            "No read provider is available.".to_string(),
        ));
    };

    // Locally tracked inventory (asset TLDs).
    let local_names: Vec<String> = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::list_assets(&db, None, None, None, None, None)?
            .into_iter()
            .map(|a| a.tld)
            .collect()
    };

    let provider_set: HashSet<String> = provider_names.iter().cloned().collect();
    let local_set: HashSet<String> = local_names.iter().cloned().collect();

    let mut matched: Vec<String> = local_names
        .iter()
        .filter(|n| provider_set.contains(*n))
        .cloned()
        .collect();
    let mut missing_at_provider: Vec<String> = local_names
        .iter()
        .filter(|n| !provider_set.contains(*n))
        .cloned()
        .collect();
    let mut extra_at_provider: Vec<String> = provider_names
        .iter()
        .filter(|n| !local_set.contains(*n))
        .cloned()
        .collect();
    matched.sort();
    missing_at_provider.sort();
    extra_at_provider.sort();

    let provider = &resolution.context.active_read_provider;
    let provider_kind = serde_json::to_value(provider.kind)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    Ok(InventoryComparison {
        provider_kind,
        provider_label: provider.label.clone(),
        matched,
        missing_at_provider,
        extra_at_provider,
    })
}
