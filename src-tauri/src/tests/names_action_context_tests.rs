//! Tests for `find_name_action_context` — the DB-backed context builder for name actions.
//!
//! This function is the critical seam between the DB (tracked UTXOs, bid commitments,
//! drafts) and the capability model. It gathers evidence from multiple queries and
//! synthesizes the `NameActionContext` struct that feeds `build_name_action_capabilities`.
//!
//! Tests here use an in-memory DB with the full schema, seeding fixtures for:
//! - Bid commitments (with/without coins)
//! - Owner coins (various covenant types)
//! - Reveal coins
//! - Pending drafts
//! - Pending OPEN coins

use crate::commands::names::find_name_action_context;
use crate::db;
use crate::noncustodial::sync::{self, COV_REVEAL};

const PROFILE: &str = "test_profile";
const NAME: &str = "example";
const ADDRESS: &str = "hs1qtest0000000000000000000000000000000";

fn test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn seed_profile(conn: &rusqlite::Connection) {
    db::queries::insert_wallet_profile(
        conn,
        PROFILE,
        "Test",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
}

fn seed_derived_address(
    conn: &rusqlite::Connection,
    address: &str,
    branch: i64,
    child_index: i64,
) {
    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index, address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, ?2, ?3, ?4, 'deadbeef', 'deadbeef')",
        rusqlite::params![PROFILE, branch, child_index, address],
    )
    .unwrap();
}

fn seed_tracked_utxo(
    conn: &rusqlite::Connection,
    txid: &str,
    vout: i64,
    address: &str,
    covenant_type: i64,
    covenant_json: Option<&str>,
) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, ?2, ?3, ?4, 'deadbeef', 100000, ?5, ?6, 'name_control', NULL)",
        rusqlite::params![txid, vout, PROFILE, address, covenant_type, covenant_json],
    )
    .unwrap();
}

fn seed_bid_commitment(
    conn: &rusqlite::Connection,
    name: &str,
    name_hash_hex: &str,
    address: &str,
) {
    db::queries::insert_bid_commitment(
        conn,
        PROFILE,
        name,
        name_hash_hex,
        address,
        0,
        0,
        1_000_000,
        2_000_000,
        "nonce",
        "blind",
    )
    .unwrap();
}

fn seed_draft(
    conn: &rusqlite::Connection,
    id: &str,
    action: &str,
    name: &str,
) {
    let summary = serde_json::json!({ "name": name }).to_string();
    db::queries::insert_tx_draft(
        conn,
        id,
        PROFILE,
        action,
        "0100000000",  // minimal unsigned tx hex
        "[]",          // signing_inputs_json
        &summary,
    )
    .unwrap();
}

fn seed_tracked_name_state(
    conn: &rusqlite::Connection,
    name: &str,
    name_hash_hex: &str,
    state: &str,
    owner_txid: Option<&str>,
    owner_vout: Option<i64>,
    height: Option<i64>,
) {
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![PROFILE, name, name_hash_hex, state, owner_txid, owner_vout, height],
    )
    .unwrap();
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn find_name_action_context_no_evidence() {
    // A name with zero evidence: no bid, no owner coin, no drafts.
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(!ctx.has_bid_commitment);
    assert!(!ctx.has_bid_coin);
    assert!(!ctx.has_reveal_coin);
    assert!(!ctx.has_owner_coin);
    assert_eq!(ctx.owner_covenant_type, None);
    assert_eq!(ctx.name_height, None);
    assert_eq!(ctx.transfer_has_items, None);
    assert_eq!(ctx.existing_bid_count, 0);
    assert!(!ctx.has_pending_open);
}

#[test]
fn find_name_action_context_with_bid_commitment_no_coin() {
    // Bid commitment exists (in DB) but the coin hasn't been synced yet.
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    seed_bid_commitment(&conn, NAME, name_hash_hex, ADDRESS);

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(ctx.has_bid_commitment);
    assert!(!ctx.has_bid_coin); // No coin synced yet
    assert!(!ctx.has_reveal_coin);
    assert_eq!(ctx.existing_bid_count, 1);
}

