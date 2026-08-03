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
    mock_names_rpc, set_hsrd_rpc_url,
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
        set_hsrd_rpc_url(&conn, &server.url());
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
