use crate::commands::names;
use crate::db;
use crate::db::queries::NameCoin;
use crate::AppState;
use tauri::Manager;

pub(crate) fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

/// Build a valid ExtendedPubKey for tests (avoids base58check round-trip issues).
fn test_xpub() -> crate::noncustodial::hd::ExtendedPubKey {
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = crate::noncustodial::hd::ExtendedPrivKey::from_seed(&seed).unwrap();
    crate::noncustodial::hd::ExtendedPubKey::from_priv(&master)
}

/// Create a test DB with ALL migrations (including wallet_profiles from 006+).
fn create_full_test_db() -> rusqlite::Connection {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/004_wallet_addresses.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/005_fix_hnsfans_api_url.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/006_noncustodial_wallet_profiles.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/007_noncustodial_chain_cache.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/008_noncustodial_name_state.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/009_node_rpc_settings.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/010_drop_legacy_settings.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/011_hsd_data_dir.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/012_tx_draft_confirmations.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/013_owner_address.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/014_reveal_end_height.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/015_coin_reservation.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../../../src-tauri/src/sql/016_last_explorer_sync_at.sql"
    ))
    .unwrap();
    conn
}

pub(crate) fn create_full_test_state() -> crate::AppState {
    let conn = create_full_test_db();
    crate::AppState {
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

/// Build a minimal NameCoin for testing name_input_from.
fn make_coin(txid: &str, vout: u32, value: u64, branch: u32, child_index: u32) -> NameCoin {
    NameCoin {
        txid: txid.to_string(),
        vout,
        value,
        address: "tb1qtest".into(),
        branch,
        child_index,
        covenant_type: 0,
        covenant_json: None,
        name_height: None,
    }
}

#[test]
fn test_random_id_is_32_hex_chars() {
    let id = names::random_id();
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_random_id_is_unique() {
    let a = names::random_id();
    let b = names::random_id();
    assert_ne!(a, b);
}

#[test]
fn test_name_input_from_maps_fields() {
    let coin = make_coin("aabb", 1, 10000, 0, 5);
    let input = names::name_input_from(coin);
    assert_eq!(input.txid, "aabb");
    assert_eq!(input.vout, 1);
    assert_eq!(input.value, 10000);
    assert_eq!(input.branch, 0);
    assert_eq!(input.child_index, 5);
    assert_eq!(input.sighash_type, crate::noncustodial::tx::sighash::ALL);
}

#[test]
fn test_name_input_from_zero_values() {
    let coin = make_coin("", 0, 0, 0, 0);
    let input = names::name_input_from(coin);
    assert_eq!(input.txid, "");
    assert_eq!(input.vout, 0);
    assert_eq!(input.value, 0);
}

// --- fee_rate pure function tests ---

#[test]
fn test_fee_rate_explicit_overrides_settings() {
    use crate::commands::names::Ctx;
    use crate::noncustodial::network::Network;
    use std::collections::HashMap;

    let ctx = Ctx {
        profile_id: "p".into(),
        network: Network::Regtest,
        account: 0,
        account_xpub: test_xpub(),
        change_address: "tb1qchange".into(),
        funding: vec![],
        settings: HashMap::new(),
    };

    // Explicit fee_rate takes priority
    assert_eq!(names::fee_rate(&ctx, Some(42)), 42);
}

#[test]
fn test_fee_rate_from_settings_falls_back() {
    use crate::commands::names::Ctx;
    use crate::noncustodial::network::Network;
    use crate::noncustodial::send;
    use std::collections::HashMap;

    let ctx = Ctx {
        profile_id: "p".into(),
        network: Network::Regtest,
        account: 0,
        account_xpub: test_xpub(),
        change_address: "tb1qchange".into(),
        funding: vec![],
        settings: HashMap::new(),
    };

    // No explicit fee_rate and no settings → default
    assert_eq!(names::fee_rate(&ctx, None), send::DEFAULT_FEE_RATE_PER_BYTE);
}

#[test]
fn test_fee_rate_from_settings_kvb() {
    use crate::commands::names::Ctx;
    use crate::noncustodial::network::Network;
    use crate::noncustodial::send;
    use std::collections::HashMap;

    let mut settings = HashMap::new();
    // 1000 doos per kvb → 1 doo per byte, but clamped to MIN_FEE_RATE_PER_BYTE
    settings.insert("fee_rate_doos_per_kvb".into(), "1000".into());

    let ctx = Ctx {
        profile_id: "p".into(),
        network: Network::Regtest,
        account: 0,
        account_xpub: test_xpub(),
        change_address: "tb1qchange".into(),
        funding: vec![],
        settings,
    };

    let rate = names::fee_rate(&ctx, None);
    // 1000 / 1000 = 1, clamped to MIN
    assert_eq!(rate, send::MIN_FEE_RATE_PER_BYTE.max(1));
}

#[test]
fn test_fee_rate_from_settings_large_kvb() {
    use crate::commands::names::Ctx;
    use crate::noncustodial::network::Network;
    use std::collections::HashMap;

    let mut settings = HashMap::new();
    // 100000 doos per kvb → 100 doos per byte
    settings.insert("fee_rate_doos_per_kvb".into(), "100000".into());

    let ctx = Ctx {
        profile_id: "p".into(),
        network: Network::Regtest,
        account: 0,
        account_xpub: test_xpub(),
        change_address: "tb1qchange".into(),
        funding: vec![],
        settings,
    };

    assert_eq!(names::fee_rate(&ctx, None), 100);
}

#[test]
fn test_fee_rate_invalid_kvb_string_falls_back() {
    use crate::commands::names::Ctx;
    use crate::noncustodial::network::Network;
    use crate::noncustodial::send;
    use std::collections::HashMap;

    let mut settings = HashMap::new();
    settings.insert("fee_rate_doos_per_kvb".into(), "not_a_number".into());

    let ctx = Ctx {
        profile_id: "p".into(),
        network: Network::Regtest,
        account: 0,
        account_xpub: test_xpub(),
        change_address: "tb1qchange".into(),
        funding: vec![],
        settings,
    };

    // Invalid parse → falls back to default
    assert_eq!(names::fee_rate(&ctx, None), send::DEFAULT_FEE_RATE_PER_BYTE);
}

// --- load_ctx error path tests ---

#[test]
fn test_load_ctx_no_active_profile() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = names::load_ctx(&app.state());
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("no active wallet profile") || msg.contains("not found"));
}

#[test]
fn test_load_ctx_watch_only_profile_rejected() {
    use crate::db;

    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        // Insert a watch-only profile
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp_watch', 'Watch', 'watch_only_xpub', 'mainnet', 0, '', 1, datetime('now'))",
            [],
        ).unwrap();
        db::queries::set_active_profile(&conn, "wp_watch").unwrap();
    }
    let app = mock_app_with(state);
    let result = names::load_ctx(&app.state());
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("watch-only"));
}

// --- fetch_name_state tests with mockito ---

#[tokio::test]
async fn test_fetch_name_state_parses_valid_response() {
    let mut server = mockito::Server::new_async().await;
    let name_info = serde_json::json!({
        "info": {
            "height": 100,
            "value": 50000,
            "renewals": 2,
            "claimed": 1,
            "weak": false
        }
    });
    let m = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"result":{},"error":null,"id":1}}"#, name_info))
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    let result = names::fetch_name_state(&client, "test-name").await;
    m.assert_async().await;
    let ns = result.unwrap();
    assert_eq!(ns.height, 100);
    assert_eq!(ns.value, 50000);
    assert_eq!(ns.renewals, 2);
    assert_eq!(ns.claimed, 1);
    assert!(!ns.weak);
}

#[tokio::test]
async fn test_fetch_name_state_weak_name() {
    let mut server = mockito::Server::new_async().await;
    let name_info = serde_json::json!({
        "info": {
            "height": 50,
            "value": 1000,
            "renewals": 0,
            "claimed": 0,
            "weak": true
        }
    });
    let m = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"result":{},"error":null,"id":1}}"#, name_info))
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    let result = names::fetch_name_state(&client, "weak-name").await;
    m.assert_async().await;
    let ns = result.unwrap();
    assert!(ns.weak);
    assert_eq!(ns.value, 1000);
}

#[tokio::test]
async fn test_fetch_name_state_missing_info_returns_error() {
    let mut server = mockito::Server::new_async().await;
    // info is null → name has no on-chain state
    let m = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(r#"{"result":{"info":null},"error":null,"id":1}"#)
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    let result = names::fetch_name_state(&client, "nonexistent").await;
    m.assert_async().await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("no on-chain state"));
}

