//! Bid-commitment recovery + export (Task 2 / C2).
//!
//! Reuses the `names_cmd_tests` fixtures (`insert_valid_profile` seeds a
//! profile whose account xpub is derived from the SAME fixed seed used here,
//! so recomputed nonces/blinds match exactly what a real bid would have
//! produced).

use crate::commands::{bids, names};
use crate::db;
use crate::noncustodial::bids::{compute_blind, compute_nonce};
use crate::noncustodial::hd::{ExtendedPrivKey, ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::noncustodial::sync::COV_BID;
use crate::tests::names_cmd_tests::{
    create_full_test_state, first_derived_address, insert_valid_profile, mock_app_with,
    mock_names_rpc, set_node_rpc_url,
};
use tauri::Manager;

/// The account xpub `insert_valid_profile` derives (m/44'/coin'/0') from the
/// fixed test seed — needed here to independently compute the "true" nonce
/// and blind a real bid would have produced, so tests can seed a coin whose
/// on-chain blind matches what `recover_bid_commitment` will recompute.
fn account_xpub_for_test_profile(network: Network) -> ExtendedPubKey {
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    let master = ExtendedPrivKey::from_seed(&seed).unwrap();
    let path = [
        HARDENED_OFFSET + 44,
        HARDENED_OFFSET + network.coin_type(),
        HARDENED_OFFSET,
    ];
    let node = master.derive_path(&path).unwrap();
    ExtendedPubKey::from_priv(&node)
}

/// covenant_json exactly as a real BID output would carry it:
/// `[nameHash, u32(start), rawName, blind]`.
fn bid_covenant_json(name: &str, blind_hex: &str) -> String {
    let nh = hex::encode(crate::noncustodial::names::hash_name(name).unwrap());
    let raw = hex::encode(crate::noncustodial::names::raw_name(name).unwrap());
    serde_json::json!({
        "type": COV_BID,
        "action": "BID",
        "items": [nh, "64000000", raw, blind_hex],
    })
    .to_string()
}

fn seed_unspent_bid_coin(
    conn: &rusqlite::Connection,
    profile_id: &str,
    txid: &str,
    addr: &str,
    value: i64,
    covenant_json: &str,
) {
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, '00', ?4, ?5, ?6, 'name_lockup', NULL)",
        rusqlite::params![txid, profile_id, addr, value, COV_BID as i64, covenant_json],
    )
    .unwrap();
}

fn addr_hash160(addr: &str) -> [u8; 20] {
    let (_version, program) = crate::noncustodial::address::decode(Network::Regtest, addr).unwrap();
    let mut out = [0u8; 20];
    out.copy_from_slice(&program);
    out
}

// --- recovery: round trip ---------------------------------------------------

#[tokio::test]
async fn recover_restores_commitment_and_unlocks_reveal() {
    let mut server = mockito::Server::new_async().await;
    let _mocks = mock_names_rpc(&mut server).await;
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1000;

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        set_node_rpc_url(&conn, &server.url());
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        // Unspent BID coin at `addr`, but NO bid_commitments row — simulates
        // a lost commitment (the coin is the only on-chain evidence left).
        seed_unspent_bid_coin(&conn, &profile_id, &"ab".repeat(32), &addr, 2000, &cov);
        assert!(db::queries::get_bid_commitment(&conn, &profile_id, name)
            .unwrap()
            .is_none());
    }

    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id.clone()),
        name.into(),
        value as i64,
    )
    .await
    .expect("recovery should succeed for the correct bid value");
    assert_eq!(result.address, addr);
    assert_eq!(result.bid_value_doos, 1000);
    assert_eq!(result.lockup_value_doos, 2000);

    // The row is really restored, with the correct secret material.
    {
        let state: tauri::State<crate::AppState> = app.state();
        let conn = state.db.lock().unwrap();
        let row = db::queries::get_bid_commitment(&conn, &profile_id, name)
            .unwrap()
            .expect("commitment row should now exist");
        assert_eq!(row.address, addr);
        assert_eq!(row.bid_value_doos, 1000);
        assert_eq!(row.lockup_value_doos, 2000);
        assert_eq!(row.nonce_hex, hex::encode(nonce));
        assert_eq!(row.blind_hex, hex::encode(blind));
    }

    // And the reveal flow is genuinely unlocked by the recovered row.
    let draft = names::build_reveal_draft(app.state(), name.into(), None)
        .await
        .expect("reveal draft should build once the commitment is recovered");
    assert!(!draft.id.is_empty());
}