#[test]
fn find_name_action_context_with_bid_coin() {
    // Bid commitment + synced BID coin (covenant type 3).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    seed_bid_commitment(&conn, NAME, name_hash_hex, ADDRESS);

    let covenant_json = serde_json::json!({
        "type": 3,
        "action": "BID",
        "items": [name_hash_hex, "64000000", "7261", "deadbeef"]
    })
    .to_string();

    seed_tracked_utxo(
        &conn,
        "tx1",
        0,
        ADDRESS,
        sync::COV_BID as i64,
        Some(&covenant_json),
    );

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(ctx.has_bid_commitment);
    assert!(ctx.has_bid_coin);
    assert!(!ctx.has_reveal_coin); // No reveal coin yet
}

#[test]
fn find_name_action_context_with_reveal_coin() {
    // Bid commitment + BID coin + REVEAL coin (covenant type 4).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    seed_bid_commitment(&conn, NAME, name_hash_hex, ADDRESS);

    let covenant_json = serde_json::json!({
        "type": 3,
        "action": "BID",
        "items": [name_hash_hex]
    })
    .to_string();

    seed_tracked_utxo(
        &conn,
        "tx1",
        0,
        ADDRESS,
        sync::COV_BID as i64,
        Some(&covenant_json),
    );

    let reveal_covenant = serde_json::json!({
        "type": 4,
        "action": "REVEAL",
        "items": [name_hash_hex]
    })
    .to_string();

    seed_tracked_utxo(
        &conn,
        "tx2",
        0,
        ADDRESS,
        COV_REVEAL as i64,
        Some(&reveal_covenant),
    );

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(ctx.has_bid_commitment);
    assert!(ctx.has_bid_coin);
    assert!(ctx.has_reveal_coin);
}

#[test]
fn find_name_action_context_with_owner_coin() {
    // Owner coin (covenant type 6 = REGISTER).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    let owner_covenant = serde_json::json!({
        "type": 6,
        "action": "REGISTER",
        "items": [name_hash_hex, "example", "0"]
    })
    .to_string();

    seed_tracked_utxo(
        &conn,
        "tx3",
        0,
        ADDRESS,
        6, // COV_REGISTER
        Some(&owner_covenant),
    );

    // Link the name to the owner UTXO via tracked_name_states
    seed_tracked_name_state(&conn, NAME, name_hash_hex, "CLOSED", Some("tx3"), Some(0), Some(12345));

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(ctx.has_owner_coin);
    assert_eq!(ctx.owner_covenant_type, Some(6));
    assert_eq!(ctx.name_height, Some(12345)); // height from tracked_name_states
}

#[test]
fn find_name_action_context_with_pending_open_draft() {
    // No coins, but a pending OPEN draft exists.
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    seed_draft(&conn, "draft_open_1", "open", NAME);

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(ctx.has_pending_open);
}

#[test]
fn find_name_action_context_multiple_bids() {
    // Multiple bid commitments for the same name (bid multiplicity).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);
    seed_derived_address(&conn, "hs1qother0000000000000000000000000000000", 0, 1);

    let name_hash_hex = "aabbccdd";
    seed_bid_commitment(&conn, NAME, name_hash_hex, ADDRESS);
    // Second bid at a different address with a different blind_hex to avoid
    // the uniqueness constraint on (wallet_profile_id, name, blind_hex).
    db::queries::insert_bid_commitment(
        &conn,
        PROFILE,
        NAME,
        name_hash_hex,
        "hs1qother0000000000000000000000000000000",
        0,
        1,
        1_000_000,
        2_000_000,
        "nonce2",
        "blind2",
    )
    .unwrap();

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert_eq!(ctx.existing_bid_count, 2);
}