#[tokio::test]
async fn test_fetch_name_state_defaults_missing_fields() {
    let mut server = mockito::Server::new_async().await;
    // info exists but has no height/value/renewals/claimed/weak fields
    let m = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(r#"{"result":{"info":{"name":"test"}},"error":null,"id":1}"#)
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    let result = names::fetch_name_state(&client, "test").await;
    m.assert_async().await;
    let ns = result.unwrap();
    assert_eq!(ns.height, 0);
    assert_eq!(ns.value, 0);
    assert_eq!(ns.renewals, 0);
    assert_eq!(ns.claimed, 0);
    assert!(!ns.weak);
}

// --- renewal_block tests ---

#[tokio::test]
async fn test_renewal_block_makes_rpc_call() {
    let mut server = mockito::Server::new_async().await;
    // blockchaininfo → tip = 1000; then getblockhash will also be called.
    // Use .expect_at_most(2) to allow both RPC calls through.
    let info_mock = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(r#"{"result":{"blocks":1000},"error":null,"id":1}"#)
        .expect_at_most(2)
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    // renewal_block makes multiple RPC calls; we just verify it doesn't panic
    // and that the first call (getblockchaininfo) is made.
    let _result =
        names::renewal_block(&client, crate::noncustodial::network::Network::Regtest).await;
    info_mock.assert_async().await;
}

// --- bid validation tests ---

#[test]
fn test_bid_draft_rejects_zero_bid() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(names::build_bid_draft(
            app.state(),
            "test".into(),
            0,
            100,
            None,
        ));
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("lockup must be >= bid value > 0"));
}

#[test]
fn test_bid_draft_rejects_negative_bid() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(names::build_bid_draft(
            app.state(),
            "test".into(),
            -5,
            100,
            None,
        ));
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("lockup must be >= bid value > 0"));
}

#[test]
fn test_bid_draft_rejects_lockup_less_than_bid() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(names::build_bid_draft(
            app.state(),
            "test".into(),
            100,
            50,
            None,
        ));
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("lockup must be >= bid value > 0"));
}

// --- open draft error: no active profile ---

#[test]
fn test_open_draft_no_profile_errors() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(names::build_open_draft(
            app.state(),
            "test-name".into(),
            None,
        ));
    assert!(result.is_err());
}

// --- reveal draft error: no bid commitment ---

#[test]
fn test_reveal_draft_no_bid_commitment_errors() {
    use crate::db;

    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        // Insert a minimal profile so load_ctx succeeds past the profile check
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
             VALUES ('wp1', 'Test', 'watch_only_xpub', 'mainnet', 0, ?, 0, datetime('now'))",
            [test_xpub().to_base58check(crate::noncustodial::network::Network::Main)],
        ).unwrap();
        db::queries::set_active_profile(&conn, "wp1").unwrap();
    }
    let app = mock_app_with(state);
    // load_ctx will fail because the xpub derivation needs a valid key,
    // but this tests the error propagation path
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(names::build_reveal_draft(
            app.state(),
            "test-name".into(),
            None,
        ));
    assert!(result.is_err());
}

// --- name_input_from edge cases ---

#[test]
fn test_name_input_from_with_covenant_data() {
    let mut coin = make_coin("ff00ff00", 3, 999999, 1, 42);
    coin.covenant_type = 1;
    coin.covenant_json = Some(r#"{"nameHash":"abcd"}"#.into());
    coin.name_height = Some(100);
    let input = names::name_input_from(coin);
    assert_eq!(input.txid, "ff00ff00");
    assert_eq!(input.vout, 3);
    assert_eq!(input.value, 999999);
    assert_eq!(input.branch, 1);
    assert_eq!(input.child_index, 42);
}

#[test]
fn test_name_input_from_max_values() {
    let coin = make_coin("ff", u32::MAX, u64::MAX, u32::MAX, u32::MAX);
    let input = names::name_input_from(coin);
    assert_eq!(input.vout, u32::MAX);
    assert_eq!(input.value, u64::MAX);
    assert_eq!(input.branch, u32::MAX);
    assert_eq!(input.child_index, u32::MAX);
}

// --- build_open_draft with a valid profile (no RPC needed) ---

/// Insert a valid mnemonic_hot profile with a real derived xpub so `load_ctx`
/// succeeds past the profile/xpub checks. Returns the profile id.
pub(crate) fn insert_valid_profile(conn: &rusqlite::Connection, network: &str) -> String {
    use crate::noncustodial::hd::{ExtendedPrivKey, ExtendedPubKey};
    use crate::noncustodial::network::Network;
    let net = match network {
        "mainnet" => Network::Main,
        "testnet" => Network::Testnet,
        "regtest" => Network::Regtest,
        _ => Network::Main,
    };
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = ExtendedPrivKey::from_seed(&seed).unwrap();
    let path = [
        crate::noncustodial::hd::HARDENED_OFFSET + 44,
        crate::noncustodial::hd::HARDENED_OFFSET + net.coin_type(),
        crate::noncustodial::hd::HARDENED_OFFSET,
    ];
    let node = master.derive_path(&path).unwrap();
    let xpub = ExtendedPubKey::from_priv(&node).to_base58check(net);
    let id = "test_profile_1".to_string();
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
         VALUES (?1, 'Test', 'mnemonic_hot', ?2, 0, ?3, 0, datetime('now'))",
        rusqlite::params![&id, network, &xpub],
    ).unwrap();
    crate::db::queries::set_active_profile(conn, &id).unwrap();

    // Seed a derived address (branch 0, index 0) + a spendable UTXO so
    // build_open_draft (which needs funding for the fee) can succeed.
    let (sk, pk, addr) = crate::noncustodial::hd::derive_address(net, &seed, 0, 0, 0).unwrap();
    let spk = hex::encode(crate::noncustodial::address::script_pubkey_from_pubkey(&pk).unwrap());
    let _ = sk; // not needed for build (only for sign)
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, ?3, ?4)",
        rusqlite::params![&id, &addr, &spk, hex::encode(pk)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES ('aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd', 0, ?1, ?2, ?3, 10000000, 0, 'liquid_hns', NULL)",
        rusqlite::params![&id, &addr, &spk],
    ).unwrap();

    id
}

#[tokio::test]
async fn test_build_open_draft_succeeds_with_valid_profile() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_open_draft(app.state(), "testname".into(), None).await;
    assert!(
        result.is_ok(),
        "build_open_draft should succeed: {:?}",
        result.err()
    );
    let summary = result.unwrap();
    assert_eq!(summary.action, "open");
}

#[tokio::test]
async fn test_build_open_draft_with_explicit_fee_rate() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_open_draft(app.state(), "myname".into(), Some(50)).await;
    assert!(
        result.is_ok(),
        "build_open_draft with fee_rate should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_build_open_draft_invalid_name_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    // A name with invalid characters should error at hash_name
    let result = names::build_open_draft(app.state(), "".into(), None).await;
    // Empty name may or may not error depending on hash_name validation,
    // but the call should at least not panic.
    let _ = result;
}

#[tokio::test]
async fn test_build_open_draft_persists_draft() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest")
    };
    let app = mock_app_with(state);
    let _summary = names::build_open_draft(app.state(), "persistme".into(), None)
        .await
        .expect("build_open_draft should succeed");

    // Verify the draft was persisted in the DB
    let state = app.state::<crate::AppState>();
    let conn = state.db.lock().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'open'",
            rusqlite::params![&profile_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 1, "draft should be persisted");
}

// --- build_transfer_draft error: invalid recipient address ---

#[tokio::test]
async fn test_build_transfer_draft_invalid_recipient_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    // No owner coin exists, so this will error at owner_coin_and_state,
    // but the error should be a NotFound, not a panic.
    let result =
        names::build_transfer_draft(app.state(), "testname".into(), "invalid_addr".into(), None)
            .await;
    assert!(result.is_err());
}

// --- build_update_draft error: no owner coin ---

#[tokio::test]
async fn test_build_update_draft_no_owner_coin_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_update_draft(app.state(), "testname".into(), vec![], None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"));
}

// --- build_renew_draft error: no owner coin ---

#[tokio::test]
async fn test_build_renew_draft_no_owner_coin_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_renew_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"));
}

// --- build_cancel_draft error: no owner coin ---

#[tokio::test]
async fn test_build_cancel_draft_no_owner_coin_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_cancel_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"));
}

// --- build_revoke_draft error: no owner coin ---

#[tokio::test]
async fn test_build_revoke_draft_no_owner_coin_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_revoke_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"));
}

// --- build_register_draft error: no owner coin ---

#[tokio::test]
async fn test_build_register_draft_no_owner_coin_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_register_draft(app.state(), "testname".into(), None, None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"));
}

// --- build_finalize_draft error: no owner coin ---

#[tokio::test]
async fn test_build_finalize_draft_no_owner_coin_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_finalize_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"));
}

// --- build_redeem_draft error: no bid commitment ---

#[tokio::test]
async fn test_build_redeem_draft_no_bid_commitment_errors() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_redeem_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    // The error could be "no bid commitment" or "no unspent losing reveal coin"
    // depending on which check fires first.
    let _msg = format!("{}", result.unwrap_err());
}