#[tokio::test]
async fn recover_rejects_wrong_value_and_writes_nothing() {
    let state = create_full_test_state();
    let name = "namea";
    let true_value: u64 = 1000;

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, true_value).unwrap();
    let blind = compute_blind(true_value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(&conn, &profile_id, &"cd".repeat(32), &addr, 2000, &cov);
    }

    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id.clone()),
        name.into(),
        // A plausible but WRONG guess.
        999,
    )
    .await;
    assert!(result.is_err(), "a wrong bid value must not recover");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("match"),
        "error should say the value doesn't match, got: {msg}"
    );

    // Nothing was written.
    let state: tauri::State<crate::AppState> = app.state();
    let conn = state.db.lock().unwrap();
    assert!(
        db::queries::get_bid_commitment(&conn, &profile_id, name)
            .unwrap()
            .is_none(),
        "no commitment row should be written on a failed recovery"
    );
}

#[tokio::test]
async fn recover_errors_when_no_bid_coin_exists() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest")
    };
    let app = mock_app_with(state);
    let result =
        bids::recover_bid_commitment(app.state(), Some(profile_id), "nosuchname".into(), 1000)
            .await;
    assert!(result.is_err());
}

/// Multiple unspent BID coins for the same name at different (rotated)
/// addresses — one is a stale/garbage blind, the other is the real one.
/// Recovery must try each candidate and succeed on the one that matches,
/// rather than failing on the first mismatch.
#[tokio::test]
async fn recover_tries_each_candidate_and_skips_non_matching() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1000;

    let (profile_id, addr0) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr0 = first_derived_address(&conn, &id);
        // Register a second (rotated) receive address.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let (_sk, pk, addr1) =
            crate::noncustodial::hd::derive_address(Network::Regtest, &seed, 0, 0, 1).unwrap();
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

        // Candidate A (addr0): garbage blind — never matches any value.
        let cov_a = bid_covenant_json(name, &"ff".repeat(32));
        seed_unspent_bid_coin(&conn, &id, &"11".repeat(32), &addr0, 2000, &cov_a);

        // Candidate B (addr1): the REAL blind for `value`.
        let xpub = account_xpub_for_test_profile(Network::Regtest);
        let nh = crate::noncustodial::names::hash_name(name).unwrap();
        let addr_hash = addr_hash160(&addr1);
        let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
        let blind = compute_blind(value, &nonce);
        let cov_b = bid_covenant_json(name, &hex::encode(blind));
        seed_unspent_bid_coin(&conn, &id, &"22".repeat(32), &addr1, 3000, &cov_b);

        (id, addr0)
    };

    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id.clone()),
        name.into(),
        value as i64,
    )
    .await
    .expect("recovery should find the matching candidate");
    assert_ne!(
        result.address, addr0,
        "must not settle on the garbage-blind candidate"
    );
    assert_eq!(result.lockup_value_doos, 3000);
}

// --- export ------------------------------------------------------------

