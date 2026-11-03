use crate::commands::secure_prompt::{
    push_test_answer, take_test_requests, SecurePromptRequest, SecurePromptResult,
};
use crate::commands::secure_wallet;
use crate::error::AppError;
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
        hsd_children: std::sync::Mutex::new(std::collections::HashMap::new()),
        hsd_starting: std::sync::Mutex::new(std::collections::HashSet::new()),
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
    // 60s TTL — comfortably unexpired during the test.
    seed_signer_with_ttl(state, wallet_profile_id, 60_000);
}

/// As [`seed_unlocked_signer`], with the session's TTL spelled out (0 is a
/// session whose time box has already elapsed).
fn seed_signer_with_ttl(state: &AppState, wallet_profile_id: &str, ttl_ms: u128) {
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = crate::noncustodial::hd::ExtendedPrivKey::from_seed(&seed).unwrap();
    let session = crate::noncustodial::session::SignerSession::unlock(
        wallet_profile_id.to_string(),
        crate::noncustodial::network::Network::Main,
        master,
        ttl_ms,
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
fn test_gap_limit_clamp_is_logged_once() {
    let flag = std::sync::atomic::AtomicBool::new(false);
    assert!(
        secure_wallet::log_clamp_once(&flag, 5000),
        "the first clamp logs"
    );
    assert!(
        !secure_wallet::log_clamp_once(&flag, 5000),
        "a repeat stays silent"
    );
    assert!(
        !secure_wallet::log_clamp_once(&flag, 9000),
        "so does another value"
    );
}

#[test]
fn test_gap_limit_logs_only_when_it_clamps() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let with = |raw: &str| {
        let mut settings = std::collections::HashMap::new();
        settings.insert("address_gap_limit".to_string(), raw.to_string());
        settings
    };
    for raw in ["50", "1000", "0", "abc"] {
        let flag = AtomicBool::new(false);
        secure_wallet::gap_limit_logging(&with(raw), &flag);
        assert!(
            !flag.load(Ordering::Relaxed),
            "{raw:?} is not clamped and must not log"
        );
    }
    let flag = AtomicBool::new(false);
    let got = secure_wallet::gap_limit_logging(&with("5000"), &flag);
    assert_eq!(got, secure_wallet::MAX_GAP_LIMIT);
    assert!(flag.load(Ordering::Relaxed), "a clamped stored value logs");
}

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

/// F2: the setting is renderer-writable and every node sync derives and asks
/// the node about `gap` addresses per branch, so a huge value is clamped to
/// the cap; the cap itself is kept.
#[test]
fn test_gap_limit_is_capped() {
    let cap = secure_wallet::MAX_GAP_LIMIT;
    for (raw, want) in [
        (cap.to_string(), cap),
        ((cap + 1).to_string(), cap),
        (u32::MAX.to_string(), cap),
        ("1".to_string(), 1),
        // Past u32: not a number this setting can hold, the default.
        ("99999999999".to_string(), 20),
        (" 50".to_string(), 20),
        ("1e3".to_string(), 20),
    ] {
        let mut settings = std::collections::HashMap::new();
        settings.insert("address_gap_limit".into(), raw.clone());
        assert_eq!(secure_wallet::gap_limit(&settings), want, "{raw:?}");
    }
}

/// Settings refuses a gap limit past the backend's cap, with the same number.
#[test]
fn test_gap_limit_cap_matches_the_settings_field() {
    const UI: &str = include_str!("../../../src/lib/settingsDefaults.ts");
    let decl = format!(
        "export const MAX_ADDRESS_GAP_LIMIT = {};",
        secure_wallet::MAX_GAP_LIMIT
    );
    assert!(UI.contains(&decl), "settingsDefaults.ts lacks `{decl}`");
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

use crate::noncustodial::network::Network;

/// A test-network wallet may opt out of a passphrase: its seed is then
/// encrypted under the device-local key.
#[test]
fn a_test_network_wallet_may_have_no_passphrase() {
    for net in [Network::Testnet, Network::Regtest] {
        let (key, kdf) = secure_wallet::resolve_secret_key("", net).unwrap();
        assert_eq!(kdf, "none");
        assert!(!key.is_empty());
    }
}

/// A mainnet wallet cannot opt out, nor take a passphrase too short to stand
/// between a copy of the database and its seed (the device-local key is in
/// the public source).
#[test]
fn a_mainnet_wallet_needs_a_passphrase_of_at_least_eight_characters() {
    for weak in ["", " ", "a", "1234567", "seven77"] {
        let err = secure_wallet::resolve_secret_key(weak, Network::Main).unwrap_err();
        assert!(
            err.to_string().contains("at least 8 characters"),
            "{weak:?}: {err}"
        );
    }
    let (key, kdf) = secure_wallet::resolve_secret_key("eight888", Network::Main).unwrap();
    assert_eq!((key.as_str(), kdf), ("eight888", "argon2id"));
    // Characters, not bytes: eight non-ASCII characters are enough.
    assert!(secure_wallet::resolve_secret_key("пароль12", Network::Main).is_ok());
}

#[test]
fn test_resolve_secret_key_with_passphrase() {
    let (key, kdf) = secure_wallet::resolve_secret_key("my-secret", Network::Regtest).unwrap();
    assert_eq!(key.as_str(), "my-secret");
    assert_eq!(kdf, "argon2id");
}

#[test]
fn test_resolve_secret_key_whitespace() {
    // A single space is a passphrase (not "none") on a test network.
    let (key, kdf) = secure_wallet::resolve_secret_key(" ", Network::Regtest).unwrap();
    assert_eq!(key.as_str(), " ");
    assert_eq!(kdf, "argon2id");
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

/// Activating a profile (create, every import kind, Ledger) must drop an
/// unlocked signer that belongs to ANOTHER profile, or the UI shows "Unlocked"
/// while every use of the key is refused.
#[test]
fn test_activate_profile_drops_another_profiles_signer() {
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "old");
    secure_wallet::activate_profile(&state, "new").unwrap();
    assert!(state.signer.lock().unwrap().is_none());
    let conn = state.db.lock().unwrap();
    assert_eq!(
        crate::db::queries::get_active_profile_id(&conn).unwrap(),
        "new"
    );
}

#[test]
fn test_write_capability_signer_counts_only_for_its_own_profile() {
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "a");
    let mut slot = state.signer.lock().unwrap();
    assert!(crate::commands::tx::signer_unlocked_for(slot.as_mut(), "a"));
    assert!(!crate::commands::tx::signer_unlocked_for(
        slot.as_mut(),
        "b"
    ));
    assert!(!crate::commands::tx::signer_unlocked_for(slot.as_mut(), ""));
    assert!(!crate::commands::tx::signer_unlocked_for(None, "a"));
}

/// A Ledger signs on the device, so it never needs a session: without this its
/// write capability read "Unlock your wallet" forever and Send stayed disabled,
/// while the backend refuses to unlock a Ledger at all.
#[test]
fn test_write_capability_ledger_is_ready_without_a_session() {
    use crate::commands::tx::signer_ready_for;
    assert!(signer_ready_for(None, "l", Some("ledger_hardware")));
    assert!(!signer_ready_for(None, "h", Some("mnemonic_hot")));
    assert!(!signer_ready_for(None, "h", None));
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "h");
    let mut slot = state.signer.lock().unwrap();
    assert!(signer_ready_for(slot.as_mut(), "h", Some("mnemonic_hot")));
    assert!(!signer_ready_for(slot.as_mut(), "other", Some("xpriv_hot")));
}