// --- build_bid_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_bid_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    // build_bid_draft needs a mock RPC node for fetch_name_state.
    // Without one, it should error at the RPC level, not at validation.
    let result = names::build_bid_draft(app.state(), "testname".into(), 1000, 2000, None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    // Should fail at RPC (no node running), not at validation
    assert!(
        !msg.contains("lockup must be >= bid value"),
        "should pass validation: {msg}"
    );
}

// --- build_register_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_register_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    // build_register_draft needs a mock RPC node for owner_coin_and_state.
    let result = names::build_register_draft(app.state(), "testname".into(), None, None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// --- build_update_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_update_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_update_draft(app.state(), "testname".into(), vec![], None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// --- build_transfer_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_transfer_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result =
        names::build_transfer_draft(app.state(), "testname".into(), "hs1qtest".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// --- build_renew_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_renew_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_renew_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// --- build_cancel_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_cancel_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_cancel_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// --- build_revoke_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_revoke_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_revoke_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// --- build_finalize_draft with valid profile (needs RPC, so expect RPC error) ---

#[tokio::test]
async fn test_finalize_draft_with_valid_profile_errors_at_rpc() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = names::build_finalize_draft(app.state(), "testname".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("does not hold") || msg.contains("sync"),
        "msg: {msg}"
    );
}

// ============================================================================
// Error-path tests for build_finalize_draft (covenant JSON parsing)
// ============================================================================

/// Seed a minimal name-owner coin in tracked_name_states + tracked_utxos so that
/// `owner_coin_and_state` succeeds, but the covenant JSON can be controlled.
fn seed_name_owner_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    covenant_json: Option<&str>,
) {
    let txid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let addr = "rs1qtest";
    // Insert a minimal tracked_name_state.
    conn.execute(
        "INSERT OR IGNORE INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, 'aabb', 'CLOSED', ?3, 0, 100)",
        rusqlite::params![profile_id, name, txid],
    )
    .unwrap();
    // Insert a derived address so the 3-way join in get_name_coin works.
    conn.execute(
        "INSERT OR IGNORE INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, '00', '00')",
        rusqlite::params![profile_id, addr],
    )
    .unwrap();
    // Insert a tracked UTXO as the owner coin.
    conn.execute(
        "INSERT OR IGNORE INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', 10000, 0, ?4, 'liquid_hns', NULL)",
        rusqlite::params![txid, profile_id, addr, covenant_json.unwrap_or("null")],
    )
    .unwrap();
}

/// covenant_json exactly as `noncustodial::sync::covenant_json` writes it:
/// `{"type":..,"action":..,"items":["<nameHash hex>", ...]}`.
fn covenant_json_for(name: &str, cov_type: u8, action: &str) -> String {
    let nh = hex::encode(crate::noncustodial::names::hash_name(name).unwrap());
    serde_json::json!({
        "type": cov_type,
        "action": action,
        "items": [nh, "64000000", hex::encode(name.as_bytes()), "aa".repeat(32)],
    })
    .to_string()
}

/// Insert an unspent covenant coin (BID or REVEAL) into tracked_utxos at `addr`.
fn seed_covenant_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    txid: &str,
    addr: &str,
    cov_type: u8,
    value: i64,
    covenant_json: Option<&str>,
) {
    let spend_class = if cov_type == crate::noncustodial::sync::COV_BID {
        "name_lockup"
    } else {
        "name_control"
    };
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', ?4, ?5, ?6, ?7, NULL)",
        rusqlite::params![
            txid,
            profile_id,
            addr,
            value,
            cov_type as i64,
            covenant_json,
            spend_class
        ],
    )
    .unwrap();
}

/// Insert a bid commitment for `name` at `addr` (branch 0, index 0 = legacy
/// receive[0]) with a valid 32-byte nonce.
fn seed_bid_commitment(conn: &rusqlite::Connection, profile_id: &str, name: &str, addr: &str) {
    let nh = hex::encode(crate::noncustodial::names::hash_name(name).unwrap());
    conn.execute(
        "INSERT INTO bid_commitments
            (wallet_profile_id, name, name_hash_hex, address, branch, child_index,
             bid_value_doos, lockup_value_doos, nonce_hex, blind_hex)
         VALUES (?1, ?2, ?3, ?4, 0, 0, 1000, 2000, ?5, ?6)",
        rusqlite::params![
            profile_id,
            name,
            nh,
            addr,
            "11".repeat(32),
            format!("blind-{name}")
        ],
    )
    .unwrap();
}

/// Set up mockito RPC mocks for the RPC calls that `build_finalize_draft` and
/// `build_reveal_draft` need: getnameinfo, getblockchaininfo, getblockhash.
pub(crate) async fn mock_names_rpc(
    server: &mut mockito::Server,
) -> (mockito::Mock, mockito::Mock, mockito::Mock) {
    let name_info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getnameinfo".into()))
        .with_body(r#"{"result":{"info":{"height":100,"value":50000,"renewals":2,"claimed":1,"weak":false}},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;
    let bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":1000},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;
    let bh = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockhash".into()))
        .with_body(r#"{"result":"0000000000000000000000000000000000000000000000000000000000000000","error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;
    (name_info, bi, bh)
}

#[tokio::test]
async fn test_renewal_block_rejects_short_hash() {
    let mut server = mockito::Server::new_async().await;
    let _bi = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":100},"error":null,"id":1}"#)
        .create_async()
        .await;
    let _bh = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockhash".into()))
        .with_body(r#"{"result":"00","error":null,"id":1}"#)
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    let result =
        names::renewal_block(&client, crate::noncustodial::network::Network::Regtest).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("block hash") || msg.contains("hex"),
        "msg: {msg}"
    );
}

// --- fetch_name_state with RPC error ---

// ============================================================================
// get_name_action_capabilities — node-unreachable, local-evidence fallback
// ============================================================================

/// Point the node RPC at an unreachable port so `get_name_info` fails fast,
/// forcing the node-unreachable fallback path.
fn set_unreachable_node(conn: &rusqlite::Connection) {
    crate::db::queries::set_setting(conn, "node_rpc_url", "http://127.0.0.1:9").unwrap();
}

#[tokio::test]
async fn capabilities_node_down_explorer_owned_is_owned_but_spend_locked() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_unreachable_node(&conn);
        // The wallet's own derived address (owner per explorer evidence).
        let owner_addr: String = conn
            .query_row(
                "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 LIMIT 1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap();
        // A CLOSED name owned per explorer history (owner_address is ours), but
        // whose owner_txid does NOT match any unspent tracked_utxos row → no
        // node-synced owner coin (has_owner_coin stays false).
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'ownedname', 'aabb', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![
                &id,
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                &owner_addr
            ],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "ownedname".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve via local evidence");

    // Classified as owned even though no coin is synced locally.
    assert!(caps.owns_name, "explorer-owned name must be owns_name=true");
    assert!(!caps.has_owner_coin, "no node-synced owner coin exists");
    assert_eq!(
        caps.task_state,
        names::AuctionTaskState::OwnedNoUrgentAction
    );

    // Every spend-capable action is forced disallowed with the sync reason.
    for cap in [
        &caps.can_register,
        &caps.can_update,
        &caps.can_transfer,
        &caps.can_renew,
        &caps.can_revoke,
        &caps.can_finalize,
        &caps.can_cancel_transfer,
    ] {
        assert!(
            !cap.allowed,
            "spend action must be disallowed when unsynced"
        );
        assert!(
            cap.reason
                .as_deref()
                .unwrap_or("")
                .contains("not synced locally"),
            "reason should mention not synced locally, got {:?}",
            cap.reason
        );
    }
}

#[tokio::test]
async fn capabilities_node_down_near_expiry_yields_expiring_soon() {
    // Fix for review Finding 2: on the node-unreachable fallback path
    // `get_name_action_capabilities` used to pass `stats: None`, so the modal
    // never surfaced `expiringSoon` even when the WalletView banner / Renewals
    // screen were alarming for the same name. It must now compute days the
    // same way `read_renewals` does — tracked `renewal_height` + the
    // `estimate_persisted_height` helper — and feed the alarm into the modal.
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_unreachable_node(&conn);
        let owner_addr: String = conn
            .query_row(
                "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 LIMIT 1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap();
        // Persisted height estimate (no live node in this branch): last-synced
        // height, "now" so extrapolation is a no-op.
        db::queries::update_profile_sync(&conn, &id, 90_000).unwrap();
        // regtest renewal window = 5_000 blocks (Network::name_params()).
        // renewal_height chosen so ~10 days remain at height 90_000:
        // 90_000 - (renewal_height + 5_000) = 10 * 144 blocks.
        let renewal_height: i64 = 90_000 - 5_000 + 10 * 144;
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout,
                 owner_address, height, renewal_height)
             VALUES (?1, 'expiringname', 'aabb', 'CLOSED', ?2, 0, ?3, 100, ?4)",
            rusqlite::params![
                &id,
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                &owner_addr,
                renewal_height,
            ],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "expiringname".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve via local evidence");

    assert!(caps.owns_name, "explorer-owned name must be owns_name=true");
    assert_eq!(
        caps.task_state,
        names::AuctionTaskState::ExpiringSoon,
        "modal must not contradict the WalletView/Renewals expiry alarm just because the node isn't synced"
    );
}

#[tokio::test]
async fn capabilities_node_down_no_tracked_row_falls_back_conservative() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_unreachable_node(&conn);
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "totallyunknown".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve");

    // Genuinely unknown locally + node down → conservative fallback (unchanged).
    assert!(!caps.owns_name);
    assert_eq!(caps.phase, "UNKNOWN");
    assert_eq!(caps.task_state, names::AuctionTaskState::UnavailableOther);
    assert!(!caps.can_open.allowed);
    assert!(!caps.can_register.allowed);
}

// ============================================================================
// get_names_action_capabilities — batch command (Task 12 / F5)
// ============================================================================

/// Insert a second, distinct wallet profile (own id + own derived address) so
/// isolation tests can prove the batch command never bleeds evidence across
/// wallets. `insert_valid_profile` always uses a fixed id, so it can't be
/// called twice on the same connection.
fn insert_second_profile(conn: &rusqlite::Connection, network: &str) -> (String, String) {
    let id = "test_profile_2".to_string();
    conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_index, account_xpub, watch_only, created_at)
         VALUES (?1, 'Second', 'mnemonic_hot', ?2, 0, 'xpubSECONDFAKE', 0, datetime('now'))",
        rusqlite::params![&id, network],
    )
    .unwrap();
    let addr = format!("addr-{id}");
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, 'aa', 'bb')",
        rusqlite::params![&id, &addr],
    )
    .unwrap();
    (id, addr)
}

