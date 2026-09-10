use crate::commands::secure_wallet;
use crate::AppState;
use tauri::Manager;

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

/// Create a test DB with ALL migrations applied (idempotent via schema_version).
fn create_full_test_db() -> rusqlite::Connection {
    let conn = crate::tests::command_helpers::create_test_db();
    crate::db::migrations::run(&conn).unwrap();
    conn
}

fn create_full_test_state() -> AppState {
    let conn = create_full_test_db();
    AppState {
        db: std::sync::Mutex::new(conn),
        signer: std::sync::Mutex::new(None),
        secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
        hsd_child: std::sync::Mutex::new(None),
        node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
        sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::commands::sync::SyncStatus::default(),
        )),
    }
}

/// Build a valid ExtendedPubKey for tests.
fn test_xpub() -> crate::noncustodial::hd::ExtendedPubKey {
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = crate::noncustodial::hd::ExtendedPrivKey::from_seed(&seed).unwrap();
    crate::noncustodial::hd::ExtendedPubKey::from_priv(&master)
}

/// Insert an unlocked [`SignerSession`] into `state.signer` for the given
/// profile id. Lets tests exercise the "signer is unlocked" branches of
/// `get_signer_session`, `set_active_wallet_profile`, and
/// `delete_wallet_profile` without going through the interactive unlock flow.
fn seed_unlocked_signer(state: &AppState, wallet_profile_id: &str) {
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = crate::noncustodial::hd::ExtendedPrivKey::from_seed(&seed).unwrap();
    let session = crate::noncustodial::session::SignerSession::unlock(
        wallet_profile_id.to_string(),
        crate::noncustodial::network::Network::Main,
        master,
        60_000, // 60s TTL — comfortably unexpired during the test.
    );
    let mut slot = state.signer.lock().unwrap();
    *slot = Some(session);
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
    let profiles = secure_wallet::list_wallet_profiles(app.state())
        .await
        .unwrap();
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
    secure_wallet::delete_wallet_profile(app.state(), "wp1".into())
        .await
        .unwrap();

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
    let summary = secure_wallet::get_signer_session(app.state())
        .await
        .unwrap();
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
    secure_wallet::set_active_wallet_profile(app.state(), "wp2".into())
        .await
        .unwrap();
    let profiles = secure_wallet::list_wallet_profiles(app.state())
        .await
        .unwrap();
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
    secure_wallet::set_active_wallet_profile(app.state(), "wp1".into())
        .await
        .unwrap();
    // Switch to wp2 — signer should be locked (no signer was unlocked, but the code path is exercised)
    let summary = secure_wallet::set_active_wallet_profile(app.state(), "wp2".into())
        .await
        .unwrap();
    assert!(summary.active);
    assert_eq!(summary.id, "wp2");
}

// --- account_xpub_from_seed tests ---

/// Fixed 32-byte test seed (deterministic; not a real BIP39 seed).
fn test_seed_32() -> Vec<u8> {
    hex::decode("c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e9a3d9bd").unwrap()
}

#[test]
fn test_account_xpub_from_seed_mainnet_produces_base58check() {
    let seed = test_seed_32();
    let xpub = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        0,
    )
    .unwrap();
    // Should be a non-empty base58check string.
    assert!(!xpub.is_empty());
    // Round-trip parse succeeds under mainnet.
    let parsed = crate::noncustodial::hd::ExtendedPubKey::from_xpub(
        crate::noncustodial::network::Network::Main,
        &xpub,
    );
    assert!(parsed.is_ok(), "mainnet xpub must round-trip");
}

#[test]
fn test_account_xpub_from_seed_testnet_produces_base58check() {
    let seed = test_seed_32();
    let xpub = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Testnet,
        &seed,
        0,
    )
    .unwrap();
    assert!(!xpub.is_empty());
    let parsed = crate::noncustodial::hd::ExtendedPubKey::from_xpub(
        crate::noncustodial::network::Network::Testnet,
        &xpub,
    );
    assert!(parsed.is_ok(), "testnet xpub must round-trip");
}

#[test]
fn test_account_xpub_from_seed_regtest_produces_base58check() {
    let seed = test_seed_32();
    let xpub = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Regtest,
        &seed,
        0,
    )
    .unwrap();
    assert!(!xpub.is_empty());
}