#[test]
fn test_activate_profile_keeps_its_own_signer() {
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "same");
    secure_wallet::activate_profile(&state, "same").unwrap();
    let slot = state.signer.lock().unwrap();
    assert_eq!(slot.as_ref().unwrap().wallet_profile_id(), "same");
}

#[test]
fn test_import_prompts_name_their_input_kind() {
    let xpub = serde_json::to_value(secure_wallet::xpub_import_request()).unwrap();
    assert_eq!(xpub["mode"], "import");
    assert_eq!(xpub["input"], "xpub");
    let phrase = serde_json::to_value(secure_wallet::phrase_import_request()).unwrap();
    assert_eq!(phrase["mode"], "import");
    assert_eq!(phrase["input"], "recovery_phrase");
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

/// The secure window's answer to a `confirm` prompt: confirmed or not, and
/// what was typed into it.
fn confirm_answer(confirmed: bool, typed: Option<&str>) -> SecurePromptResult {
    SecurePromptResult {
        value: typed.map(|t| t.to_string().into()),
        confirmed,
    }
}

/// A recovery-phrase profile `id` labelled `label`, with its encrypted
/// recovery phrase stored (a `wallet_secrets` row).
fn insert_hot_profile_with_seed(state: &AppState, id: &str, label: &str) {
    let conn = state.db.lock().unwrap();
    crate::db::queries::insert_wallet_profile(
        &conn,
        id,
        label,
        "mnemonic_hot",
        "mainnet",
        &format!("xpub_{id}"),
        0,
        false,
    )
    .unwrap();
    crate::db::queries::insert_wallet_secret(
        &conn,
        id,
        b"encrypted-vault-blob",
        "argon2id",
        &secure_wallet::fingerprint(&format!("xpub_{id}")),
    )
    .unwrap();
}

fn insert_profile_of_kind(state: &AppState, id: &str, label: &str, kind: &str, watch_only: bool) {
    let conn = state.db.lock().unwrap();
    crate::db::queries::insert_wallet_profile(
        &conn,
        id,
        label,
        kind,
        "mainnet",
        &format!("xpub_{id}"),
        0,
        watch_only,
    )
    .unwrap();
}

fn has_seed(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> bool {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    crate::db::queries::get_wallet_secret_meta(&conn, id)
        .unwrap()
        .is_some()
}

fn has_profile(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> bool {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    crate::db::queries::get_wallet_profile(&conn, id)
        .unwrap()
        .is_some()
}

/// The rows of a `confirm` request as (label, value) pairs.
fn rows_of(request: &SecurePromptRequest) -> Vec<(String, String)> {
    request.details.as_ref().unwrap()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["label"].as_str().unwrap().to_string(),
                r["value"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn row<'a>(rows: &'a [(String, String)], label: &str) -> Option<&'a str> {
    rows.iter()
        .find(|(l, _)| l == label)
        .map(|(_, v)| v.as_str())
}

/// A watch-only profile holds no secret: one click in the secure window
/// deletes it, after it showed which wallet it is.
#[tokio::test]
async fn deleting_a_watch_only_profile_asks_the_secure_window_without_typed_text() {
    let state = create_full_test_state();
    insert_profile_of_kind(&state, "wp1", "Watcher", "watch_only_xpub", true);
    let app = mock_app_with(state);
    push_test_answer(confirm_answer(true, None));

    secure_wallet::delete_wallet_profile(app.handle().clone(), "wp1".into())
        .await
        .unwrap();

    assert!(!has_profile(&app, "wp1"));
    let requests = take_test_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].mode, "confirm");
    let details = requests[0].details.as_ref().unwrap();
    assert!(details.get("confirmText").is_none(), "{details}");
    assert_eq!(details["confirmLabel"], "Delete wallet");
    let rows = rows_of(&requests[0]);
    assert_eq!(row(&rows, "Wallet"), Some("Watcher"));
    assert_eq!(row(&rows, "Network"), Some("mainnet"));
    assert_eq!(row(&rows, "Kind"), Some("Watch-only"));
    assert_eq!(
        row(&rows, "Fingerprint"),
        Some(secure_wallet::fingerprint("xpub_wp1").as_str())
    );
    assert_eq!(row(&rows, "Warning"), None);
}

#[tokio::test]
async fn deleting_a_watch_only_profile_declined_keeps_it() {
    let state = create_full_test_state();
    insert_profile_of_kind(&state, "wp1", "Watcher", "watch_only_xpub", true);
    let app = mock_app_with(state);
    push_test_answer(confirm_answer(false, None));

    let err = secure_wallet::delete_wallet_profile(app.handle().clone(), "wp1".into())
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::UserRejected), "{err:?}");
    assert!(has_profile(&app, "wp1"));
}