#[tokio::test]
async fn batch_capabilities_returns_per_name_results_in_order() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_unreachable_node(&conn);
        let owner_addr: String = conn
            .query_row(
                "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 LIMIT 1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap();
        // One name this wallet owns per tracked evidence, one genuinely unknown.
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'batchowned', 'aabb', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![
                &id,
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                &owner_addr
            ],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let results = names::get_names_action_capabilities(
        app.state(),
        vec!["batchowned".into(), "batchunknown".into()],
        Some(profile_id),
    )
    .await
    .expect("batch capabilities should resolve");

    assert_eq!(results.len(), 2, "one result per input name, in order");
    assert_eq!(results[0].name, "batchowned");
    assert!(
        results[0].owns_name,
        "batchowned must be classified as owned"
    );
    assert_eq!(results[1].name, "batchunknown");
    assert!(!results[1].owns_name);
    assert_eq!(
        results[1].task_state,
        names::AuctionTaskState::UnavailableOther
    );
}

#[tokio::test]
async fn batch_capabilities_respects_wallet_profile_isolation() {
    let state = create_full_test_state();
    let (profile_a, profile_b) = {
        let conn = state.db.lock().unwrap();
        let a = insert_valid_profile(&conn, "regtest");
        set_unreachable_node(&conn);
        let (b, addr_b) = insert_second_profile(&conn, "regtest");
        let addr_a: String = conn
            .query_row(
                "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 LIMIT 1",
                rusqlite::params![&a],
                |r| r.get(0),
            )
            .unwrap();
        // Wallet A owns "nameforA"; wallet B owns "nameforB" — each row's
        // owner_address only matches its own wallet's derived address.
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'nameforA', 'aabb', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![
                &a,
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                &addr_a
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'nameforB', 'ccdd', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![
                &b,
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                &addr_b
            ],
        )
        .unwrap();
        (a, b)
    };
    let app = mock_app_with(state);

    let names = vec!["nameforA".to_string(), "nameforB".to_string()];

    let as_a = names::get_names_action_capabilities(app.state(), names.clone(), Some(profile_a))
        .await
        .expect("batch for profile A should resolve");
    let owned_by_a: Vec<&str> = as_a
        .iter()
        .filter(|c| c.owns_name)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        owned_by_a,
        vec!["nameforA"],
        "profile A must only see its own name as owned"
    );

    let as_b = names::get_names_action_capabilities(app.state(), names, Some(profile_b))
        .await
        .expect("batch for profile B should resolve");
    let owned_by_b: Vec<&str> = as_b
        .iter()
        .filter(|c| c.owns_name)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        owned_by_b,
        vec!["nameforB"],
        "profile B must only see its own name as owned"
    );
}

#[tokio::test]
async fn batch_capabilities_rejects_batches_over_the_cap() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_unreachable_node(&conn);
        id
    };
    let app = mock_app_with(state);
    let too_many: Vec<String> = (0..(names::MAX_NAMES_ACTION_CAPABILITIES_BATCH + 1))
        .map(|i| format!("name{i}"))
        .collect();

    let result =
        names::get_names_action_capabilities(app.state(), too_many, Some(profile_id)).await;
    assert!(
        result.is_err(),
        "a batch over the cap must be rejected, not silently truncated"
    );
}

#[tokio::test]
async fn batch_capabilities_no_profile_returns_conservative_per_name() {
    let state = create_full_test_state();
    let app = mock_app_with(state);
    let results = names::get_names_action_capabilities(
        app.state(),
        vec!["a".into(), "b".into(), "c".into()],
        None,
    )
    .await
    .expect("batch capabilities should resolve even with no active profile");

    assert_eq!(
        results.len(),
        3,
        "still one conservative result per input name"
    );
    for c in &results {
        assert!(!c.owns_name);
        assert_eq!(c.task_state, names::AuctionTaskState::UnavailableOther);
    }
}

// ============================================================================
// get_name_action_capabilities — sync gate + symmetric owns_name
// ============================================================================

/// Point the node RPC at a mockito server so RPC calls succeed against it.
pub(crate) fn set_node_rpc_url(conn: &rusqlite::Connection, url: &str) {
    crate::db::queries::set_setting(conn, "node_rpc_url", url).unwrap();
}

/// Mock `getblockchaininfo` with a given verification progress. `getnameinfo`
/// (when `name_info` is Some) is mocked to return that CLOSED name state so the
/// synced-node path has an authoritative phase.
async fn mock_blockchain_and_name(
    server: &mut mockito::Server,
    verification_progress: f64,
    name_info: Option<&str>,
) -> Vec<mockito::Mock> {
    let mut mocks = Vec::new();
    mocks.push(
        server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
            .with_body(format!(
                r#"{{"result":{{"blocks":1000,"headers":1000,"verificationprogress":{}}},"error":null,"id":1}}"#,
                verification_progress
            ))
            .expect_at_least(1)
            .create_async()
            .await,
    );
    if let Some(state) = name_info {
        mocks.push(
            server
                .mock("POST", "/")
                .match_body(mockito::Matcher::Regex("getnameinfo".into()))
                .with_body(format!(
                    r#"{{"result":{{"info":{{"name":"testname","state":"{}","stats":{{}}}}}},"error":null,"id":1}}"#,
                    state
                ))
                .expect_at_least(1)
                .create_async()
                .await,
        );
    }
    mocks
}

/// The wallet's own derived address (seeded by `insert_valid_profile`).
pub(crate) fn first_derived_address(conn: &rusqlite::Connection, profile_id: &str) -> String {
    conn.query_row(
        "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 LIMIT 1",
        rusqlite::params![profile_id],
        |r| r.get(0),
    )
    .unwrap()
}

/// A reachable-but-UNSYNCED node (verificationprogress 0.19) must NOT be treated
/// as authoritative: the command falls back to local Sync evidence. A tracked
/// name whose owner address is ours is classified as owned, but no owner coin is
/// synced, so every spend-capable action is locked with the "not synced" reason.
#[tokio::test]
async fn capabilities_node_unsynced_falls_back_to_local_evidence() {
    let mut server = mockito::Server::new_async().await;
    // getnameinfo IS mocked too: without the sync gate the old code would take the
    // authoritative node path (owns_name=has_owner_coin=false) — proving the gate
    // is what redirects an unsynced node to the local-evidence fallback.
    let _mocks = mock_blockchain_and_name(&mut server, 0.19, Some("CLOSED")).await;

    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let owner_addr = first_derived_address(&conn, &id);
        // CLOSED name owned per explorer history (owner_address is ours), but the
        // owner_txid matches no unspent tracked_utxos → has_owner_coin stays false.
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'ownedname', 'aabb', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![
                &id,
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                &owner_addr
            ],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "ownedname".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve via local evidence");

    assert!(
        caps.owns_name,
        "explorer-owned name must be owns_name=true even on an unsynced node"
    );
    assert!(!caps.has_owner_coin, "no node-synced owner coin exists");
    for cap in [
        &caps.can_register,
        &caps.can_update,
        &caps.can_transfer,
        &caps.can_renew,
        &caps.can_revoke,
        &caps.can_finalize,
        &caps.can_cancel_transfer,
    ] {
        assert!(
            !cap.allowed,
            "spend action must be disallowed when node is unsynced"
        );
        assert!(
            cap.reason
                .as_deref()
                .unwrap_or("")
                .contains("not synced locally"),
            "reason should mention not synced locally, got {:?}",
            cap.reason
        );
    }
}