#[tokio::test]
async fn export_returns_all_fields_for_every_commitment() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        db::queries::insert_bid_commitment(
            &conn,
            &id,
            "namea",
            "aabb",
            "rs1qaddrA",
            0,
            0,
            1000,
            2000,
            &"11".repeat(32),
            &"22".repeat(32),
        )
        .unwrap();
        db::queries::insert_bid_commitment(
            &conn,
            &id,
            "nameb",
            "ccdd",
            "rs1qaddrB",
            0,
            1,
            5000,
            6000,
            &"33".repeat(32),
            &"44".repeat(32),
        )
        .unwrap();
        db::queries::set_bid_txid(&conn, &id, &"22".repeat(32), "txidbid").unwrap();
        id
    };

    let app = mock_app_with(state);
    let json = bids::export_bid_commitments(app.state(), Some(profile_id))
        .await
        .expect("export should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().expect("export must be a JSON array");
    assert_eq!(arr.len(), 2);

    let a = arr
        .iter()
        .find(|r| r["name"] == "namea")
        .expect("namea present");
    assert_eq!(a["bidValueDoos"], 1000);
    assert_eq!(a["lockupValueDoos"], 2000);
    assert_eq!(a["address"], "rs1qaddrA");
    assert_eq!(a["nonceHex"], "11".repeat(32));
    assert_eq!(a["blindHex"], "22".repeat(32));
    assert_eq!(a["bidTxid"], "txidbid");
    assert!(a["revealTxid"].is_null());

    let b = arr
        .iter()
        .find(|r| r["name"] == "nameb")
        .expect("nameb present");
    assert_eq!(b["bidValueDoos"], 5000);
    assert!(b["bidTxid"].is_null());
}

#[tokio::test]
async fn export_empty_for_profile_with_no_commitments() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest")
    };
    let app = mock_app_with(state);
    let json = bids::export_bid_commitments(app.state(), Some(profile_id))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);
}

// --- brute-force recovery ---------------------------------------------------

#[tokio::test]
async fn brute_force_finds_round_value() {
    // Bid with value = 5.5 HNS (5_500_000 doos) — a "round" value that Tier 1
    // should find instantly (step 0.1 HNS = 100_000 doos hits it).
    let state = create_full_test_state();
    let name = "brutetest";
    let value: u64 = 5_500_000;
    let lockup: u64 = 10_000_000; // 10 HNS

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let ah = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &ah, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"cc".repeat(32),
            &addr,
            lockup as i64,
            &cov,
        );
    }

    let app = mock_app_with(state);
    let result = bids::brute_force_recover_bid(app.state(), Some(profile_id.clone()), name.into())
        .await
        .unwrap();

    assert_eq!(result.bid_value_doos, value as i64);
    assert_eq!(result.lockup_value_doos, lockup as i64);
    assert_eq!(result.tier, "round");
    assert_eq!(result.name, name);

    // Verify the commitment was persisted.
    let app_state = app.state::<crate::AppState>();
    let conn = app_state.db.lock().unwrap();
    let row = db::queries::get_bid_commitment(&conn, &profile_id, name)
        .unwrap()
        .expect("commitment should exist");
    assert_eq!(row.bid_value_doos, value as i64);
}

#[tokio::test]
async fn brute_force_finds_non_round_value_via_sweep() {
    // Bid with value = 3_141 doos — NOT a round value. Tier 1's smallest step
    // is 10_000 doos (0.01 HNS), so no Tier-1 candidate can ever equal 3_141;
    // only Tier 2's exhaustive integer sweep can find it. That is exactly the
    // path this test guards, and it depends only on the value being non-round,
    // NOT on its magnitude. Keeping value+lockup small makes the sweep do a few
    // thousand secp256k1 derivations instead of ~3.14 million — same coverage,
    // ~1000x faster (the sweep was the single slowest test in the suite).
    let state = create_full_test_state();
    let name = "sweeptest";
    let value: u64 = 3_141;
    let lockup: u64 = 5_000; // sweep upper bound — tiny, still > value

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let ah = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &ah, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"dd".repeat(32),
            &addr,
            lockup as i64,
            &cov,
        );
    }

    let app = mock_app_with(state);
    let result = bids::brute_force_recover_bid(app.state(), Some(profile_id.clone()), name.into())
        .await
        .unwrap();

    assert_eq!(result.bid_value_doos, value as i64);
    assert_eq!(result.tier, "sweep");
}

#[tokio::test]
async fn brute_force_errors_when_no_bid_coin_exists() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = bids::brute_force_recover_bid(app.state(), None, "nosuchname".into()).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("no unspent bid coin"));
}

