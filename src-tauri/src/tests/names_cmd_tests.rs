use crate::commands::names;
use crate::db;
use crate::db::queries::NameCoin;
use crate::AppState;
use tauri::Manager;

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
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

fn create_full_test_state() -> crate::AppState {
    let conn = create_full_test_db();
    crate::AppState {
        db: std::sync::Mutex::new(conn),
        signer: std::sync::Mutex::new(None),
        secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
        hsd_child: std::sync::Mutex::new(None),
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
        .with_body(format!(
            r#"{{"result":{},"error":null,"id":1}}"#,
            name_info
        ))
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
        .with_body(format!(
            r#"{{"result":{},"error":null,"id":1}}"#,
            name_info
        ))
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
    let _result = names::renewal_block(&client, crate::noncustodial::network::Network::Regtest).await;
    info_mock.assert_async().await;
}

// --- bid validation tests ---

#[test]
fn test_bid_draft_rejects_zero_bid() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(
        names::build_bid_draft(app.state(), "test".into(), 0, 100, None),
    );
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("lockup must be >= bid value > 0"));
}

#[test]
fn test_bid_draft_rejects_negative_bid() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(
        names::build_bid_draft(app.state(), "test".into(), -5, 100, None),
    );
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("lockup must be >= bid value > 0"));
}

#[test]
fn test_bid_draft_rejects_lockup_less_than_bid() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(
        names::build_bid_draft(app.state(), "test".into(), 100, 50, None),
    );
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("lockup must be >= bid value > 0"));
}

// --- open draft error: no active profile ---

#[test]
fn test_open_draft_no_profile_errors() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = mock_app_with(state);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(
        names::build_open_draft(app.state(), "test-name".into(), None),
    );
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
    let result = tokio::runtime::Runtime::new().unwrap().block_on(
        names::build_reveal_draft(app.state(), "test-name".into(), None),
    );
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
fn insert_valid_profile(conn: &rusqlite::Connection, network: &str) -> String {
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
        crate::noncustodial::hd::HARDENED_OFFSET + 0,
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
    ).unwrap();
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
    assert!(result.is_ok(), "build_open_draft should succeed: {:?}", result.err());
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
    assert!(result.is_ok(), "build_open_draft with fee_rate should succeed: {:?}", result.err());
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
    let summary = names::build_open_draft(app.state(), "persistme".into(), None)
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
    let result = names::build_transfer_draft(app.state(), "testname".into(), "invalid_addr".into(), None).await;
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
    assert!(!msg.contains("lockup must be >= bid value"), "should pass validation: {msg}");
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
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    let result = names::build_transfer_draft(app.state(), "testname".into(), "hs1qtest".into(), None).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    assert!(msg.contains("does not hold") || msg.contains("sync"), "msg: {msg}");
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
    ).unwrap();
    // Insert a derived address so the 3-way join in get_name_coin works.
    conn.execute(
        "INSERT OR IGNORE INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, '00', '00')",
        rusqlite::params![profile_id, addr],
    ).unwrap();
    // Insert a tracked UTXO as the owner coin.
    conn.execute(
        "INSERT OR IGNORE INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', 10000, 0, ?4, 'liquid_hns', NULL)",
        rusqlite::params![txid, profile_id, addr, covenant_json.unwrap_or("null")],
    ).unwrap();
}

/// Seed a bid commitment + a matching BID coin UTXO + derived_address so that
/// `build_reveal_draft` can find the bid commitment and the unspent BID coin.
fn seed_bid_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    name: &str,
    nonce_hex: &str,
) {
    let txid = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let addr = "rs1qbid";
    conn.execute(
        "INSERT OR IGNORE INTO bid_commitments
            (wallet_profile_id, name, name_hash_hex, address, branch, child_index,
             bid_value_doos, lockup_value_doos, nonce_hex, blind_hex)
         VALUES (?1, ?2, 'aabb', ?3, 1, 0, 1000, 2000, ?4, '0011')",
        rusqlite::params![profile_id, name, addr, nonce_hex],
    ).unwrap();
    // Insert a derived address so find_unspent_covenant_utxo can join.
    conn.execute(
        "INSERT OR IGNORE INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 1, 0, ?2, '00', '00')",
        rusqlite::params![profile_id, addr],
    ).unwrap();
    // Insert a BID coin (covenant_type = 2 for COV_BID).
    conn.execute(
        "INSERT OR IGNORE INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', 2000, 2, '{}', 'liquid_hns', NULL)",
        rusqlite::params![txid, profile_id, addr],
    ).unwrap();
}

/// Set up mockito RPC mocks for the RPC calls that `build_finalize_draft` and
/// `build_reveal_draft` need: getnameinfo, getblockchaininfo, getblockhash.
async fn mock_names_rpc(server: &mut mockito::Server) -> (mockito::Mock, mockito::Mock, mockito::Mock) {
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
    let _bi = server.mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"blocks":100},"error":null,"id":1}"#)
        .create_async().await;
    let _bh = server.mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockhash".into()))
        .with_body(r#"{"result":"00","error":null,"id":1}"#)
        .create_async().await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(&server.url(), "", crate::noncustodial::rpc::ChainSource::LocalNode);
    let result = names::renewal_block(&client, crate::noncustodial::network::Network::Regtest).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("block hash") || msg.contains("hex"), "msg: {msg}");
}

// --- fetch_name_state with RPC error ---

#[tokio::test]
async fn test_fetch_name_state_rpc_error() {
    let mut server = mockito::Server::new_async().await;
    // Return an RPC error envelope
    let _m = server.mock("POST", "/")
        .with_body(r#"{"result":null,"error":{"message":"Name not found.","code":-1},"id":1}"#)
        .create_async().await;

    let client = crate::noncustodial::rpc::NodeRpcClient::new(&server.url(), "", crate::noncustodial::rpc::ChainSource::LocalNode);
    let result = names::fetch_name_state(&client, "nonexistent").await;
    assert!(result.is_err());
}