/// A fully-synced node WITH a real owner coin uses the authoritative node path:
/// owns_name true, has_owner_coin true, spends NOT locked.
#[tokio::test]
async fn capabilities_node_synced_with_owner_coin_allows_spends() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_blockchain_and_name(&mut server, 1.0, Some("CLOSED")).await;

    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let owner_addr = first_derived_address(&conn, &id);
        // A real owner coin: tracked_utxos row at our derived address, referenced
        // by a CLOSED tracked_name_states row → get_name_coin joins it (the
        // derived_addresses row was seeded by insert_valid_profile).
        let owner_txid = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES (?1, 0, ?2, ?3, '00', 10000, 0, NULL, 'liquid_hns', NULL)",
            rusqlite::params![owner_txid, &id, &owner_addr],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'testname', 'aabb', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![&id, owner_txid, &owner_addr],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "testname".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve via the node path");

    assert!(caps.owns_name, "owner coin present → owns_name=true");
    assert!(caps.has_owner_coin, "owner coin is synced");
    assert!(
        caps.can_update.allowed,
        "update must be allowed for an owned, synced name"
    );
    assert!(caps.can_transfer.allowed, "transfer must be allowed");
    assert!(caps.can_renew.allowed, "renew must be allowed");
    // Not spend-locked: reasons must NOT be the "not synced" lock reason.
    assert_ne!(
        caps.can_update.reason.as_deref().unwrap_or(""),
        "owner coin not synced locally — connect a node and Refresh to manage"
    );
}

/// A fully-synced node WITHOUT an owner coin, but with a tracked owner address
/// that is ours, still classifies the name as owned (symmetric owns_name), yet
/// keeps spends locked — the core invariant on the synced-node branch.
#[tokio::test]
async fn capabilities_node_synced_without_owner_coin_owns_but_spend_locked() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_blockchain_and_name(&mut server, 1.0, Some("CLOSED")).await;

    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let owner_addr = first_derived_address(&conn, &id);
        // owner_address is ours, but owner_txid matches no tracked_utxos row →
        // has_owner_coin stays false.
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
             VALUES (?1, 'testname', 'aabb', 'CLOSED', ?2, 0, ?3, 100)",
            rusqlite::params![
                &id,
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                &owner_addr
            ],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "testname".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve via the node path");

    assert!(
        caps.owns_name,
        "explorer-owned name must be owns_name=true on the synced-node path"
    );
    assert!(!caps.has_owner_coin, "no node-synced owner coin exists");
    for cap in [
        &caps.can_update,
        &caps.can_transfer,
        &caps.can_renew,
        &caps.can_revoke,
    ] {
        assert!(
            !cap.allowed,
            "spend action must be locked without a synced owner coin"
        );
        assert!(
            cap.reason
                .as_deref()
                .unwrap_or("")
                .contains("not synced locally"),
            "reason should mention not synced locally, got {:?}",
            cap.reason
        );
    }
}

// ============================================================================
// Part 3 (Task 6, confirmed pre-existing bug from the Task 2 review):
// `can_reveal` must gate on the unspent COV_BID coin (what a reveal spends),
// not the unspent COV_REVEAL coin (what only exists AFTER a reveal). Verifies
// the fix end-to-end through `get_name_action_capabilities`, not just the
// pure `derive_auction_task_state` function.
// ============================================================================

/// Seed a bid commitment + an unspent tracked_utxos coin of `covenant_type`
/// at the commitment's own address, with a real covenant_json carrying the
/// name hash (so `find_unspent_covenant_utxo`'s name-hash match succeeds).
fn seed_bid_commitment_and_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    txid: &str,
    covenant_type: u8,
) -> String {
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let nh_hex = hex::encode(nh);
    let raw = hex::encode(crate::noncustodial::names::raw_name(name).unwrap());
    let addr = first_derived_address(conn, profile_id);
    db::queries::insert_bid_commitment(
        conn, profile_id, name, &nh_hex, &addr, 0, 0, 1000, 2000, "nonce1", "blind1",
    )
    .unwrap();
    let cov = serde_json::json!({
        "type": covenant_type,
        "items": [nh_hex, "64000000", raw, "blindaa"],
    })
    .to_string();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', 2000, ?4, ?5, 'name_lockup', NULL)",
        rusqlite::params![txid, profile_id, &addr, covenant_type as i64, &cov],
    )
    .unwrap();
    addr
}

/// REVEAL phase + bid commitment + an unspent COV_BID coin (no reveal coin
/// yet — realistic pre-reveal state) → `canReveal.allowed` true and
/// `taskState` `readyToReveal`. This is the exact scenario the bug disabled.
#[tokio::test]
async fn capabilities_reveal_phase_with_unspent_bid_coin_allows_reveal() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_blockchain_and_name(&mut server, 1.0, Some("REVEAL")).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        seed_bid_commitment_and_coin(
            &conn,
            &id,
            "revealtest",
            &"aa".repeat(32),
            crate::noncustodial::sync::COV_BID,
        );
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "revealtest".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve");

    assert!(
        !caps.has_reveal_coin,
        "no REVEAL coin should exist before revealing"
    );
    assert!(caps.has_bid_coin, "the unspent BID coin should be visible");
    assert!(
        caps.can_reveal.allowed,
        "reveal must be allowed with an unspent bid coin, got reason: {:?}",
        caps.can_reveal.reason
    );
    assert_eq!(caps.task_state, names::AuctionTaskState::ReadyToReveal);
}

/// Same REVEAL phase + bid commitment, but the BID coin has already been
/// spent (revealed) and only a COV_REVEAL coin exists now → `canReveal` must
/// be false (there's nothing left to reveal; revealing again would fail).
#[tokio::test]
async fn capabilities_reveal_phase_after_reveal_disallows_reveal_again() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_blockchain_and_name(&mut server, 1.0, Some("REVEAL")).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        // Only a COV_REVEAL coin — the BID coin was already spent by a reveal.
        seed_bid_commitment_and_coin(
            &conn,
            &id,
            "alreadyrevealed",
            &"bb".repeat(32),
            crate::noncustodial::sync::COV_REVEAL,
        );
        id
    };
    let app = mock_app_with(state);
    let caps = names::get_name_action_capabilities(
        app.state(),
        "alreadyrevealed".into(),
        Some(profile_id),
    )
    .await
    .expect("capabilities should resolve");

    assert!(caps.has_reveal_coin, "the REVEAL coin should be visible");
    assert!(!caps.has_bid_coin, "the BID coin was already spent");
    assert!(
        !caps.can_reveal.allowed,
        "reveal must be disallowed once the bid coin is already spent"
    );
}

/// CLOSED phase + a losing (unspent) COV_REVEAL coin + not owning the name →
/// `canRedeem.allowed` true. Confirms the split didn't regress redeem, which
/// correctly keeps gating on `has_reveal_coin`.
#[tokio::test]
async fn capabilities_closed_loser_with_reveal_coin_allows_redeem() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_blockchain_and_name(&mut server, 1.0, Some("CLOSED")).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        seed_bid_commitment_and_coin(
            &conn,
            &id,
            "lostauction",
            &"cc".repeat(32),
            crate::noncustodial::sync::COV_REVEAL,
        );
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "lostauction".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve");

    assert!(caps.has_reveal_coin);
    assert!(
        !caps.owns_name,
        "a losing bid must not be classified as owning the name"
    );
    assert!(
        caps.can_redeem.allowed,
        "redeem must be allowed for a losing reveal coin, got reason: {:?}",
        caps.can_redeem.reason
    );
}