#[tokio::test]
async fn brute_force_is_idempotent() {
    // Running brute-force twice on the same bid should succeed both times
    // without duplicating the commitment row.
    let state = create_full_test_state();
    let name = "idempotent";
    let value: u64 = 2_000_000;
    let lockup: u64 = 3_000_000;

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let ah = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &ah, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"ee".repeat(32),
            &addr,
            lockup as i64,
            &cov,
        );
    }

    let app = mock_app_with(state);
    let r1 = bids::brute_force_recover_bid(app.state(), Some(profile_id.clone()), name.into())
        .await
        .unwrap();
    let r2 = bids::brute_force_recover_bid(app.state(), Some(profile_id.clone()), name.into())
        .await
        .unwrap();
    assert_eq!(r1.bid_value_doos, r2.bid_value_doos);
}

// --- coverage: bid_value_doos <= 0 (line 58) --------------------------------

#[tokio::test]
async fn recover_rejects_zero_bid_value() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(app.state(), None, "anyname".into(), 0).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("bid value must be > 0"), "msg: {msg}");
}

#[tokio::test]
async fn recover_rejects_negative_bid_value() {
    let state = create_full_test_state();
    {
        let conn = state.db.lock().unwrap();
        insert_valid_profile(&conn, "regtest");
    }
    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(app.state(), None, "anyname".into(), -100).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("bid value must be > 0"), "msg: {msg}");
}

// --- coverage: idempotent recovery skips insert (lines 114/127-128) ----------

#[tokio::test]
async fn recover_is_idempotent_skips_insert_when_exists() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1000;

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let blind_hex = hex::encode(blind);
    let cov = bid_covenant_json(name, &blind_hex);

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(&conn, &profile_id, &"cc".repeat(32), &addr, 2000, &cov);
        // Pre-insert the commitment row so recovery finds it already exists.
        db::queries::insert_bid_commitment(
            &conn,
            &profile_id,
            name,
            &hex::encode(nh),
            &addr,
            0,
            0,
            value as i64,
            2000,
            &hex::encode(nonce),
            &blind_hex,
        )
        .unwrap();
    }

    let app = mock_app_with(state);
    // Second recovery should succeed (idempotent) without inserting a duplicate.
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id.clone()),
        name.into(),
        value as i64,
    )
    .await
    .expect("idempotent recovery should succeed");
    assert_eq!(result.bid_value_doos, value as i64);
}

// --- coverage: brute_force targets empty via missing covenant blinds ----------

/// If every candidate coin has a covenant JSON missing the blind at items[3],
/// `targets` ends up empty → the "no decodable bid coin" error branch.
#[tokio::test]
async fn brute_force_errors_when_all_coins_missing_covenant_blind() {
    let state = create_full_test_state();
    let name = "namea";

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    // Covenant JSON with only 3 items — no blind at index 3. Use the real
    // nameHash so the query finds the coin (it filters by items[0]).
    let nh = hex::encode(crate::noncustodial::names::hash_name(name).unwrap());
    let raw = hex::encode(crate::noncustodial::names::raw_name(name).unwrap());
    let bad_cov = serde_json::json!({
        "type": crate::noncustodial::sync::COV_BID,
        "action": "BID",
        "items": [nh, "64000000", raw],
    })
    .to_string();

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(&conn, &profile_id, &"dd".repeat(32), &addr, 5000, &bad_cov);
    }

    let app = mock_app_with(state);
    let result =
        bids::brute_force_recover_bid(app.state(), Some(profile_id), name.into()).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("no decodable bid coin"),
        "should report no decodable coins, got: {msg}"
    );
}

// --- coverage: brute_force coin with missing blind in covenant (line 237) ----