#[test]
fn find_name_action_context_with_reveal_txid() {
    // Bid commitment with a reveal_txid set (reveal broadcast).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    db::queries::insert_bid_commitment(
        &conn,
        PROFILE,
        NAME,
        name_hash_hex,
        ADDRESS,
        0,
        0,
        1_000_000,
        2_000_000,
        "nonce",
        "blind",
    )
    .unwrap();

    // Update the bid commitment to set reveal_txid.
    conn.execute(
        "UPDATE bid_commitments SET reveal_txid = ?1 WHERE name = ?2 AND wallet_profile_id = ?3",
        rusqlite::params!["reveal_tx_123", NAME, PROFILE],
    )
    .unwrap();

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert_eq!(ctx.reveal_txid.as_deref(), Some("reveal_tx_123"));
}

#[test]
fn find_name_action_context_reveal_draft_status() {
    // Bid commitment with reveal_txid + a draft for that txid with a status.
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    db::queries::insert_bid_commitment(
        &conn,
        PROFILE,
        NAME,
        name_hash_hex,
        ADDRESS,
        0,
        0,
        1_000_000,
        2_000_000,
        "nonce",
        "blind",
    )
    .unwrap();

    conn.execute(
        "UPDATE bid_commitments SET reveal_txid = ?1 WHERE name = ?2 AND wallet_profile_id = ?3",
        rusqlite::params!["reveal_tx_123", NAME, PROFILE],
    )
    .unwrap();

    // Insert a draft with that txid and status "confirmed".
    db::queries::insert_tx_draft(
        &conn,
        "draft_reveal",
        PROFILE,
        "reveal",
        "0100000000",
        "[]",
        &serde_json::json!({"name": NAME}).to_string(),
    )
    .unwrap();

    // Update the draft to have the reveal txid and status.
    conn.execute(
        "UPDATE wallet_tx_drafts SET txid = ?1, status = ?2 WHERE id = ?3",
        rusqlite::params!["reveal_tx_123", "confirmed", "draft_reveal"],
    )
    .unwrap();

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert_eq!(ctx.reveal_draft_status.as_deref(), Some("confirmed"));
}

#[test]
fn find_name_action_context_transfer_with_items() {
    // Owner coin with a TRANSFER covenant that has items (pending finalize).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    let transfer_covenant = serde_json::json!({
        "type": 8,
        "action": "TRANSFER",
        "items": [name_hash_hex, "recipient_addr", "0", "0"]
    })
    .to_string();

    seed_tracked_utxo(
        &conn,
        "tx4",
        0,
        ADDRESS,
        8, // COV_TRANSFER
        Some(&transfer_covenant),
    );

    // Link to tracked_name_states
    seed_tracked_name_state(&conn, NAME, name_hash_hex, "TRANSFER", Some("tx4"), Some(0), None);

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert_eq!(ctx.transfer_has_items, Some(true));
}

#[test]
fn find_name_action_context_transfer_without_items() {
    // Owner coin with a TRANSFER covenant that has no items (shouldn't happen,
    // but the code handles it gracefully).
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    let transfer_covenant = serde_json::json!({
        "type": 8,
        "action": "TRANSFER",
        "items": []
    })
    .to_string();

    seed_tracked_utxo(
        &conn,
        "tx4",
        0,
        ADDRESS,
        8, // COV_TRANSFER
        Some(&transfer_covenant),
    );

    seed_tracked_name_state(&conn, NAME, name_hash_hex, "TRANSFER", Some("tx4"), Some(0), None);

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert_eq!(ctx.transfer_has_items, Some(false));
}