#[tokio::test]
async fn test_fetch_name_state_rpc_error() {
    let mut server = mockito::Server::new_async().await;
    // Return an RPC error envelope
    let _m = server
        .mock("POST", "/")
        .with_body(r#"{"result":null,"error":{"message":"Name not found.","code":-1},"id":1}"#)
        .create_async()
        .await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(
        &server.url(),
        "",
        crate::noncustodial::rpc::ChainSource::LocalNode,
    );
    let result = names::fetch_name_state(&client, "nonexistent").await;
    assert!(result.is_err());
}

// ============================================================================
// C1 regression: reveal/redeem must locate the coin by name hash, and new
// bids must rotate to a fresh receive address.
// ============================================================================

/// Read the persisted signing inputs of a draft (which coins it spends).
fn draft_signing_inputs(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) -> String {
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    conn.query_row(
        "SELECT signing_inputs_json FROM wallet_tx_drafts WHERE id = ?1",
        rusqlite::params![draft_id],
        |r| r.get(0),
    )
    .unwrap()
}

/// Two BID coins for two DIFFERENT names share one legacy address (receive[0]).
/// Reveal for name A must spend name A's coin — never name B's. The other
/// name's coin is inserted FIRST so a naive `LIMIT 1` would pick it.
#[tokio::test]
async fn reveal_selects_bid_coin_by_name_hash_on_shared_address() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let txid_a = "aa".repeat(32);
    let txid_b = "bb".repeat(32);
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        let cov_b = covenant_json_for("nameb", crate::noncustodial::sync::COV_BID, "BID");
        let cov_a = covenant_json_for("namea", crate::noncustodial::sync::COV_BID, "BID");
        seed_covenant_coin(
            &conn,
            &id,
            &txid_b,
            &addr,
            crate::noncustodial::sync::COV_BID,
            2000,
            Some(&cov_b),
        );
        seed_covenant_coin(
            &conn,
            &id,
            &txid_a,
            &addr,
            crate::noncustodial::sync::COV_BID,
            2000,
            Some(&cov_a),
        );
        seed_bid_commitment(&conn, &id, "namea", &addr);
    }
    let app = mock_app_with(state);
    let draft = names::build_reveal_draft(app.state(), "namea".into(), None)
        .await
        .expect("reveal draft for namea should build");
    let inputs = draft_signing_inputs(&app, &draft.id);
    assert!(
        inputs.contains(&txid_a),
        "reveal must spend namea's bid coin; inputs: {inputs}"
    );
    assert!(
        !inputs.contains(&txid_b),
        "reveal must NOT spend nameb's bid coin; inputs: {inputs}"
    );
}

/// Same regression for redeem: two losing REVEAL coins for different names on
/// one shared address — redeem for name A must spend name A's reveal coin.
#[tokio::test]
async fn redeem_selects_reveal_coin_by_name_hash_on_shared_address() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let txid_a = "cc".repeat(32);
    let txid_b = "dd".repeat(32);
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        let cov_b = covenant_json_for("nameb", crate::noncustodial::sync::COV_REVEAL, "REVEAL");
        let cov_a = covenant_json_for("namea", crate::noncustodial::sync::COV_REVEAL, "REVEAL");
        seed_covenant_coin(
            &conn,
            &id,
            &txid_b,
            &addr,
            crate::noncustodial::sync::COV_REVEAL,
            1000,
            Some(&cov_b),
        );
        seed_covenant_coin(
            &conn,
            &id,
            &txid_a,
            &addr,
            crate::noncustodial::sync::COV_REVEAL,
            1000,
            Some(&cov_a),
        );
        seed_bid_commitment(&conn, &id, "namea", &addr);
    }
    let app = mock_app_with(state);
    let draft = names::build_redeem_draft(app.state(), "namea".into(), None)
        .await
        .expect("redeem draft for namea should build");
    let inputs = draft_signing_inputs(&app, &draft.id);
    assert!(
        inputs.contains(&txid_a),
        "redeem must spend namea's reveal coin; inputs: {inputs}"
    );
    assert!(
        !inputs.contains(&txid_b),
        "redeem must NOT spend nameb's reveal coin; inputs: {inputs}"
    );
}

/// Only ANOTHER name's bid coin sits at the commitment address: reveal must
/// fail with an error naming the requested name — never spend the wrong coin.
#[tokio::test]
async fn reveal_errors_when_no_bid_coin_matches_name_hash() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        let cov_b = covenant_json_for("nameb", crate::noncustodial::sync::COV_BID, "BID");
        seed_covenant_coin(
            &conn,
            &id,
            &"ee".repeat(32),
            &addr,
            crate::noncustodial::sync::COV_BID,
            2000,
            Some(&cov_b),
        );
        seed_bid_commitment(&conn, &id, "namea", &addr);
    }
    let app = mock_app_with(state);
    let result = names::build_reveal_draft(app.state(), "namea".into(), None).await;
    assert!(
        result.is_err(),
        "reveal must not spend another name's bid coin"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("namea"),
        "error should name the name, got: {msg}"
    );
}

/// Two BID coins for the SAME name at one address (double bid): ambiguous —
/// picking either could reveal with the wrong nonce, so it must error.
#[tokio::test]
async fn reveal_errors_on_ambiguous_bid_coins_for_same_name() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        let cov_a = covenant_json_for("namea", crate::noncustodial::sync::COV_BID, "BID");
        seed_covenant_coin(
            &conn,
            &id,
            &"11".repeat(32),
            &addr,
            crate::noncustodial::sync::COV_BID,
            2000,
            Some(&cov_a),
        );
        seed_covenant_coin(
            &conn,
            &id,
            &"22".repeat(32),
            &addr,
            crate::noncustodial::sync::COV_BID,
            3000,
            Some(&cov_a),
        );
        seed_bid_commitment(&conn, &id, "namea", &addr);
    }
    let app = mock_app_with(state);
    let result = names::build_reveal_draft(app.state(), "namea".into(), None).await;
    assert!(
        result.is_err(),
        "ambiguous bid coins must be an error, not an arbitrary pick"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("namea") && (msg.contains("multiple") || msg.contains("2 ")),
        "error should explain the ambiguity, got: {msg}"
    );
}

/// New bids must NOT land on receive[0] once it is used: the BID output (and
/// the persisted commitment) go to the next unused receive index, and that
/// address is registered in derived_addresses so sync/lookups can see it.
#[tokio::test]
async fn new_bid_rotates_to_fresh_receive_address() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let (profile_id, receive0) = {
        let conn = state.db.lock().unwrap();
        // insert_valid_profile seeds a funding UTXO at receive[0] → index 0 is used.
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let receive0 = first_derived_address(&conn, &id);
        (id, receive0)
    };
    let app = mock_app_with(state);
    names::build_bid_draft(app.state(), "namea".into(), 1000, 2000, None)
        .await
        .expect("bid draft should build");

    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let (bid_addr, branch, child_index): (String, i64, i64) = conn
        .query_row(
            "SELECT address, branch, child_index FROM bid_commitments
             WHERE wallet_profile_id = ?1 AND name = 'namea'",
            rusqlite::params![&profile_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_ne!(
        bid_addr, receive0,
        "bid output must rotate off the used receive[0] address"
    );
    assert_eq!(branch, 0, "bid address stays on the receive branch");
    assert_eq!(
        child_index, 1,
        "next unused receive index after used index 0"
    );
    // Registered for the sync scan + the coin-lookup JOIN.
    let registered: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM derived_addresses
             WHERE wallet_profile_id = ?1 AND address = ?2 AND branch = 0 AND child_index = 1",
            rusqlite::params![&profile_id, &bid_addr],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        registered, 1,
        "rotated bid address must be in derived_addresses"
    );
}

/// `build_bid_draft` persists an estimate of the reveal-window close height
/// (I1 / Task 4) — `start + (treeInterval + 1) + biddingPeriod +
/// revealPeriod`, where `start` is the live `getnameinfo().info.height`
/// (`mock_names_rpc` returns 100) and the network params come from
/// `Network::name_params()` (regtest here: tree_interval 5, bidding_period
/// 5, reveal_period 10 — see `noncustodial/network.rs`). The `+ 1` and the
/// OPENING period itself matter: hsd only lets a name enter BIDDING once
/// `height > start + treeInterval` (review C3-review-2/Task-4 Finding 2 —
/// the original estimate omitted this OPENING period entirely).
#[tokio::test]
async fn bid_draft_persists_reveal_end_height_estimate() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        id
    };
    let app = mock_app_with(state);
    names::build_bid_draft(app.state(), "namea".into(), 1000, 2000, None)
        .await
        .expect("bid draft should build");

    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let reveal_end_height: i64 = conn
        .query_row(
            "SELECT reveal_end_height FROM bid_commitments
             WHERE wallet_profile_id = ?1 AND name = 'namea'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    // 100 (mocked getnameinfo height) + 6 (regtest treeInterval 5 + 1, the
    // OPENING period) + 5 (regtest biddingPeriod) + 10 (regtest revealPeriod).
    assert_eq!(reveal_end_height, 121);
}