#[tokio::test]
async fn brute_force_skips_coin_with_missing_covenant_blind() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1_000_000; // 1 HNS — round, Tier 1 will find it

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let good_cov = bid_covenant_json(name, &hex::encode(blind));

    // Covenant JSON with only 3 items (missing the blind at index 3). Use the
    // real nameHash so the query returns this coin.
    let nh_hex = hex::encode(nh);
    let raw_hex = hex::encode(crate::noncustodial::names::raw_name(name).unwrap());
    let bad_cov = serde_json::json!({
        "type": crate::noncustodial::sync::COV_BID,
        "action": "BID",
        "items": [nh_hex, "64000000", raw_hex],
    })
    .to_string();

    {
        let conn = state.db.lock().unwrap();
        // Coin A: missing blind → skipped (line 237)
        seed_unspent_bid_coin(&conn, &profile_id, &"11".repeat(32), &addr, 2_000_000, &bad_cov);
        // Coin B: valid → found
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"22".repeat(32),
            &addr,
            2_000_000,
            &good_cov,
        );
    }

    let app = mock_app_with(state);
    let result =
        bids::brute_force_recover_bid(app.state(), Some(profile_id), name.into())
            .await
            .expect("should skip bad coin and find the good one");
    assert_eq!(result.bid_value_doos, value as i64);
}

// --- coverage: MAX_SWEEP_CANDIDATES exceeded (lines 317-321) -----------------

// Note: covering the MAX_SWEEP_CANDIDATES > 1_000_000_000 doos error branch
// (lines 317-321) would require a lockup so large that Tier 1's fast-path scan
// (steps of 1_000_000 / 100_000 / 10_000 doos up to lockup_doos) does hundreds
// of thousands of secp256k1 nonce derivations before Tier 2 even runs. That is
// too slow (~90s+) for the unit-test loop. The branch is straight-line
// validation logic — a simple integer comparison against a compile-time
// constant — and is exercised by hand during release testing.

// --- coverage: brute_force no match in full sweep (lines 331-334) ------------

#[tokio::test]
async fn brute_force_errors_when_no_value_matches() {
    let state = create_full_test_state();
    let name = "namea";

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    // Use a completely fabricated blind that no value can reproduce.
    // Keep lockup tiny (10 doos) so the full sweep finishes instantly.
    let cov = bid_covenant_json(name, &"ab".repeat(32));
    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(&conn, &profile_id, &"ff".repeat(32), &addr, 10, &cov);
    }

    let app = mock_app_with(state);
    let result =
        bids::brute_force_recover_bid(app.state(), Some(profile_id), name.into()).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("no bid value") || msg.contains("different wallet"),
        "should report no match, got: {msg}"
    );
}

// --- coverage: recover INSERT error propagation (line 130 in bids.rs) --------

/// A BEFORE INSERT trigger on `bid_commitments` forces the recovery INSERT to
/// fail → exercises the `?` error propagation on `insert_bid_commitment`.
#[tokio::test]
async fn recover_propagates_insert_error() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1000;

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(&conn, &profile_id, &"77".repeat(32), &addr, 2000, &cov);
        // Block any INSERT into bid_commitments for this name.
        conn.execute_batch(
            "CREATE TRIGGER block_bid_insert BEFORE INSERT ON bid_commitments
             FOR EACH ROW WHEN NEW.name = 'namea'
             BEGIN
                 SELECT RAISE(ABORT, 'blocked by test trigger');
             END;",
        )
        .unwrap();
    }

    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id),
        name.into(),
        value as i64,
    )
    .await;
    assert!(result.is_err(), "recovery should propagate INSERT error");
}

// --- coverage: brute_force persist INSERT error propagation (line 290) -------