#[test]
fn test_account_xpub_from_seed_different_accounts_differ() {
    let seed = test_seed_32();
    let xpub0 = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        0,
    )
    .unwrap();
    let xpub1 = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        1,
    )
    .unwrap();
    // Different BIP44 account indices must produce different xpubs.
    assert_ne!(xpub0, xpub1);
}

#[test]
fn test_account_xpub_from_seed_deterministic() {
    let seed = test_seed_32();
    let a = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        0,
    )
    .unwrap();
    let b = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        0,
    )
    .unwrap();
    assert_eq!(a, b);
}

// --- provision_addresses tests ---

/// Insert a minimal watch-only wallet profile row (required by the FK on
/// `derived_addresses.wallet_profile_id`).
fn insert_test_profile(conn: &rusqlite::Connection, id: &str, network: &str, xpub: &str) {
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
         VALUES (?1, 'Test', 'watch_only_xpub', ?2, 0, ?3, 1, datetime('now'))",
        rusqlite::params![id, network, xpub],
    )
    .unwrap();
}

#[test]
fn test_provision_addresses_creates_receive_and_change() {
    let conn = create_full_test_db();
    let seed = test_seed_32();
    let xpub_str = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        0,
    )
    .unwrap();
    insert_test_profile(&conn, "wp_prov", "mainnet", &xpub_str);

    let first = secure_wallet::provision_addresses(
        &conn,
        "wp_prov",
        crate::noncustodial::network::Network::Main,
        &xpub_str,
        5,
    )
    .expect("provision_addresses");
    assert!(!first.is_empty(), "must return the first receive address");

    // gap*2 rows: 5 receive + 5 change.
    let count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = 'wp_prov'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 10);

    // Branch split: 5 rows on each of branch 0 (receive) and branch 1 (change).
    let receive: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = 'wp_prov' AND branch = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let change: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = 'wp_prov' AND branch = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(receive, 5);
    assert_eq!(change, 5);
}

#[test]
fn test_provision_addresses_invalid_xpub_errors() {
    let conn = create_full_test_db();
    insert_test_profile(&conn, "wp_bad", "mainnet", "xpub_placeholder");

    let result = secure_wallet::provision_addresses(
        &conn,
        "wp_bad",
        crate::noncustodial::network::Network::Main,
        "not-a-valid-xpub",
        5,
    );
    assert!(result.is_err());

    // Nothing should have been inserted.
    let count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = 'wp_bad'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_provision_addresses_idempotent() {
    let conn = create_full_test_db();
    let seed = test_seed_32();
    let xpub_str = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &seed,
        0,
    )
    .unwrap();
    insert_test_profile(&conn, "wp_idem", "mainnet", &xpub_str);

    let first1 = secure_wallet::provision_addresses(
        &conn,
        "wp_idem",
        crate::noncustodial::network::Network::Main,
        &xpub_str,
        3,
    )
    .unwrap();
    let first2 = secure_wallet::provision_addresses(
        &conn,
        "wp_idem",
        crate::noncustodial::network::Network::Main,
        &xpub_str,
        3,
    )
    .unwrap();

    // Re-provisioning is a no-op: same first receive address, no duplicate rows.
    assert_eq!(first1, first2);
    let count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = 'wp_idem'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 6);
}

// --- unlocked-signer branches -------------------------------------------------
//
// These tests seed an unlocked [`SignerSession`] directly into `state.signer`
// (bypassing the interactive secure-window unlock flow) so we can exercise the
// three "signer is unlocked" branches in `secure_wallet.rs` that the rest of
// the suite leaves cold:
//   * `get_signer_session` L564-568 — the `Some(s) if s.is_unlocked()` arm that
//     reports the unlocked profile id, `unlocked: true`, and the TTL deadline.
//   * `set_active_wallet_profile` L603-604 — the `*slot = None` line when the
//     caller switches away from the currently-unlocked profile.
//   * `delete_wallet_profile` L637-638 — the `*slot = None` line when the
//     currently-unlocked profile is the one being deleted.

#[tokio::test]
async fn get_signer_session_reports_unlocked_when_slot_populated() {
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "wp_unlocked");
    let app = mock_app_with(state);

    let summary = secure_wallet::get_signer_session(app.state())
        .await
        .expect("get_signer_session should not error");

    assert!(summary.unlocked, "unlocked branch not entered");
    assert_eq!(
        summary.wallet_profile_id.as_deref(),
        Some("wp_unlocked"),
        "summary must expose the unlocked profile id"
    );
    assert!(
        summary.unlocked_until_epoch_ms > 0,
        "unlocked_until_epoch_ms must be populated (was {})",
        summary.unlocked_until_epoch_ms
    );
}