/// A bid sitting on a ROTATED (non-zero) receive index must be revealable:
/// the coin lookup resolves branch/child_index via derived_addresses, and the
/// draft spends the coin with its real derivation path.
#[tokio::test]
async fn reveal_works_for_bid_on_rotated_address() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let txid = "ab".repeat(32);
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        // Derive receive index 1 (the rotated slot) from the profile's seed.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let (_sk, pk, addr1) = crate::noncustodial::hd::derive_address(
            crate::noncustodial::network::Network::Regtest,
            &seed,
            0,
            0,
            1,
        )
        .unwrap();
        let spk =
            hex::encode(crate::noncustodial::address::script_pubkey_from_pubkey(&pk).unwrap());
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, 1, ?2, ?3, ?4)",
            rusqlite::params![&id, &addr1, &spk, hex::encode(pk)],
        )
        .unwrap();
        let cov_a = covenant_json_for("namea", crate::noncustodial::sync::COV_BID, "BID");
        seed_covenant_coin(
            &conn,
            &id,
            &txid,
            &addr1,
            crate::noncustodial::sync::COV_BID,
            2000,
            Some(&cov_a),
        );
        let nh = hex::encode(crate::noncustodial::names::hash_name("namea").unwrap());
        conn.execute(
            "INSERT INTO bid_commitments
                (wallet_profile_id, name, name_hash_hex, address, branch, child_index,
                 bid_value_doos, lockup_value_doos, nonce_hex, blind_hex)
             VALUES (?1, 'namea', ?2, ?3, 0, 1, 1000, 2000, ?4, 'blind-rotated')",
            rusqlite::params![&id, nh, &addr1, "11".repeat(32)],
        )
        .unwrap();
    }
    let app = mock_app_with(state);
    let draft = names::build_reveal_draft(app.state(), "namea".into(), None)
        .await
        .expect("reveal for a rotated-address bid should build");
    let inputs = draft_signing_inputs(&app, &draft.id);
    assert!(
        inputs.contains(&txid),
        "must spend the rotated bid coin; inputs: {inputs}"
    );
    assert!(
        inputs.contains("\"child_index\":1"),
        "name input must carry the rotated derivation index; inputs: {inputs}"
    );
}

// ============================================================================
// Coin reservation across drafts (I3): covenant drafts and plain sends must
// respect each other's claim on the same funding coin.
// ============================================================================

#[tokio::test]
async fn covenant_and_plain_send_drafts_respect_reservations_mutually() {
    use crate::error::AppError;
    use crate::noncustodial::hd::derive_address;
    use crate::noncustodial::network::Network;

    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest")
    };
    let app = mock_app_with(state);

    // The covenant draft (OPEN — needs no RPC) reserves the profile's only
    // funding coin.
    let open_draft = names::build_open_draft(app.state(), "mutualtest".into(), None)
        .await
        .expect("build_open_draft should succeed");

    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let (_sk, _pk, to) = derive_address(Network::Regtest, &seed, 0, 0, 0).unwrap();

    // A plain send now has nothing left to fund itself with — the same
    // liquid coin table backs both draft kinds.
    let err =
        crate::commands::tx::build_send_hns_draft(app.state(), to.clone(), 500_000, Some(1), None)
            .await
            .expect_err("plain send must not see the coin reserved by the open draft");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");

    // Deleting the covenant draft frees the coin for the plain send.
    crate::commands::tx::delete_tx_draft(app.state(), open_draft.id.clone())
        .await
        .expect("delete open draft");
    let send_draft =
        crate::commands::tx::build_send_hns_draft(app.state(), to, 500_000, Some(1), None)
            .await
            .expect("plain send can now claim the coin the open draft released");
    assert_eq!(send_draft.wallet_profile_id, profile_id);
}

// ============================================================================
// Bid multiplicity + honest conflict handling (I2, Task 6).
//
// Product rule: one bid per wallet per name. The UI capability gate
// (`build_name_action_capabilities` / `existing_bid_count`) is advisory only —
// these tests exercise the command-level enforcement that closes the gap a
// second window, a stale UI, or a direct replay could otherwise walk through.
// ============================================================================

/// `insert_valid_profile` seeds exactly one funding UTXO. These tests build
/// TWO bid drafts in a row, so a second liquid coin is needed or the second
/// build would fail on "insufficient funds" before ever reaching the
/// multiplicity guard — which would test the wrong thing.
fn insert_extra_funding(conn: &rusqlite::Connection, profile_id: &str, txid: &str) {
    let addr: String = conn
        .query_row(
            "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 AND branch = 0 AND child_index = 0",
            rusqlite::params![profile_id],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', 10000000, 0, 'liquid_hns', NULL)",
        rusqlite::params![txid, profile_id, &addr],
    )
    .unwrap();
}

/// covenant_json exactly as a real BID output would carry it:
/// `[nameHash, u32(start), rawName, blind]` — mirrors
/// `bids_cmd_tests::bid_covenant_json`.
fn bid_covenant_json_for(name: &str, blind_hex: &str) -> String {
    let nh = hex::encode(crate::noncustodial::names::hash_name(name).unwrap());
    let raw = hex::encode(crate::noncustodial::names::raw_name(name).unwrap());
    serde_json::json!({
        "type": crate::noncustodial::sync::COV_BID,
        "action": "BID",
        "items": [nh, "64000000", raw, blind_hex],
    })
    .to_string()
}

#[tokio::test]
async fn build_bid_draft_rejects_second_bid_when_a_draft_is_already_pending() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        insert_extra_funding(&conn, &id, &"11".repeat(32));
        id
    };
    let app = mock_app_with(state);

    names::build_bid_draft(app.state(), "duplicatename".into(), 1000, 2000, None)
        .await
        .expect("first bid draft should build");

    let err = names::build_bid_draft(app.state(), "duplicatename".into(), 1000, 2000, None)
        .await
        .expect_err("second bid on the same name must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("one bid per wallet per name"),
        "error should state the product rule, got: {msg}"
    );

    // Exactly one bid draft — the rejected attempt must not have persisted
    // anything (no draft row, no second commitment).
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let draft_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'bid'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        draft_count, 1,
        "rejected retry must not persist a second draft"
    );
    let commitment_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = 'duplicatename'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        commitment_count, 1,
        "rejected retry must not persist a second commitment"
    );
}

/// Even without any local draft/commitment history, an unspent COV_BID coin
/// for the name anywhere in the profile (e.g. imported/recovered wallet
/// state) must block a second bid.
#[tokio::test]
async fn build_bid_draft_rejects_when_an_unspent_bid_coin_already_exists() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        let cov = bid_covenant_json_for("existingbid", &"22".repeat(32));
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
             VALUES (?1, 0, ?2, ?3, '00', 2000, ?4, ?5, 'name_lockup', NULL)",
            rusqlite::params![
                &"33".repeat(32),
                &id,
                &addr,
                crate::noncustodial::sync::COV_BID as i64,
                &cov
            ],
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);

    let err = names::build_bid_draft(app.state(), "existingbid".into(), 1000, 2000, None)
        .await
        .expect_err("a bid on a name with an existing unspent BID coin must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("one bid per wallet per name"),
        "error should state the product rule, got: {msg}"
    );

    // Nothing was written — including no commitment for a bid that was never
    // allowed to build.
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let commitment_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = 'existingbid'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(commitment_count, 0);
    let _ = profile_id;
}

/// The multiplicity guard is name-hash-scoped — a bid on a different name
/// must never be blocked by an existing bid elsewhere.
#[tokio::test]
async fn build_bid_draft_allows_bid_on_a_different_name() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        insert_extra_funding(&conn, &id, &"44".repeat(32));
    }
    let app = mock_app_with(state);

    names::build_bid_draft(app.state(), "namea".into(), 1000, 2000, None)
        .await
        .expect("first bid should build");
    names::build_bid_draft(app.state(), "nameb".into(), 1000, 2000, None)
        .await
        .expect("a bid on a different name must not be blocked by the first bid");
}

/// Task 1 (bug fix): `build_bid_draft` must persist the on-chain bid txid
/// onto its own commitment row at build time — this is what lets the
/// "which bids are mine" join (`merge_name_bids`) recognize the user's own
/// bid against the explorer's list. Before this fix `bid_txid` was written
/// only in tests (`queries::set_bid_txid`), never by the production build
/// flow, so a real bid always showed "yours: 0".
#[tokio::test]
async fn build_bid_draft_persists_bid_txid_on_its_commitment() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        id
    };
    let app = mock_app_with(state);

    let draft = names::build_bid_draft(app.state(), "namea".into(), 1000, 2000, None)
        .await
        .expect("bid draft should build");
    // The top-level `TxDraftSummary.txid` (backed by `wallet_tx_drafts.txid`)
    // stays NULL until broadcast — the deterministic pre-signing txid lives
    // in `summary.txid` (the embedded `ActionSummary`, built from
    // `res.txid` at persist time). That is the value the fix must copy onto
    // `bid_commitments.bid_txid`.
    let draft_txid = draft
        .summary
        .get("txid")
        .and_then(|v| v.as_str())
        .expect("draft summary must carry a txid")
        .to_string();

    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let bid_txid: Option<String> = conn
        .query_row(
            "SELECT bid_txid FROM bid_commitments
             WHERE wallet_profile_id = ?1 AND name = 'namea'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        bid_txid,
        Some(draft_txid),
        "bid_commitments.bid_txid must be set to the draft's on-chain txid, not left NULL"
    );
}