#[test]
fn find_name_action_context_spent_coins_ignored() {
    // Bid coin that's been spent (spent_by_txid set) should be ignored.
    let conn = test_db();
    seed_profile(&conn);
    seed_derived_address(&conn, ADDRESS, 0, 0);

    let name_hash_hex = "aabbccdd";
    seed_bid_commitment(&conn, NAME, name_hash_hex, ADDRESS);

    let covenant_json = serde_json::json!({
        "type": 3,
        "action": "BID",
        "items": [name_hash_hex]
    })
    .to_string();

    // Insert a spent BID coin.
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, ?2, ?3, ?4, 'deadbeef', 100000, ?5, ?6, 'name_control', ?7)",
        rusqlite::params![
            "tx1",
            0,
            PROFILE,
            ADDRESS,
            sync::COV_BID as i64,
            Some(&covenant_json),
            "spent_by_tx"
        ],
    )
    .unwrap();

    let ctx = find_name_action_context(&conn, PROFILE, NAME).unwrap();
    assert!(ctx.has_bid_commitment);
    assert!(!ctx.has_bid_coin); // Spent coin is ignored
}

// ============================================================================
// RPC-injected tests for renewal_block and fee_rate helpers
// ============================================================================

#[cfg(test)]
mod rpc_injected_tests {
    use crate::commands::names::{fee_rate, renewal_block, Ctx};
    use crate::noncustodial::network::Network;
    use crate::noncustodial::rpc::BlockchainInfo;
    use crate::tests::mock_node_rpc::MockNodeRpc;

    /// Build a minimal `BlockchainInfo` with a specific tip height. Other
    /// optional fields are left at defaults — `renewal_block` only reads
    /// `blocks`.
    fn blockchain_info(tip: i64) -> BlockchainInfo {
        BlockchainInfo {
            blocks: tip,
            headers: Some(tip),
            verification_progress: Some(1.0),
            chain: Some("regtest".into()),
            bestblockhash: None,
        }
    }

