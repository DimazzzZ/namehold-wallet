//! Command-level tests for `sign_name_message` (Task 3): the wallet key that
//! owns a name signs an arbitrary message (hsd `signmessagewithname` parity),
//! for domain-claim verification flows such as Namebase's.
//!
//! Drives the REAL `#[tauri::command]` with a managed `AppState` over a
//! fully-migrated in-memory DB, mirroring the harness in `tx_lifecycle_tests.rs`.
//! The pure byte-level correctness (preimage + signature verification) is
//! covered separately in `noncustodial::message::tests`; these tests prove the
//! command-level plumbing: owner-only, unlock gate, per-wallet isolation.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::tx::sign_name_message;
use crate::db;
use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::AppState;

const MNEMONIC_A: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                          goose client purse deer funny hobby shrug give anxiety truly rack \
                          stand salad coach";
// A different, independently valid BIP39 mnemonic — used as wallet B's seed
// to prove per-wallet isolation (B's unlocked signer must not sign for A's name).
const MNEMONIC_B: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
                          abandon abandon abandon about";

const PROFILE_A: &str = "profileA";
const PROFILE_B: &str = "profileB";
const NAME: &str = "ecology";
const COIN_TXID: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn seed(mnemonic: &str) -> [u8; 64] {
    hd::seed_from_mnemonic(mnemonic, "").unwrap()
}

fn master(mnemonic: &str) -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed(mnemonic)).unwrap()
}

fn account_xpub(mnemonic: &str) -> String {
    let path = hd::bip44_path(Network::Main, 0, 0, 0);
    let account = master(mnemonic).derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account).to_base58check(Network::Main)
}

/// Receive address + script pubkey + compressed pubkey hex for leaf 0/0/0.
fn leaf00(mnemonic: &str) -> (String, String, String) {
    let (_sk, pk, addr) = hd::derive_address(Network::Main, &seed(mnemonic), 0, 0, 0).unwrap();
    let spk = hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

/// A fully-migrated in-memory DB with wallet profile `PROFILE_A` (from
/// `MNEMONIC_A`) owning `NAME` at leaf 0/0/0 — a `tracked_name_states` row
/// joined to a spendable `tracked_utxos` owner coin, exactly what
/// `get_name_coin` requires. Also seeds a second, unrelated profile `PROFILE_B`
/// (from `MNEMONIC_B`) with no owned names, for the isolation test.
fn seeded_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();

    let (addr, spk, pubkey) = leaf00(MNEMONIC_A);
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE_A,
        "Wallet A",
        "mnemonic_hot",
        "mainnet",
        &account_xpub(MNEMONIC_A),
        0,
        false,
    )
    .unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE_B,
        "Wallet B",
        "mnemonic_hot",
        "mainnet",
        &account_xpub(MNEMONIC_B),
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE_A).unwrap();

    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, ?3, ?4)",
        params![PROFILE_A, addr, spk, pubkey],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, 1000000, 4, 'name_control', NULL)",
        params![COIN_TXID, PROFILE_A, addr, spk],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
         VALUES (?1, ?2, 'aabbcc', 'CLOSED', ?3, 0, 100)",
        params![PROFILE_A, NAME, COIN_TXID],
    )
    .unwrap();
    conn
}

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn unlock(app: &tauri::App<tauri::test::MockRuntime>, profile_id: &str, mnemonic: &str) {
    let state = app.state::<AppState>();
    *state.signer.lock().unwrap() = Some(SignerSession::unlock(
        profile_id.to_string(),
        Network::Main,
        master(mnemonic),
        600_000,
    ));
}

const MSG: &str = "Namebase registry: I verify ownership of \"ecology\" for account #20544.";

#[tokio::test]
async fn signs_and_returns_signature_pubkey_address_for_the_owning_key() {
    let conn = seeded_conn();
    let app = app_with(conn);
    unlock(&app, PROFILE_A, MNEMONIC_A);

    let result = sign_name_message(
        app.state(),
        NAME.to_string(),
        MSG.to_string(),
        Some(PROFILE_A.to_string()),
    )
    .await
    .expect("sign_name_message succeeds for the owning, unlocked wallet");

    let signature_b64 = result["signature"].as_str().expect("signature field");
    let sig_bytes = BASE64.decode(signature_b64).expect("valid base64");
    assert_eq!(sig_bytes.len(), 64, "hsd compact signature is 64 bytes");

    // Verify against the SAME pubkey the command reports, over the exact hsd
    // preimage — proves the returned signature is genuinely usable, not just
    // well-formed.
    let pubkey_hex = result["publicKey"].as_str().expect("publicKey field");
    let pubkey_bytes = hex::decode(pubkey_hex).unwrap();
    let secp = secp256k1::Secp256k1::new();
    let pubkey = secp256k1::PublicKey::from_slice(&pubkey_bytes).unwrap();
    let sig = secp256k1::ecdsa::Signature::from_compact(&sig_bytes).unwrap();
    let hash =
        crate::noncustodial::tx::blake2b256(&crate::noncustodial::message::message_preimage(MSG));
    let msg = secp256k1::Message::from_digest(hash);
    secp.verify_ecdsa(&msg, &sig, &pubkey)
        .expect("signature verifies");

    let (expected_addr, _, expected_pubkey_hex) = leaf00(MNEMONIC_A);
    assert_eq!(result["address"], serde_json::json!(expected_addr));
    assert_eq!(pubkey_hex, expected_pubkey_hex);
}

#[tokio::test]
async fn rejects_a_name_the_wallet_does_not_own() {
    let conn = seeded_conn();
    let app = app_with(conn);
    unlock(&app, PROFILE_A, MNEMONIC_A);

    let err = sign_name_message(
        app.state(),
        "somenamewedonotown".to_string(),
        MSG.to_string(),
        Some(PROFILE_A.to_string()),
    )
    .await
    .expect_err("must reject a name with no owner coin");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
}

#[tokio::test]
async fn rejects_when_the_signer_is_locked() {
    let conn = seeded_conn();
    let app = app_with(conn);
    // No unlock() call — signer slot stays None.

    let err = sign_name_message(
        app.state(),
        NAME.to_string(),
        MSG.to_string(),
        Some(PROFILE_A.to_string()),
    )
    .await
    .expect_err("must reject with a locked signer");
    assert!(matches!(err, AppError::WalletLocked), "got {err:?}");
}

#[tokio::test]
async fn rejects_a_signer_unlocked_for_a_different_wallet_profile() {
    let conn = seeded_conn();
    let app = app_with(conn);
    // Wallet B's signer is unlocked, but the name is owned by wallet A —
    // per-wallet isolation must refuse to sign across profiles.
    unlock(&app, PROFILE_B, MNEMONIC_B);

    let err = sign_name_message(
        app.state(),
        NAME.to_string(),
        MSG.to_string(),
        Some(PROFILE_A.to_string()),
    )
    .await
    .expect_err("must reject signer/profile mismatch");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
}

#[tokio::test]
async fn rejects_when_no_wallet_profile_resolves() {
    // Empty DB — no profiles at all, so `resolve_profile` finds nothing.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    let app = app_with(conn);

    let err = sign_name_message(app.state(), NAME.to_string(), MSG.to_string(), None)
        .await
        .expect_err("must reject with no resolvable profile");
    assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
}