#[tokio::test]
async fn deleting_a_ledger_profile_asks_without_typed_text() {
    let state = create_full_test_state();
    insert_profile_of_kind(&state, "led", "Nano", "ledger_hardware", false);
    let app = mock_app_with(state);
    push_test_answer(confirm_answer(true, None));

    secure_wallet::delete_wallet_profile(app.handle().clone(), "led".into())
        .await
        .unwrap();

    assert!(!has_profile(&app, "led"));
    let requests = take_test_requests();
    assert!(requests[0]
        .details
        .as_ref()
        .unwrap()
        .get("confirmText")
        .is_none());
    assert_eq!(row(&rows_of(&requests[0]), "Kind"), Some("Ledger"));
}

/// The only copy of a hot wallet's encrypted recovery phrase on this device
/// is never erased by a call alone: a declined secure window deletes nothing.
#[tokio::test]
async fn delete_hot_profile_refused_when_the_secure_window_declines() {
    let state = create_full_test_state();
    insert_hot_profile_with_seed(&state, "hot", "Savings");
    let app = mock_app_with(state);
    push_test_answer(confirm_answer(false, None));

    let err = secure_wallet::delete_wallet_profile(app.handle().clone(), "hot".into())
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::UserRejected), "{err:?}");
    assert!(has_profile(&app, "hot"));
    assert!(has_seed(&app, "hot"));
    let requests = take_test_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].mode, "confirm");
    let rows = rows_of(&requests[0]);
    assert_eq!(row(&rows, "Wallet"), Some("Savings"));
    assert_eq!(row(&rows, "Kind"), Some("Hot"));
    assert_eq!(
        row(&rows, "Fingerprint"),
        Some(secure_wallet::fingerprint("xpub_hot").as_str())
    );
    assert_eq!(
        row(&rows, "Warning"),
        Some(secure_wallet::DELETE_SEED_WARNING)
    );
    assert_eq!(
        requests[0].details.as_ref().unwrap()["confirmText"],
        "Savings"
    );
}

