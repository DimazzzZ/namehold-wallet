use crate::commands::secure_wallet;
use crate::AppState;
use tauri::Manager;

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

/// Create a test DB with ALL migrations (including wallet_profiles from 006+).
fn create_full_test_db() -> rusqlite::Connection {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/004_wallet_addresses.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/005_fix_hnsfans_api_url.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/006_noncustodial_wallet_profiles.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/007_noncustodial_chain_cache.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/008_noncustodial_name_state.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/009_node_rpc_settings.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/010_drop_legacy_settings.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/011_hsd_data_dir.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../../src-tauri/src/sql/012_tx_draft_confirmations.sql"))
        .unwrap();
    conn
}

fn create_full_test_state() -> AppState {
    let conn = create_full_test_db();
    AppState {
        db: std::sync::Mutex::new(conn),
        signer: std::sync::Mutex::new(None),
        secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
        hsd_child: std::sync::Mutex::new(None), node_rpc_alive: std::sync::atomic::AtomicBool::new(false), sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(crate::commands::sync::SyncStatus::default()))
    }
}

/// Build a valid ExtendedPubKey for tests.
fn test_xpub() -> crate::noncustodial::hd::ExtendedPubKey {
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = crate::noncustodial::hd::ExtendedPrivKey::from_seed(&seed).unwrap();
    crate::noncustodial::hd::ExtendedPubKey::from_priv(&master)
}

// --- random_id tests ---

#[test]
fn test_random_id_length() {
    let id = secure_wallet::random_id();
    assert_eq!(id.len(), 32);
}

#[test]
fn test_random_id_hex() {
    let id = secure_wallet::random_id();
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_random_id_unique() {
    let a = secure_wallet::random_id();
    let b = secure_wallet::random_id();
    assert_ne!(a, b);
}

// --- validate_network tests ---

#[test]
fn test_validate_network_mainnet() {
    let (s, net) = secure_wallet::validate_network("mainnet").unwrap();
    assert_eq!(s, "mainnet");
    assert_eq!(net, crate::noncustodial::network::Network::Main);
}

#[test]
fn test_validate_network_testnet() {
    let (s, net) = secure_wallet::validate_network("testnet").unwrap();
    assert_eq!(s, "testnet");
    assert_eq!(net, crate::noncustodial::network::Network::Testnet);
}

#[test]
fn test_validate_network_regtest() {
    let (s, net) = secure_wallet::validate_network("regtest").unwrap();
    assert_eq!(s, "regtest");
    assert_eq!(net, crate::noncustodial::network::Network::Regtest);
}

#[test]
fn test_validate_network_invalid() {
    let result = secure_wallet::validate_network("invalid");
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("unsupported network"));
}

#[test]
fn test_validate_network_empty() {
    let result = secure_wallet::validate_network("");
    assert!(result.is_err());
}

// --- fingerprint tests ---

#[test]
fn test_fingerprint_deterministic() {
    let fp1 = secure_wallet::fingerprint("xpub6SomeTest");
    let fp2 = secure_wallet::fingerprint("xpub6SomeTest");
    assert_eq!(fp1, fp2);
}

#[test]
fn test_fingerprint_length() {
    let fp = secure_wallet::fingerprint("anything");
    assert_eq!(fp.len(), 16); // 8 bytes = 16 hex chars
}

#[test]
fn test_fingerprint_different_inputs() {
    let fp1 = secure_wallet::fingerprint("xpub_a");
    let fp2 = secure_wallet::fingerprint("xpub_b");
    assert_ne!(fp1, fp2);
}

#[test]
fn test_fingerprint_empty_string() {
    let fp = secure_wallet::fingerprint("");
    assert_eq!(fp.len(), 16);
}

// --- gap_limit tests ---

#[test]
fn test_gap_limit_default() {
    let settings = std::collections::HashMap::new();
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_gap_limit_custom() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".into(), "50".into());
    assert_eq!(secure_wallet::gap_limit(&settings), 50);
}

#[test]
fn test_gap_limit_zero_falls_back() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".into(), "0".into());
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_gap_limit_invalid_falls_back() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".into(), "abc".into());
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_gap_limit_negative_falls_back() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".into(), "-5".into());
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

// --- session_ttl_ms tests ---

#[test]
fn test_session_ttl_ms_default() {
    let settings = std::collections::HashMap::new();
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 900_000);
}

#[test]
fn test_session_ttl_ms_custom() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("signer_session_timeout_seconds".into(), "60".into());
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 60_000);
}

#[test]
fn test_session_ttl_ms_zero_falls_back() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("signer_session_timeout_seconds".into(), "0".into());
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 900_000);
}

#[test]
fn test_session_ttl_ms_invalid_falls_back() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("signer_session_timeout_seconds".into(), "xyz".into());
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 900_000);
}

// --- resolve_secret_key tests ---

#[test]
fn test_resolve_secret_key_empty() {
    let (key, kdf) = secure_wallet::resolve_secret_key("");
    assert_eq!(kdf, "none");
    assert_eq!(key, "namehold::no-passphrase::v1");
}

#[test]
fn test_resolve_secret_key_with_passphrase() {
    let (key, kdf) = secure_wallet::resolve_secret_key("my-secret");
    assert_eq!(kdf, "argon2id");
    assert_eq!(key, "my-secret");
}