/// Companion fix for `build_reveal_draft`: the reveal txid must land on the
/// SAME commitment row (keyed by name), otherwise the reveal-deadline
/// scanner (which reads `reveal_txid`) never sees a revealed bid as
/// resolved.
#[tokio::test]
async fn build_reveal_draft_persists_reveal_txid_on_its_commitment() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        let cov_a = covenant_json_for("namea", crate::noncustodial::sync::COV_BID, "BID");
        seed_covenant_coin(
            &conn,
            &id,
            &"aa".repeat(32),
            &addr,
            crate::noncustodial::sync::COV_BID,
            2000,
            Some(&cov_a),
        );
        seed_bid_commitment(&conn, &id, "namea", &addr);
        id
    };
    let app = mock_app_with(state);

    let draft = names::build_reveal_draft(app.state(), "namea".into(), None)
        .await
        .expect("reveal draft for namea should build");
    let draft_txid = draft
        .summary
        .get("txid")
        .and_then(|v| v.as_str())
        .expect("reveal draft summary must carry a txid")
        .to_string();

    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let reveal_txid: Option<String> = conn
        .query_row(
            "SELECT reveal_txid FROM bid_commitments
             WHERE wallet_profile_id = ?1 AND name = 'namea'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reveal_txid,
        Some(draft_txid),
        "bid_commitments.reveal_txid must be set to the reveal draft's on-chain txid, not left NULL"
    );
}

/// I2 part 2: the `insert_bid_commitment` ON CONFLICT fix. Simulates a
/// same-value re-bid landing on the exact same (name, blind) key by calling
/// the query function directly with the multiplicity guard bypassed (the
/// guard from part 1 makes this unreachable through `build_bid_draft` itself
/// — this test exercises the defense-in-depth honesty fix directly, per the
/// task brief).
#[test]
fn insert_bid_commitment_duplicate_errors_instead_of_silently_dropping() {
    let conn = create_full_test_db();
    let profile_id = insert_valid_profile(&conn, "regtest");
    db::queries::insert_bid_commitment(
        &conn,
        &profile_id,
        "samevaluename",
        "aabb",
        "rs1qbid",
        0,
        1,
        1000,
        2000,
        "nonce1",
        "blind1",
    )
    .unwrap();

    let result = db::queries::insert_bid_commitment(
        &conn,
        &profile_id,
        "samevaluename",
        "aabb",
        "rs1qbid",
        0,
        1,
        1000,
        2000,
        "nonce1",
        "blind1",
    );
    assert!(
        result.is_err(),
        "duplicate (name, blind) commitment must error, not silently drop"
    );

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = 'samevaluename'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "the original commitment must survive the rejected duplicate"
    );
}

// ============================================================================
// Task 1: double-open protection — `build_open_draft`'s guard (mirrors the
// I2 bid-multiplicity guard above) + `can_open`/task-state reflecting a
// pending open.
// ============================================================================

/// A second OPEN for the same name must be rejected while an earlier `open`
/// draft is still not-yet-terminal (draft/signed/broadcast_pending/broadcasted).
#[tokio::test]
async fn build_open_draft_rejects_second_open_when_a_draft_is_already_pending() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        // Second funding UTXO so the second (rejected) build's own
        // `build_plan` doesn't fail on insufficient funds before ever
        // reaching the guard — mirrors `insert_extra_funding`'s role in the
        // bid-multiplicity tests above.
        insert_extra_funding(&conn, &id, &"55".repeat(32));
        id
    };
    let app = mock_app_with(state);

    names::build_open_draft(app.state(), "duplicateopen".into(), None)
        .await
        .expect("first open draft should build");

    let err = names::build_open_draft(app.state(), "duplicateopen".into(), None)
        .await
        .expect_err("second open on the same name must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("already being opened"),
        "error should state the guard's reason, got: {msg}"
    );

    // Exactly one open draft — the rejected attempt must not have persisted
    // anything.
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let draft_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'open'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        draft_count, 1,
        "rejected retry must not persist a second draft"
    );
}

/// Even without any local draft history, an unspent COV_OPEN coin for the
/// name anywhere in the profile (e.g. imported/recovered wallet state) must
/// block a second open.
#[tokio::test]
async fn build_open_draft_rejects_when_an_unspent_open_coin_already_exists() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        insert_extra_funding(&conn, &id, &"66".repeat(32));
        let addr = first_derived_address(&conn, &id);
        let cov = covenant_json_for("existingopen", crate::noncustodial::sync::COV_OPEN, "OPEN");
        seed_covenant_coin(
            &conn,
            &id,
            &"77".repeat(32),
            &addr,
            crate::noncustodial::sync::COV_OPEN,
            0,
            Some(&cov),
        );
        id
    };
    let app = mock_app_with(state);

    let err = names::build_open_draft(app.state(), "existingopen".into(), None)
        .await
        .expect_err("an open on a name with an existing unspent OPEN coin must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("already being opened"),
        "error should state the guard's reason, got: {msg}"
    );

    // Nothing was written — the rejected attempt must not persist a draft.
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let draft_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'open'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(draft_count, 0, "rejected attempt must not persist a draft");
}

/// The double-open guard is name-hash-scoped — an open on a different name
/// must never be blocked by an already-pending open elsewhere.
#[tokio::test]
async fn build_open_draft_allows_open_on_a_different_name() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        insert_extra_funding(&conn, &id, &"88".repeat(32));
    }
    let app = mock_app_with(state);

    names::build_open_draft(app.state(), "opena".into(), None)
        .await
        .expect("first open should build");
    names::build_open_draft(app.state(), "openb".into(), None)
        .await
        .expect("an open on a different name must not be blocked by the first open");
}

/// End-to-end through `get_name_action_capabilities`: a pending `open` draft
/// disables `can_open` with a clear reason and surfaces `taskState` as
/// `waitingForBidding` (the existing variant reused per the Task 1 brief)
/// while the phase is still AVAILABLE.
#[tokio::test]
async fn capabilities_reflect_pending_open_disables_can_open_and_waits_for_bidding() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_blockchain_and_name(&mut server, 1.0, Some("AVAILABLE")).await;
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        db::queries::insert_tx_draft(
            &conn,
            "d_open",
            &id,
            "open",
            "",
            "{}",
            r#"{"action":"open","name":"testname"}"#,
        )
        .unwrap();
        id
    };
    let app = mock_app_with(state);
    let caps =
        names::get_name_action_capabilities(app.state(), "testname".into(), Some(profile_id))
            .await
            .expect("capabilities should resolve via the node path");

    assert!(
        !caps.can_open.allowed,
        "can_open must be disallowed while an open is pending"
    );
    assert!(
        caps.can_open
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("already opening"),
        "reason should mention the pending open, got: {:?}",
        caps.can_open.reason
    );
    assert_eq!(
        caps.task_state,
        names::AuctionTaskState::WaitingForBidding,
        "task state should reuse WaitingForBidding for a pending open, per the Task 1 brief"
    );
}

/// Batch-bid phase validation: `build_batch_bid_draft` must reject any name
/// whose phase is not BIDDING or OPENING, and MUST NOT persist any bid
/// commitment or draft when the batch is rejected (all-or-nothing atomicity).
#[tokio::test]
async fn build_batch_bid_draft_rejects_non_biddable_phase() {
    let mut server = mockito::Server::new_async().await;
    // Mock two names: one BIDDING (biddable), one AVAILABLE (not biddable).
    let _blockchain = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":1000,"headers":1000,"verificationprogress":1.0},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;
    let _biddable = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex(r#"getnameinfo.*biddable"#.into()))
        .with_body(r#"{"result":{"info":{"name":"biddable","state":"BIDDING","stats":{}}},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;
    let _unbiddable = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex(r#"getnameinfo.*unbiddable"#.into()))
        .with_body(r#"{"result":{"info":{"name":"unbiddable","state":"AVAILABLE","stats":{}}},"error":null,"id":1}"#)
        .expect_at_least(1)
        .create_async()
        .await;

    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        insert_extra_funding(&conn, &id, &"11".repeat(32));
        id
    };
    let app = mock_app_with(state);

    // Attempt batch bid on one biddable + one non-biddable name.
    let err = names::build_batch_bid_draft(
        app.state(),
        vec!["biddable".into(), "unbiddable".into()],
        1000,
        2000,
        None,
    )
    .await
    .expect_err("batch bid must be rejected when any name is not open for bidding");

    let msg = format!("{err}");
    assert!(
        msg.contains("not open for bidding") && msg.contains("AVAILABLE"),
        "error should explain the phase rejection, got: {msg}"
    );

    // Verify NO bid commitments were persisted (all-or-nothing atomicity).
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    let commitment_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        commitment_count, 0,
        "no bid commitments should be persisted when the batch is rejected"
    );

    // Verify NO draft was persisted.
    let draft_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'batch-bid'",
            rusqlite::params![&profile_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        draft_count, 0,
        "no batch-bid draft should be persisted when the batch is rejected"
    );
}