/// Confirm alone is not enough for a stored seed: the label must be typed,
/// and the backend checks the typed text itself.
#[tokio::test]
async fn delete_hot_profile_refused_without_the_typed_label() {
    for typed in [None, Some(""), Some("savings"), Some("Saving")] {
        let state = create_full_test_state();
        insert_hot_profile_with_seed(&state, "hot", "Savings");
        let app = mock_app_with(state);
        push_test_answer(confirm_answer(true, typed));

        let err = secure_wallet::delete_wallet_profile(app.handle().clone(), "hot".into())
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::UserRejected), "{typed:?}: {err:?}");
        assert!(has_profile(&app, "hot"), "{typed:?}");
        assert!(has_seed(&app, "hot"), "{typed:?}");
    }
}

#[tokio::test]
async fn delete_hot_profile_proceeds_after_confirmation_with_the_typed_label() {
    let state = create_full_test_state();
    insert_hot_profile_with_seed(&state, "hot", "Savings");
    insert_profile_of_kind(&state, "other", "Other", "watch_only_xpub", true);
    let app = mock_app_with(state);
    push_test_answer(confirm_answer(true, Some(" Savings ")));

    secure_wallet::delete_wallet_profile(app.handle().clone(), "hot".into())
        .await
        .unwrap();

    assert!(!has_profile(&app, "hot"));
    assert!(!has_seed(&app, "hot"));
    assert!(has_profile(&app, "other"));
}

/// A blank label cannot be typed back; the fingerprint shown in the rows is
/// asked for instead, so such a wallet can still be deleted.
#[tokio::test]
async fn delete_hot_profile_with_a_blank_label_asks_for_its_fingerprint() {
    let state = create_full_test_state();
    insert_hot_profile_with_seed(&state, "hot", "  ");
    let app = mock_app_with(state);
    let fp = secure_wallet::fingerprint("xpub_hot");
    push_test_answer(confirm_answer(true, Some(&fp)));

    secure_wallet::delete_wallet_profile(app.handle().clone(), "hot".into())
        .await
        .unwrap();

    assert!(!has_profile(&app, "hot"));
    assert_eq!(
        take_test_requests()[0].details.as_ref().unwrap()["confirmText"],
        fp.as_str()
    );
}