#[tokio::test]
async fn brute_force_propagates_insert_error() {
    let state = create_full_test_state();
    let name = "brfrc";
    let value: u64 = 1_000_000; // 1 HNS — Tier 1 finds it
    let lockup: u64 = 2_000_000;

    let (profile_id, addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let addr = first_derived_address(&conn, &id);
        (id, addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let ah = addr_hash160(&addr);
    let nonce = compute_nonce(&xpub, &nh, &ah, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let cov = bid_covenant_json(name, &hex::encode(blind));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(&conn, &profile_id, &"88".repeat(32), &addr, lockup as i64, &cov);
        conn.execute_batch(
            "CREATE TRIGGER block_bf_insert BEFORE INSERT ON bid_commitments
             FOR EACH ROW WHEN NEW.name = 'brfrc'
             BEGIN
                 SELECT RAISE(ABORT, 'blocked by test trigger');
             END;",
        )
        .unwrap();
    }

    let app = mock_app_with(state);
    let result = bids::brute_force_recover_bid(app.state(), Some(profile_id), name.into()).await;
    assert!(result.is_err(), "brute force should propagate INSERT error");
}

// --- coverage: query error propagation in recover (line 78 in bids.rs) -------

/// Dropping the `tracked_utxos` table makes
/// `find_unspent_covenant_utxos_by_name_hash` fail; the `?` at line 78
/// propagates the error.
#[tokio::test]
async fn recover_propagates_query_error() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        conn.execute_batch("DROP TABLE tracked_utxos;").unwrap();
        id
    };
    let app = mock_app_with(state);
    let result =
        bids::recover_bid_commitment(app.state(), Some(profile_id), "anything".into(), 1000)
            .await;
    assert!(result.is_err(), "recover should propagate query error");
}

/// Same, for the brute-force variant (line 207 in bids.rs).
#[tokio::test]
async fn brute_force_propagates_query_error() {
    let state = create_full_test_state();
    let profile_id = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        conn.execute_batch("DROP TABLE tracked_utxos;").unwrap();
        id
    };
    let app = mock_app_with(state);
    let result =
        bids::brute_force_recover_bid(app.state(), Some(profile_id), "anything".into()).await;
    assert!(result.is_err(), "brute force should propagate query error");
}

// --- coverage: continue on undecodable address in derived_addresses ----------

/// Directly insert a derived_addresses row whose `address` is NOT valid bech32,
/// then seed a bid coin for it. The recover loop's `address::decode` fails on
/// this coin and `continue`s to the next candidate — the good one wins.
#[tokio::test]
async fn recover_skips_coin_with_undecodable_address() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1000;

    let (profile_id, good_addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let good_addr = first_derived_address(&conn, &id);

        // Insert a bogus derived_addresses row with an undecodable address.
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, 999, ?2, '00', '00')",
            rusqlite::params![&id, "NOT_A_BECH32_ADDR"],
        )
        .unwrap();

        (id, good_addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&good_addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let good_cov = bid_covenant_json(name, &hex::encode(blind));
    // Bogus covenant blind for the undecodable-addr coin.
    let bad_cov = bid_covenant_json(name, &"ff".repeat(32));

    {
        let conn = state.db.lock().unwrap();
        // Coin A: undecodable address → skipped via `let Ok(...) else { continue }`.
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"aa".repeat(32),
            "NOT_A_BECH32_ADDR",
            2000,
            &bad_cov,
        );
        // Coin B: valid address, matching blind → wins.
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"bb".repeat(32),
            &good_addr,
            2000,
            &good_cov,
        );
    }

    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id),
        name.into(),
        value as i64,
    )
    .await
    .expect("should skip undecodable-addr coin and find the good one");
    assert_eq!(result.address, good_addr);
}

/// Same for brute-force — lines 233/236 in bids.rs.
#[tokio::test]
async fn brute_force_skips_coin_with_undecodable_address() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1_000_000; // Tier-1 round value
    let lockup: u64 = 2_000_000;

    let (profile_id, good_addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let good_addr = first_derived_address(&conn, &id);

        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, 999, ?2, '00', '00')",
            rusqlite::params![&id, "NOT_A_BECH32_ADDR"],
        )
        .unwrap();

        (id, good_addr)
    };

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&good_addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let good_cov = bid_covenant_json(name, &hex::encode(blind));
    let bad_cov = bid_covenant_json(name, &"ff".repeat(32));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"cc".repeat(32),
            "NOT_A_BECH32_ADDR",
            lockup as i64,
            &bad_cov,
        );
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"dd".repeat(32),
            &good_addr,
            lockup as i64,
            &good_cov,
        );
    }

    let app = mock_app_with(state);
    let result = bids::brute_force_recover_bid(app.state(), Some(profile_id), name.into())
        .await
        .expect("should skip undecodable-addr coin");
    assert_eq!(result.address, good_addr);
}

