//! Provider resolution: given the current settings and live health checks,
//! decide which backend serves reads, whether writes are allowed, and whether
//! a fallback to an external read-only provider is active.

use crate::db::queries;
use crate::error::AppError;
use crate::hsd::client::HandshakeClient;
use crate::providers::hnsfans::HnsFansClient;
use crate::providers::types::*;
use crate::AppState;

/// Bundle returned to commands: the resolved context plus ready-to-use clients
/// for whichever backends are relevant.
pub struct ProviderResolution {
    pub context: ReadContext,
    pub mode: ConnectionMode,
    /// Present when the active read provider is a (local or remote) hsd.
    pub hsd: Option<HandshakeClient>,
    /// Present when an external provider is the active read source.
    pub external: Option<HnsFansClient>,
    pub watch_addresses: Vec<String>,
    pub watch_names: Vec<String>,
}

fn parse_string_array(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Resolve the active read context. Performs live health checks against hsd
/// and/or the external provider as required by the connection mode.
pub async fn resolve_read_context(state: &AppState) -> Result<ProviderResolution, AppError> {
    let (
        settings_mode,
        external_provider,
        external_url,
        watch_addresses,
        watch_names,
        watch_auto_derived,
        trusted_remote,
        remote_label,
        hsd_client,
    ) = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let settings = queries::get_settings(&db)?;
        let mode = ConnectionMode::from_setting(
            settings
                .get("connection_mode")
                .map(|s| s.as_str())
                .unwrap_or("local_managed_hsd"),
        );
        let external_provider = ExternalReadProvider::from_setting(
            settings
                .get("external_read_provider")
                .map(|s| s.as_str())
                .unwrap_or("none"),
        );
        let external_url = settings
            .get("external_read_api_url")
            .cloned()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://e.hnsfans.com".to_string());
        let mut watch_addresses = parse_string_array(
            settings
                .get("external_read_watch_addresses")
                .map(|s| s.as_str())
                .unwrap_or("[]"),
        );
        let mut watch_names = parse_string_array(
            settings
                .get("external_read_watch_names")
                .map(|s| s.as_str())
                .unwrap_or("[]"),
        );

        // Option B (wallet-aware external mode): when the user hasn't supplied
        // explicit watch targets, auto-derive them from locally cached data so
        // the selected wallet "just works" in external read-only mode.
        //  - addresses come from the per-wallet address cache (preferred),
        //    populated during sync against a local/remote hsd. This is scoped to
        //    the *selected wallet* so the right data shows up.
        //  - if that cache is empty (e.g. wallet never synced locally), fall
        //    back to addresses recorded in past wallet snapshots.
        //  - names come from the local asset inventory (TLDs).
        let selected_wallet_id = settings
            .get("hsd_wallet_id")
            .cloned()
            .unwrap_or_else(|| "primary".to_string());
        let mut watch_auto_derived = false;
        if watch_addresses.is_empty() {
            if let Ok(addrs) =
                queries::get_wallet_addresses_for_wallet(&db, &selected_wallet_id, 200)
            {
                if !addrs.is_empty() {
                    watch_addresses = addrs;
                    watch_auto_derived = true;
                }
            }
        }
        if watch_addresses.is_empty() {
            if let Ok(addrs) = queries::get_known_wallet_addresses(&db, 50) {
                if !addrs.is_empty() {
                    watch_addresses = addrs;
                    watch_auto_derived = true;
                }
            }
        }
        if watch_names.is_empty() {
            if let Ok(tlds) = queries::get_inventory_tlds(&db) {
                if !tlds.is_empty() {
                    watch_names = tlds;
                    watch_auto_derived = true;
                }
            }
        }

        let trusted_remote = settings
            .get("trusted_remote_hsd")
            .map(|s| s == "true")
            .unwrap_or(false);
        let remote_label = settings.get("remote_hsd_label").cloned().unwrap_or_default();
        let hsd_client = HandshakeClient::from_settings(&settings);
        (
            mode,
            external_provider,
            external_url,
            watch_addresses,
            watch_names,
            watch_auto_derived,
            trusted_remote,
            remote_label,
            hsd_client,
        )
    };

    // Probe hsd health for modes that may use it.
    let needs_hsd = matches!(
        settings_mode,
        ConnectionMode::LocalManagedHsd
            | ConnectionMode::RemoteHsd
            | ConnectionMode::AutoFallback
    );
    let hsd_healthy = if needs_hsd {
        hsd_client.check_connection().await.is_ok()
    } else {
        false
    };

    let external_client = if external_provider == ExternalReadProvider::Hnsfans {
        Some(HnsFansClient::new(&external_url))
    } else {
        None
    };

    match settings_mode {
        ConnectionMode::LocalManagedHsd => Ok(resolve_hsd(
            settings_mode,
            ReadProviderKind::LocalHsd,
            "Local hsd".to_string(),
            true,
            true,
            None,
            hsd_healthy,
            hsd_client,
            watch_addresses,
            watch_names,
        )),
        ConnectionMode::RemoteHsd => {
            let label = if remote_label.is_empty() {
                "Remote hsd".to_string()
            } else {
                remote_label
            };
            let write_allowed = trusted_remote && hsd_healthy;
            let write_reason = if !trusted_remote {
                Some("Remote hsd is not trusted. Acknowledge trust in Settings to enable writes.".to_string())
            } else if !hsd_healthy {
                Some("Remote hsd is unreachable.".to_string())
            } else {
                None
            };
            Ok(resolve_hsd(
                settings_mode,
                ReadProviderKind::RemoteHsd,
                label,
                write_allowed,
                false,
                write_reason,
                hsd_healthy,
                hsd_client,
                watch_addresses,
                watch_names,
            ))
        }
        ConnectionMode::AutoFallback => {
            if hsd_healthy {
                Ok(resolve_hsd(
                    settings_mode,
                    ReadProviderKind::LocalHsd,
                    "Local hsd".to_string(),
                    true,
                    true,
                    None,
                    true,
                    hsd_client,
                    watch_addresses,
                    watch_names,
                ))
            } else {
                // Fall back to external read-only.
                resolve_external(
                    settings_mode,
                    external_provider,
                    external_client,
                    &external_url,
                    watch_addresses,
                    watch_names,
                    watch_auto_derived,
                    true,
                    false,
                )
                .await
            }
        }
        ConnectionMode::ExternalReadOnly => {
            resolve_external(
                settings_mode,
                external_provider,
                external_client,
                &external_url,
                watch_addresses,
                watch_names,
                watch_auto_derived,
                false,
                false,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_hsd(
    mode: ConnectionMode,
    kind: ReadProviderKind,
    label: String,
    write_allowed: bool,
    manageable: bool,
    write_reason: Option<String>,
    healthy: bool,
    hsd: HandshakeClient,
    watch_addresses: Vec<String>,
    watch_names: Vec<String>,
) -> ProviderResolution {
    let status = ProviderStatus {
        kind,
        label,
        healthy,
        write_capable: write_allowed,
        manageable,
        reason: if healthy {
            None
        } else {
            Some("hsd is unreachable.".to_string())
        },
        provider_url: None,
        network: None,
        chain_height: None,
        verification_progress: None,
        syncing: None,
    };
    let context = ReadContext {
        connection_mode: mode.as_setting().to_string(),
        active_read_provider: status,
        fallback_active: false,
        local_node_healthy: healthy && matches!(kind, ReadProviderKind::LocalHsd),
        wallet_available: healthy,
        write_allowed,
        write_reason,
    };
    ProviderResolution {
        context,
        mode,
        hsd: Some(hsd),
        external: None,
        watch_addresses,
        watch_names,
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_external(
    mode: ConnectionMode,
    provider: ExternalReadProvider,
    external_client: Option<HnsFansClient>,
    external_url: &str,
    watch_addresses: Vec<String>,
    watch_names: Vec<String>,
    watch_auto_derived: bool,
    fallback_active: bool,
    local_node_healthy: bool,
) -> Result<ProviderResolution, AppError> {
    if provider != ExternalReadProvider::Hnsfans || external_client.is_none() {
        return Err(AppError::Other(
            "No external read provider is configured. Set one in Settings.".to_string(),
        ));
    }
    let client = external_client.unwrap();
    let healthy = client.health().await.is_ok();

    // Whether we have *anything* to read for this wallet. In external mode the
    // provider only knows public addresses/names, so without any watch targets
    // (explicit or auto-derived) there is nothing to show.
    let has_watch_targets = !watch_addresses.is_empty() || !watch_names.is_empty();

    // Compute a precise reason so the UI doesn't show a vague "unavailable".
    let reason = if !healthy {
        Some(format!("{} is unreachable.", external_url))
    } else if !has_watch_targets {
        Some(
            "No public addresses or tracked names are known yet for this wallet. \
             Sync once with a local node, or add watch addresses/names in Settings → Advanced."
                .to_string(),
        )
    } else if watch_auto_derived {
        Some("Using cached wallet data (addresses from snapshots, names from inventory).".to_string())
    } else if fallback_active {
        Some("Local hsd unavailable; using external read-only provider.".to_string())
    } else {
        None
    };

    // The provider is only usable when it is reachable AND we have something to
    // read for the selected wallet.
    let usable = healthy && has_watch_targets;

    let status = ProviderStatus {
        kind: ReadProviderKind::ExternalHnsfans,
        label: "HNSFans (read-only)".to_string(),
        healthy: usable,
        write_capable: false,
        manageable: false,
        reason,
        provider_url: Some(external_url.to_string()),
        network: None,
        chain_height: None,
        verification_progress: None,
        syncing: None,
    };

    let context = ReadContext {
        connection_mode: mode.as_setting().to_string(),
        active_read_provider: status,
        fallback_active,
        local_node_healthy,
        wallet_available: usable,
        write_allowed: false,
        write_reason: Some(
            "External read-only provider does not support writes.".to_string(),
        ),
    };

    Ok(ProviderResolution {
        context,
        mode,
        hsd: None,
        external: Some(client),
        watch_addresses,
        watch_names,
    })
}