/// What was confirmed is what is deleted: a profile that gained a stored
/// recovery phrase while the window was open is not deleted on a one-click
/// confirmation meant for a watch-only wallet.
#[tokio::test]
async fn delete_refused_when_the_profile_gains_a_seed_while_the_window_is_open() {
    let state = create_full_test_state();
    insert_profile_of_kind(&state, "wp1", "Watcher", "watch_only_xpub", true);
    let app = mock_app_with(state);
    let handle = app.handle().clone();
    crate::commands::secure_prompt::on_next_test_answer(move || {
        let state = handle.state::<AppState>();
        let conn = state.db.lock().unwrap();
        crate::db::queries::insert_wallet_secret(&conn, "wp1", b"blob", "argon2id", "fp").unwrap();
    });
    push_test_answer(confirm_answer(true, None));

    let err = secure_wallet::delete_wallet_profile(app.handle().clone(), "wp1".into())
        .await
        .unwrap_err();

    match err {
        AppError::Other(m) => assert_eq!(m, secure_wallet::DELETE_PROFILE_CHANGED),
        other => panic!("{other:?}"),
    }
    assert!(has_profile(&app, "wp1"));
    assert!(has_seed(&app, "wp1"));
}

/// The other direction: the seed the user was warned about is already gone,
/// so what they confirmed is not what would be deleted.
#[tokio::test]
async fn delete_refused_when_the_profile_loses_its_seed_while_the_window_is_open() {
    let state = create_full_test_state();
    insert_hot_profile_with_seed(&state, "hot", "Savings");
    let app = mock_app_with(state);
    let handle = app.handle().clone();
    crate::commands::secure_prompt::on_next_test_answer(move || {
        let state = handle.state::<AppState>();
        let conn = state.db.lock().unwrap();
        conn.execute(
            "DELETE FROM wallet_secrets WHERE wallet_profile_id = 'hot'",
            [],
        )
        .unwrap();
    });
    push_test_answer(confirm_answer(true, Some("Savings")));

    let err = secure_wallet::delete_wallet_profile(app.handle().clone(), "hot".into())
        .await
        .unwrap_err();

    match err {
        AppError::Other(m) => assert_eq!(m, secure_wallet::DELETE_PROFILE_CHANGED),
        other => panic!("{other:?}"),
    }
    assert!(has_profile(&app, "hot"));
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
    push_test_answer(confirm_answer(true, None));
    secure_wallet::delete_wallet_profile(app.handle().clone(), "wp1".into())
        .await
        .unwrap();

    // Active should be cleared
    let state_ref = app.state::<AppState>();
    let conn = state_ref.db.lock().unwrap();
    let active = crate::db::queries::get_active_profile_id(&conn).unwrap();
    assert_eq!(active, "");
}

/// There is nothing to show for a profile that does not exist, so nothing is
/// asked and the call fails.
#[tokio::test]
async fn deleting_an_unknown_profile_is_not_found_and_asks_nothing() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let result =
        secure_wallet::delete_wallet_profile(app.handle().clone(), "nonexistent".into()).await;
    assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
    assert!(take_test_requests().is_empty());
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

/// An expired session is dropped, key and all, by the first look at it: the
/// UI's poll of the session state, not the next signing attempt.
#[tokio::test]
async fn get_signer_session_wipes_an_expired_slot() {
    let state = create_full_test_state();
    seed_signer_with_ttl(&state, "wp1", 0);
    let app = mock_app_with(state);

    let summary = secure_wallet::get_signer_session(app.state())
        .await
        .unwrap();

    assert!(!summary.unlocked);
    assert!(app.state::<AppState>().signer.lock().unwrap().is_none());
}

/// The once-a-second sweep `lib.rs` runs drops an expired session and
/// leaves a live one alone.
#[test]
fn sweep_expired_signer_drops_only_an_expired_session() {
    let state = create_full_test_state();
    assert!(!secure_wallet::sweep_expired_signer(&state));

    seed_unlocked_signer(&state, "live");
    assert!(!secure_wallet::sweep_expired_signer(&state));
    assert!(state.signer.lock().unwrap().is_some());

    seed_signer_with_ttl(&state, "expired", 0);
    assert!(secure_wallet::sweep_expired_signer(&state));
    assert!(state.signer.lock().unwrap().is_none());
}