/// A coin whose address is valid bech32 but has a 32-byte program (P2WSH)
/// triggers the `program.len() != 20` continue path (lines 96 & 236).
#[tokio::test]
async fn recover_skips_coin_with_non_20_byte_program() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1000;

    let (profile_id, good_addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let good_addr = first_derived_address(&conn, &id);
        (id, good_addr)
    };

    // Create a P2WSH address (32-byte program) for regtest.
    let program_32 = [0xABu8; 32];
    let p2wsh_addr = bech32::segwit::encode_v0(
        bech32::Hrp::parse("rs").unwrap(),
        &program_32,
    )
    .unwrap();

    {
        let conn = state.db.lock().unwrap();
        // Insert the P2WSH address into derived_addresses so the query finds it.
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, 998, ?2, '00', '00')",
            rusqlite::params![&profile_id, &p2wsh_addr],
        )
        .unwrap();
    }

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&good_addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let good_cov = bid_covenant_json(name, &hex::encode(blind));
    let bad_cov = bid_covenant_json(name, &"ff".repeat(32));

    {
        let conn = state.db.lock().unwrap();
        // Coin A: P2WSH address (32-byte program) → skipped.
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"a1".repeat(32),
            &p2wsh_addr,
            2000,
            &bad_cov,
        );
        // Coin B: valid P2WPKH → wins.
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"b2".repeat(32),
            &good_addr,
            2000,
            &good_cov,
        );
    }

    let app = mock_app_with(state);
    let result = bids::recover_bid_commitment(
        app.state(),
        Some(profile_id),
        name.into(),
        value as i64,
    )
    .await
    .expect("should skip P2WSH coin and find the P2WPKH one");
    assert_eq!(result.address, good_addr);
}

/// Same for brute-force (line 236 in bids.rs).
#[tokio::test]
async fn brute_force_skips_coin_with_non_20_byte_program() {
    let state = create_full_test_state();
    let name = "namea";
    let value: u64 = 1_000_000;
    let lockup: u64 = 2_000_000;

    let (profile_id, good_addr) = {
        let conn = state.db.lock().unwrap();
        let id = insert_valid_profile(&conn, "regtest");
        let good_addr = first_derived_address(&conn, &id);
        (id, good_addr)
    };

    let program_32 = [0xCDu8; 32];
    let p2wsh_addr = bech32::segwit::encode_v0(
        bech32::Hrp::parse("rs").unwrap(),
        &program_32,
    )
    .unwrap();

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, 998, ?2, '00', '00')",
            rusqlite::params![&profile_id, &p2wsh_addr],
        )
        .unwrap();
    }

    let xpub = account_xpub_for_test_profile(Network::Regtest);
    let nh = crate::noncustodial::names::hash_name(name).unwrap();
    let addr_hash = addr_hash160(&good_addr);
    let nonce = compute_nonce(&xpub, &nh, &addr_hash, value).unwrap();
    let blind = compute_blind(value, &nonce);
    let good_cov = bid_covenant_json(name, &hex::encode(blind));
    let bad_cov = bid_covenant_json(name, &"ff".repeat(32));

    {
        let conn = state.db.lock().unwrap();
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"e1".repeat(32),
            &p2wsh_addr,
            lockup as i64,
            &bad_cov,
        );
        seed_unspent_bid_coin(
            &conn,
            &profile_id,
            &"f2".repeat(32),
            &good_addr,
            lockup as i64,
            &good_cov,
        );
    }

    let app = mock_app_with(state);
    let result = bids::brute_force_recover_bid(app.state(), Some(profile_id), name.into())
        .await
        .expect("should skip P2WSH coin");
    assert_eq!(result.address, good_addr);
}