    #[tokio::test]
    async fn renewal_block_computes_correct_height() {
        // Regtest renewal_maturity = 50, so height = tip - 2*50 = tip - 100.
        // The mock returns the same hash for any height; we're testing that
        // the RPC path completes and the hash is byte-reversed correctly.
        let mock = MockNodeRpc::new()
            .with_blockchain_info(blockchain_info(1000))
            .with_block_hash(
                "0000000000000000000000000000000000000000000000000000000000000001".into(),
            );

        let result = renewal_block(&mock, Network::Regtest).await.unwrap();
        // The hash "..01" is reversed (display -> internal), so the first
        // byte becomes 0x01 and the rest zeros.
        let mut expected = [0u8; 32];
        expected[0] = 1;
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn renewal_block_clamps_negative_height_to_zero() {
        // When tip < 2*renewal_maturity, the code clamps to 0.
        // The mock still returns a valid hash — we're proving the clamp
        // doesn't panic or produce a negative-height RPC error.
        let mock = MockNodeRpc::new()
            .with_blockchain_info(blockchain_info(10))
            .with_block_hash(
                "0000000000000000000000000000000000000000000000000000000000000002".into(),
            );

        let result = renewal_block(&mock, Network::Regtest).await.unwrap();
        let mut expected = [0u8; 32];
        expected[0] = 2;
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn renewal_block_rejects_invalid_hex() {
        // A block hash that isn't valid hex should surface as AppError::Rpc.
        let mock = MockNodeRpc::new()
            .with_blockchain_info(blockchain_info(1000))
            .with_block_hash("not_valid_hex_zz".into());

        let err = renewal_block(&mock, Network::Regtest).await.unwrap_err();
        match err {
            crate::error::AppError::Rpc(msg) => assert!(msg.contains("bad block hash")),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn renewal_block_rejects_wrong_length_hash() {
        // A hex-valid but wrong-length hash (not 32 bytes) should error.
        let mock = MockNodeRpc::new()
            .with_blockchain_info(blockchain_info(1000))
            .with_block_hash("aabbccdd".into()); // only 4 bytes

        let err = renewal_block(&mock, Network::Regtest).await.unwrap_err();
        match err {
            crate::error::AppError::Rpc(msg) => assert!(msg.contains("not 32 bytes")),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn renewal_block_propagates_blockchain_info_error() {
        // RPC error from get_blockchain_info should propagate unchanged.
        let mock = MockNodeRpc::new().with_blockchain_info_err("connection refused");

        let err = renewal_block(&mock, Network::Regtest).await.unwrap_err();
        match err {
            crate::error::AppError::Rpc(msg) => assert!(msg.contains("connection refused")),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn renewal_block_propagates_block_hash_error() {
        // RPC error from get_block_hash should propagate.
        let mock = MockNodeRpc::new()
            .with_blockchain_info(blockchain_info(1000))
            .with_block_hash_err("block not found");

        let err = renewal_block(&mock, Network::Regtest).await.unwrap_err();
        match err {
            crate::error::AppError::Rpc(msg) => assert!(msg.contains("block not found")),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // fee_rate() helper: explicit override > settings kvb / 1000 > default,
    // with a hard floor of MIN_FEE_RATE_PER_BYTE.
    // ------------------------------------------------------------------

    /// Build a minimal `Ctx` for testing `fee_rate`. Only `settings` matters —
    /// the other fields aren't read by the fee resolver.
    fn ctx_with_settings(settings: std::collections::HashMap<String, String>) -> Ctx {
        use crate::noncustodial::hd::ExtendedPubKey;
        // A dummy ExtendedPubKey — fee_rate() doesn't read it, only settings.
        let xpub = ExtendedPubKey::from_parts(
            &[2; 33],  // dummy compressed pubkey (valid prefix + 32 zero bytes)
            &[0; 32],  // dummy chain code
        )
        .unwrap();
        Ctx {
            profile_id: "test".into(),
            network: Network::Main,
            account: 0,
            account_xpub: xpub,
            change_address: "addr".into(),
            funding: vec![],
            settings,
        }
    }
    #[test]
    fn fee_rate_uses_explicit_override() {
        // Explicit fee_rate arg wins over everything else.
        let mut s = std::collections::HashMap::new();
        s.insert("fee_rate_doos_per_kvb".into(), "5000".into());
        let ctx = ctx_with_settings(s);

        assert_eq!(fee_rate(&ctx, Some(100)), 100);
    }

    #[test]
    fn fee_rate_falls_back_to_settings_kvb() {
        // No explicit override — kvb setting / 1000 = per-byte rate.
        let mut s = std::collections::HashMap::new();
        s.insert("fee_rate_doos_per_kvb".into(), "5000".into());
        let ctx = ctx_with_settings(s);

        // 5000 / 1000 = 5 doos/byte (well above MIN_FEE_RATE_PER_BYTE = 1).
        assert_eq!(fee_rate(&ctx, None), 5);
    }

    #[test]
    fn fee_rate_defaults_when_settings_missing() {
        // No override and no setting — fall back to DEFAULT_FEE_RATE_PER_BYTE.
        let ctx = ctx_with_settings(std::collections::HashMap::new());

        assert_eq!(
            fee_rate(&ctx, None),
            crate::noncustodial::send::DEFAULT_FEE_RATE_PER_BYTE
        );
    }

    #[test]
    fn fee_rate_clamps_below_minimum() {
        // A tiny kvb rate (100/1000 = 0) clamps up to MIN_FEE_RATE_PER_BYTE.
        let mut s = std::collections::HashMap::new();
        s.insert("fee_rate_doos_per_kvb".into(), "100".into());
        let ctx = ctx_with_settings(s);

        assert_eq!(
            fee_rate(&ctx, None),
            crate::noncustodial::send::MIN_FEE_RATE_PER_BYTE
        );
    }

    #[test]
    fn fee_rate_ignores_unparseable_setting() {
        // A garbage kvb setting parses as None and falls through to default.
        let mut s = std::collections::HashMap::new();
        s.insert("fee_rate_doos_per_kvb".into(), "not_a_number".into());
        let ctx = ctx_with_settings(s);

        assert_eq!(
            fee_rate(&ctx, None),
            crate::noncustodial::send::DEFAULT_FEE_RATE_PER_BYTE
        );
    }
}