/// While a command holds the signer (signing), the sweep skips its tick
/// rather than wait on the lock.
#[test]
fn sweep_expired_signer_skips_a_busy_slot() {
    let state = create_full_test_state();
    seed_signer_with_ttl(&state, "expired", 0);
    let held = state.signer.lock().unwrap();
    assert!(!secure_wallet::sweep_expired_signer(&state));
    drop(held);
    assert!(secure_wallet::sweep_expired_signer(&state));
}

/// A panic while holding the signer poisons its lock for good, so no command
/// can reach the key again: the sweep empties the slot, expired or not.
#[test]
fn sweep_expired_signer_drops_the_key_behind_a_poisoned_lock() {
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "live");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = state.signer.lock().unwrap();
        panic!("poison the signer lock");
    }));
    assert!(state.signer.is_poisoned());

    assert!(secure_wallet::sweep_expired_signer(&state));
    assert!(state.signer.lock().err().unwrap().into_inner().is_none());
    assert!(!secure_wallet::sweep_expired_signer(&state));
}

/// README promises the key goes within a second of expiry even with the window
/// hidden. Nothing can run the app's setup hook under test, so the wiring is
/// read from the source (as `shakedex_layering_tests` does).
#[test]
fn the_app_runs_the_expiry_sweep_on_a_timer() {
    let lib_rs = include_str!("../lib.rs");
    let run = &lib_rs[lib_rs.find("pub fn run()").expect("lib.rs has run()")..];
    let code: Vec<&str> = run
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with("//"))
        .collect();
    let at = code
        .iter()
        .position(|l| l.contains("commands::secure_wallet::sweep_expired_signer("))
        .expect("run() calls sweep_expired_signer");
    assert!(
        code[at - 3..at].iter().any(|l| l.contains("loop {")),
        "the sweep runs in a loop"
    );
    assert!(
        code[at - 4..at]
            .iter()
            .any(|l| l.contains("Duration::from_secs(1)")),
        "the sweep runs every second"
    );
}