#[tokio::test]
async fn set_active_wallet_profile_clears_signer_when_switching_away_from_unlocked() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'First', 'watch_only_xpub', 'mainnet', 0, 'xpub1', 1, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp2', 'Second', 'watch_only_xpub', 'mainnet', 0, 'xpub2', 1, datetime('now'))",
            [],
        )
        .unwrap();
    }
    // Signer is unlocked for wp1. Switching to wp2 must clear the slot so stale
    // key material can't be reused for a different wallet.
    seed_unlocked_signer(&state, "wp1");
    let app = mock_app_with(state);

    let summary = secure_wallet::set_active_wallet_profile(app.state(), "wp2".into())
        .await
        .expect("set_active_wallet_profile should succeed");
    assert_eq!(summary.id, "wp2");
    assert!(summary.active);

    // Slot must have been cleared by the `*slot = None` arm.
    let state_ref = app.state::<AppState>();
    let slot = state_ref.signer.lock().unwrap();
    assert!(
        slot.is_none(),
        "signer slot must be cleared when switching away from the unlocked profile"
    );
}

#[tokio::test]
async fn set_active_wallet_profile_preserves_signer_when_reselecting_same_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'First', 'watch_only_xpub', 'mainnet', 0, 'xpub1', 1, datetime('now'))",
            [],
        )
        .unwrap();
    }
    // Signer is unlocked for wp1. Re-selecting wp1 must leave the slot intact
    // — this asserts the `s.wallet_profile_id() != wallet_profile_id` guard
    // fires as `false`, so the `*slot = None` line is skipped.
    seed_unlocked_signer(&state, "wp1");
    let app = mock_app_with(state);

    secure_wallet::set_active_wallet_profile(app.state(), "wp1".into())
        .await
        .expect("set_active_wallet_profile should succeed");

    let state_ref = app.state::<AppState>();
    let slot = state_ref.signer.lock().unwrap();
    assert!(
        slot.is_some(),
        "signer slot must be preserved when re-selecting the currently-unlocked profile"
    );
    assert_eq!(
        slot.as_ref().unwrap().wallet_profile_id(),
        "wp1",
        "the preserved session must still belong to wp1"
    );
}

#[tokio::test]
async fn delete_wallet_profile_clears_signer_when_deleting_active_unlocked_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Active', 'watch_only_xpub', 'mainnet', 0, 'xpub1', 1, datetime('now'))",
            [],
        )
        .unwrap();
        crate::db::queries::set_active_profile(&conn, "wp1").unwrap();
    }
    seed_unlocked_signer(&state, "wp1");
    let app = mock_app_with(state);

    secure_wallet::delete_wallet_profile(app.state(), "wp1".into())
        .await
        .expect("delete_wallet_profile should succeed");

    // The `*slot = None` arm must have fired: signer slot is now empty.
    let state_ref = app.state::<AppState>();
    let slot = state_ref.signer.lock().unwrap();
    assert!(
        slot.is_none(),
        "signer slot must be cleared when deleting the currently-unlocked profile"
    );
}

#[tokio::test]
async fn delete_wallet_profile_preserves_signer_when_deleting_a_different_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Unlocked', 'watch_only_xpub', 'mainnet', 0, 'xpub1', 1, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp2', 'Other', 'watch_only_xpub', 'mainnet', 0, 'xpub2', 1, datetime('now'))",
            [],
        )
        .unwrap();
    }
    // Signer is unlocked for wp1; deleting wp2 must leave the slot alone.
    // This exercises the `s.wallet_profile_id() == wallet_profile_id` guard
    // returning `false`, so the `*slot = None` line inside the `if` is skipped.
    seed_unlocked_signer(&state, "wp1");
    let app = mock_app_with(state);

    secure_wallet::delete_wallet_profile(app.state(), "wp2".into())
        .await
        .expect("delete_wallet_profile should succeed");

    let state_ref = app.state::<AppState>();
    let slot = state_ref.signer.lock().unwrap();
    assert!(
        slot.is_some(),
        "signer slot must be preserved when deleting an unrelated profile"
    );
    assert_eq!(
        slot.as_ref().unwrap().wallet_profile_id(),
        "wp1",
        "the preserved session must still belong to wp1"
    );
}