#[test]
fn test_resolve_secret_key_whitespace() {
    let (key, kdf) = secure_wallet::resolve_secret_key(" ");
    assert_eq!(kdf, "argon2id");
    assert_eq!(key, " ");
}

// --- DB-backed command tests ---

#[tokio::test]
async fn test_list_wallet_profiles_empty() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let result = secure_wallet::list_wallet_profiles(app.state()).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[tokio::test]
async fn test_list_wallet_profiles_after_insert() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Test', 'watch_only_xpub', 'mainnet', 0, 'xpub_test', 1, datetime('now'))",
            [],
        ).unwrap();
    }
    let app = mock_app_with(state);
    let result = secure_wallet::list_wallet_profiles(app.state()).await;
    assert!(result.is_ok());
    let profiles = result.unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, "wp1");
}

#[tokio::test]
async fn test_set_active_wallet_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Test', 'watch_only_xpub', 'mainnet', 0, 'xpub_test', 1, datetime('now'))",
            [],
        ).unwrap();
    }
    let app = mock_app_with(state);
    let result = secure_wallet::set_active_wallet_profile(app.state(), "wp1".into()).await;
    assert!(result.is_ok());
    let summary = result.unwrap();
    assert!(summary.active);
    assert_eq!(summary.id, "wp1");
}

#[tokio::test]
async fn test_set_active_wallet_profile_not_found() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let result = secure_wallet::set_active_wallet_profile(app.state(), "nonexistent".into()).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("not found") || msg.contains("wallet profile"));
}

#[tokio::test]
async fn test_delete_wallet_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Test', 'watch_only_xpub', 'mainnet', 0, 'xpub_test', 1, datetime('now'))",
            [],
        ).unwrap();
    }
    let app = mock_app_with(state);
    let result = secure_wallet::delete_wallet_profile(app.state(), "wp1".into()).await;
    assert!(result.is_ok());

    // Verify it's gone
    let profiles = secure_wallet::list_wallet_profiles(app.state()).await.unwrap();
    assert!(profiles.is_empty());
}

#[tokio::test]
async fn test_delete_wallet_profile_clears_active() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Test', 'watch_only_xpub', 'mainnet', 0, 'xpub_test', 1, datetime('now'))",
            [],
        ).unwrap();
        crate::db::queries::set_active_profile(&conn, "wp1").unwrap();
    }
    let app = mock_app_with(state);
    secure_wallet::delete_wallet_profile(app.state(), "wp1".into()).await.unwrap();

    // Active should be cleared
    let state_ref = app.state::<AppState>();
    let conn = state_ref.db.lock().unwrap();
    let active = crate::db::queries::get_active_profile_id(&conn).unwrap();
    assert_eq!(active, "");
}

#[tokio::test]
async fn test_delete_nonexistent_profile() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let result = secure_wallet::delete_wallet_profile(app.state(), "nonexistent".into()).await;
    // delete_wallet_profile doesn't check existence first, it just deletes (0 rows affected is OK)
    assert!(result.is_ok());
}

// --- signer session tests ---

#[tokio::test]
async fn test_get_signer_session_locked() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let result = secure_wallet::get_signer_session(app.state()).await;
    assert!(result.is_ok());
    let summary = result.unwrap();
    assert!(!summary.unlocked);
}

#[tokio::test]
async fn test_lock_local_signer_when_already_locked() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let result = secure_wallet::lock_local_signer(app.state()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_get_signer_session_after_lock() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    // Lock (no-op when already locked)
    secure_wallet::lock_local_signer(app.state()).await.unwrap();
    // Should still report locked
    let summary = secure_wallet::get_signer_session(app.state()).await.unwrap();
    assert!(!summary.unlocked);
}

// --- set_active then list ---

#[tokio::test]
async fn test_set_active_then_list() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'First', 'watch_only_xpub', 'mainnet', 0, 'xpub1', 1, datetime('now'))",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp2', 'Second', 'watch_only_xpub', 'mainnet', 0, 'xpub2', 1, datetime('now'))",
            [],
        ).unwrap();
    }
    let app = mock_app_with(state);
    secure_wallet::set_active_wallet_profile(app.state(), "wp2".into()).await.unwrap();
    let profiles = secure_wallet::list_wallet_profiles(app.state()).await.unwrap();
    assert_eq!(profiles.len(), 2);
    // wp2 should be active
    let active = profiles.iter().find(|p| p.active).unwrap();
    assert_eq!(active.id, "wp2");
}

// --- set_active switches signer lock ---

#[tokio::test]
async fn test_set_active_different_profile_locks_signer() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'First', 'watch_only_xpub', 'mainnet', 0, 'xpub1', 1, datetime('now'))",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp2', 'Second', 'watch_only_xpub', 'mainnet', 0, 'xpub2', 1, datetime('now'))",
            [],
        ).unwrap();
    }
    let app = mock_app_with(state);

    // Set wp1 active
    secure_wallet::set_active_wallet_profile(app.state(), "wp1".into()).await.unwrap();
    // Switch to wp2 — signer should be locked (no signer was unlocked, but the code path is exercised)
    let summary = secure_wallet::set_active_wallet_profile(app.state(), "wp2".into()).await.unwrap();
    assert!(summary.active);
    assert_eq!(summary.id, "wp2");
}