/// Lock drops the unlocked key through its wiping `Drop`, not only empties
/// the slot.
#[tokio::test]
async fn lock_local_signer_wipes_the_unlocked_key() {
    let state = create_full_test_state();
    seed_unlocked_signer(&state, "wp1");
    let app = mock_app_with(state);
    let before = crate::noncustodial::hd::wipes_on_this_thread();

    secure_wallet::lock_local_signer(app.state()).await.unwrap();

    assert!(app.state::<AppState>().signer.lock().unwrap().is_none());
    assert_eq!(crate::noncustodial::hd::wipes_on_this_thread(), before + 1);
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
fn provision_addresses_for_a_profile_with_no_row_fails_and_stores_nothing() {
    let conn = create_full_test_db();
    let xpub_str = secure_wallet::account_xpub_from_seed(
        crate::noncustodial::network::Network::Main,
        &test_seed_32(),
        0,
    )
    .unwrap();

    let result = secure_wallet::provision_addresses(
        &conn,
        "wp_missing",
        crate::noncustodial::network::Network::Main,
        &xpub_str,
        3,
    );

    assert!(result.is_err(), "{result:?}");
    let count: i32 = conn
        .query_row("SELECT COUNT(*) FROM derived_addresses", [], |row| {
            row.get(0)
        })
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
    push_test_answer(confirm_answer(true, None));

    secure_wallet::delete_wallet_profile(app.handle().clone(), "wp1".into())
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
    push_test_answer(confirm_answer(true, None));

    secure_wallet::delete_wallet_profile(app.handle().clone(), "wp2".into())
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

// --- wallet creation: nothing exists until the phrase is confirmed ---

const NEW_PASSPHRASE: &str = "correct horse";

fn wallet_row_counts(app: &tauri::App<tauri::test::MockRuntime>) -> (i64, i64, i64) {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    (
        count("SELECT COUNT(*) FROM wallet_profiles"),
        count("SELECT COUNT(*) FROM wallet_secrets"),
        count("SELECT COUNT(*) FROM derived_addresses"),
    )
}

fn active_profile(app: &tauri::App<tauri::test::MockRuntime>) -> String {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    crate::db::queries::get_active_profile_id(&conn).unwrap()
}

/// Order: passphrase, reveal, then persist. What is stored is what was shown:
/// the vault holds the revealed phrase and the xpub comes from it.
#[tokio::test]
async fn create_wallet_persists_and_activates_after_the_reveal_is_confirmed() {
    let app = mock_app_with(create_full_test_state());
    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    push_test_answer(confirm_answer(true, None));

    let summary =
        secure_wallet::secure_create_wallet(app.handle().clone(), "New".into(), "regtest".into())
            .await
            .unwrap();

    let requests = take_test_requests();
    let modes: Vec<&str> = requests.iter().map(|r| r.mode.as_str()).collect();
    assert_eq!(modes, ["passphrase_new", "reveal"]);
    let revealed = requests[1].payload.as_ref().unwrap();
    assert_eq!(revealed.split(' ').count(), 12);

    assert_eq!(active_profile(&app), summary.id);
    let (blob, kdf) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        crate::db::queries::get_wallet_secret_meta(&conn, &summary.id)
            .unwrap()
            .unwrap()
    };
    assert_eq!(kdf, "argon2id");
    let stored = crate::noncustodial::vault::decrypt(&blob, NEW_PASSPHRASE).unwrap();
    assert_eq!(stored, revealed.as_bytes());
    let seed = crate::noncustodial::hd::seed_from_mnemonic(revealed, "").unwrap();
    assert_eq!(
        summary.account_xpub,
        secure_wallet::account_xpub_from_seed(Network::Regtest, &seed, 0).unwrap()
    );
}

/// A reveal closed or declined leaves no trace: no profile, no secret, no
/// addresses, the active wallet unchanged, and the error says so.
#[tokio::test]
async fn create_wallet_declined_reveal_creates_nothing_and_says_so() {
    let state = create_full_test_state();
    insert_profile_of_kind(&state, "old", "Old", "watch_only_xpub", true);
    {
        let conn = state.db.lock().unwrap();
        crate::db::queries::set_active_profile(&conn, "old").unwrap();
    }
    let app = mock_app_with(state);
    let before = wallet_row_counts(&app);
    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    push_test_answer(confirm_answer(false, None));

    let err =
        secure_wallet::secure_create_wallet(app.handle().clone(), "New".into(), "regtest".into())
            .await
            .unwrap_err();

    match err {
        AppError::Other(m) => assert_eq!(m, secure_wallet::CREATE_NOT_CONFIRMED),
        other => panic!("{other:?}"),
    }
    assert_eq!(wallet_row_counts(&app), before);
    assert_eq!(active_profile(&app), "old");
}

/// A reveal window that cannot open is not a reveal: nothing is saved and the
/// failure is reported.
#[tokio::test]
async fn create_wallet_with_a_reveal_window_that_cannot_open_creates_nothing() {
    let app = mock_app_with(create_full_test_state());
    let before = wallet_row_counts(&app);
    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    crate::commands::secure_prompt::push_test_prompt_failure();

    let result =
        secure_wallet::secure_create_wallet(app.handle().clone(), "New".into(), "regtest".into())
            .await;

    assert!(matches!(result, Err(AppError::Other(_))), "{result:?}");
    assert_eq!(wallet_row_counts(&app), before);
}

/// Cancelling the passphrase ends it before a phrase is generated or shown.
#[tokio::test]
async fn create_wallet_cancelled_passphrase_generates_and_shows_nothing() {
    let app = mock_app_with(create_full_test_state());
    push_test_answer(confirm_answer(false, None));

    let result =
        secure_wallet::secure_create_wallet(app.handle().clone(), "New".into(), "regtest".into())
            .await;

    assert!(
        matches!(result, Err(AppError::InvalidInput(_))),
        "{result:?}"
    );
    assert_eq!(take_test_requests().len(), 1);
    assert_eq!(wallet_row_counts(&app), (0, 0, 0));
}

/// A database failure after the confirmation does not leave half a wallet: a
/// profile without its secret would read as a wallet the user cannot unlock.
#[tokio::test]
async fn create_wallet_failing_to_store_the_secret_leaves_no_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER no_secret BEFORE INSERT ON wallet_secrets
             BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .unwrap();
    }
    let app = mock_app_with(state);
    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    push_test_answer(confirm_answer(true, None));

    let result =
        secure_wallet::secure_create_wallet(app.handle().clone(), "New".into(), "regtest".into())
            .await;

    assert!(result.is_err());
    assert_eq!(wallet_row_counts(&app), (0, 0, 0));
    assert_eq!(active_profile(&app), "");
}

