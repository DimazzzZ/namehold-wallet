//! Tests for the provider-aware read layer (`commands::read`) and the provider
//! resolution it depends on (`providers::resolver`).
//!
//! These focus on the deterministic pieces that do not require live network
//! access: connection-mode/provider-setting parsing, provider-mode default
//! settings, and the error path when `external_read_only` mode is selected
//! without an external provider configured.

use crate::db;
use crate::providers::types::{ConnectionMode, ExternalReadProvider, ReadProviderKind};
use crate::providers::resolve_read_context;
use crate::tests::command_helpers::{create_test_db, create_test_state};

#[test]
fn test_connection_mode_parsing_round_trip() {
    assert_eq!(
        ConnectionMode::from_setting("local_managed_hsd"),
        ConnectionMode::LocalManagedHsd
    );
    assert_eq!(
        ConnectionMode::from_setting("remote_hsd"),
        ConnectionMode::RemoteHsd
    );
    assert_eq!(
        ConnectionMode::from_setting("auto_fallback"),
        ConnectionMode::AutoFallback
    );
    assert_eq!(
        ConnectionMode::from_setting("external_read_only"),
        ConnectionMode::ExternalReadOnly
    );
    // Unknown values default to the safe local managed mode.
    assert_eq!(
        ConnectionMode::from_setting("nonsense"),
        ConnectionMode::LocalManagedHsd
    );

    // as_setting is the inverse of from_setting for known variants.
    for mode in [
        ConnectionMode::LocalManagedHsd,
        ConnectionMode::RemoteHsd,
        ConnectionMode::AutoFallback,
        ConnectionMode::ExternalReadOnly,
    ] {
        assert_eq!(ConnectionMode::from_setting(mode.as_setting()), mode);
    }
}

#[test]
fn test_external_read_provider_parsing() {
    assert_eq!(
        ExternalReadProvider::from_setting("hnsfans"),
        ExternalReadProvider::Hnsfans
    );
    assert_eq!(
        ExternalReadProvider::from_setting("none"),
        ExternalReadProvider::None
    );
    assert_eq!(
        ExternalReadProvider::from_setting("anything_else"),
        ExternalReadProvider::None
    );
}

#[test]
fn test_provider_mode_default_settings_present() {
    let conn = create_test_db();
    let settings = db::queries::get_settings(&conn).unwrap();

    // Defaults installed by the 003_provider_modes migration.
    assert_eq!(settings["connection_mode"], "local_managed_hsd");
    assert_eq!(settings["external_read_provider"], "none");
    assert_eq!(settings["external_read_api_url"], "https://e.hnsfans.com");
    assert_eq!(settings["external_read_watch_addresses"], "[]");
    assert_eq!(settings["external_read_watch_names"], "[]");
    assert_eq!(settings["remote_hsd_label"], "");
    assert_eq!(settings["trusted_remote_hsd"], "false");
    assert_eq!(settings["future_signer_mode"], "none");
}

#[tokio::test]
async fn test_external_read_only_without_provider_errors() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::set_setting(&db, "connection_mode", "external_read_only").unwrap();
        // Leave external_read_provider at its default of "none".
    }

    let result = resolve_read_context(&state).await;
    assert!(
        result.is_err(),
        "external_read_only with no provider should error"
    );
}

#[tokio::test]
async fn test_local_managed_mode_resolves_to_local_hsd() {
    let state = create_test_state();
    // Default connection_mode is local_managed_hsd; point hsd at an unreachable
    // address so the health probe deterministically fails without hanging long.
    {
        let db = state.db.lock().unwrap();
        db::queries::set_setting(&db, "hsd_wallet_api_url", "http://127.0.0.1:1").unwrap();
        db::queries::set_setting(&db, "hsd_node_api_url", "http://127.0.0.1:1").unwrap();
    }

    let resolution = resolve_read_context(&state)
        .await
        .expect("local managed mode always resolves to an hsd context");

    assert_eq!(resolution.mode, ConnectionMode::LocalManagedHsd);
    assert!(resolution.hsd.is_some());
    assert!(resolution.external.is_none());
    assert_eq!(
        resolution.context.active_read_provider.kind,
        ReadProviderKind::LocalHsd
    );
    // hsd is unreachable, so it is reported unhealthy and reads will fail, but
    // the context still describes the local provider as the active one.
    assert!(!resolution.context.active_read_provider.healthy);
    // Local managed mode is always write-capable in principle.
    assert!(resolution.context.write_allowed);
}

#[tokio::test]
async fn test_remote_untrusted_mode_disallows_writes() {
    let state = create_test_state();
    {
        let db = state.db.lock().unwrap();
        db::queries::set_setting(&db, "connection_mode", "remote_hsd").unwrap();
        db::queries::set_setting(&db, "remote_hsd_label", "My Remote").unwrap();
        db::queries::set_setting(&db, "trusted_remote_hsd", "false").unwrap();
        db::queries::set_setting(&db, "hsd_wallet_api_url", "http://127.0.0.1:1").unwrap();
        db::queries::set_setting(&db, "hsd_node_api_url", "http://127.0.0.1:1").unwrap();
    }

    let resolution = resolve_read_context(&state)
        .await
        .expect("remote mode resolves to an hsd context");

    assert_eq!(resolution.mode, ConnectionMode::RemoteHsd);
    assert_eq!(
        resolution.context.active_read_provider.kind,
        ReadProviderKind::RemoteHsd
    );
    assert_eq!(resolution.context.active_read_provider.label, "My Remote");
    // Untrusted remote never permits writes.
    assert!(!resolution.context.write_allowed);
    assert!(resolution.context.write_reason.is_some());
}