// --- the secret paths still work now that they hold Zeroizing values ---

/// A wallet with a real vault, created through the command.
async fn created_wallet(app: &tauri::App<tauri::test::MockRuntime>) -> (String, String) {
    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    push_test_answer(confirm_answer(true, None));
    let id =
        secure_wallet::secure_create_wallet(app.handle().clone(), "New".into(), "regtest".into())
            .await
            .unwrap()
            .id;
    let requests = take_test_requests();
    let phrase = requests[1].payload.as_ref().unwrap().to_string();
    (id, phrase)
}

#[tokio::test]
async fn unlock_opens_the_vault_with_the_right_passphrase_only() {
    let app = mock_app_with(create_full_test_state());
    let (id, _) = created_wallet(&app).await;

    push_test_answer(confirm_answer(true, Some("wrong passphrase")));
    let err = secure_wallet::unlock_local_signer(app.handle().clone(), id.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Crypto(_)), "{err:?}");
    assert!(app.state::<AppState>().signer.lock().unwrap().is_none());

    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    let summary = secure_wallet::unlock_local_signer(app.handle().clone(), id.clone())
        .await
        .unwrap();
    assert!(summary.unlocked);
    assert_eq!(summary.wallet_profile_id.as_deref(), Some(id.as_str()));
}

#[tokio::test]
async fn reveal_backup_phrase_shows_the_stored_phrase_after_the_passphrase() {
    let app = mock_app_with(create_full_test_state());
    let (id, phrase) = created_wallet(&app).await;

    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    push_test_answer(confirm_answer(false, None));
    secure_wallet::secure_reveal_backup_phrase(app.handle().clone(), id)
        .await
        .unwrap();

    let requests = take_test_requests();
    assert_eq!(requests[0].mode, "passphrase");
    assert_eq!(requests[1].mode, "reveal");
    assert_eq!(requests[1].payload.as_ref().unwrap().as_str(), phrase);
}

/// "Show recovery phrase" must not look like it worked when the window could
/// not open.
#[tokio::test]
async fn reveal_backup_phrase_reports_a_window_that_cannot_open() {
    let app = mock_app_with(create_full_test_state());
    let (id, _phrase) = created_wallet(&app).await;

    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    crate::commands::secure_prompt::push_test_prompt_failure();
    let result = secure_wallet::secure_reveal_backup_phrase(app.handle().clone(), id).await;

    assert!(matches!(result, Err(AppError::Other(_))), "{result:?}");
}

/// Import reaches the vault with the typed phrase; a bad phrase or a cancelled
/// passphrase stores nothing.
#[tokio::test]
async fn import_stores_the_typed_phrase_and_nothing_on_failure() {
    // A throwaway phrase from fixed entropy; nothing holds funds under it.
    let phrase = bip39::Mnemonic::from_entropy(&[7u8; 16])
        .unwrap()
        .to_string();
    let app = mock_app_with(create_full_test_state());
    let import = || {
        secure_wallet::secure_import_wallet(
            app.handle().clone(),
            "Imported".into(),
            "regtest".into(),
            "mnemonic_hot".into(),
        )
    };

    push_test_answer(confirm_answer(true, Some("not a recovery phrase")));
    assert!(matches!(import().await, Err(AppError::InvalidInput(_))));
    push_test_answer(confirm_answer(true, Some(&phrase)));
    push_test_answer(confirm_answer(false, None));
    assert!(matches!(import().await, Err(AppError::InvalidInput(_))));
    assert_eq!(wallet_row_counts(&app), (0, 0, 0));

    push_test_answer(confirm_answer(true, Some(&phrase)));
    push_test_answer(confirm_answer(true, Some(NEW_PASSPHRASE)));
    let id = import().await.unwrap().id;
    let (blob, _) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        crate::db::queries::get_wallet_secret_meta(&conn, &id)
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        crate::noncustodial::vault::decrypt(&blob, NEW_PASSPHRASE).unwrap(),
        phrase.as_bytes()
    );
}
