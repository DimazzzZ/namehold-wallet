//! Live-node integration tests (regtest). **Skipped unless `HNS_IT_NODE_URL` is
//! set**, so the normal `cargo test` stays fast and offline.
//!
//! Run against a running regtest hsd started with `--index-address --index-tx`:
//!
//! ```sh
//! HNS_IT_NODE_URL=http://127.0.0.1:14037 HNS_IT_NODE_API_KEY=test \
//!   cargo test --manifest-path src-tauri/Cargo.toml live_node -- --nocapture --test-threads=1
//! ```
//!
//! Unlike `tx_lifecycle_tests` (mockito), these drive the REAL command layer
//! (sync → build → sign → broadcast → refresh, plus the name covenants) against
//! a real node, mining via `generatetoaddress` to fund the wallet and advance
//! auction phases. The signer is unlocked via the same test seam as the mock
//! tests (no secure window) — derived/seeded on `Network::Regtest` end-to-end so
//! signing and address encoding agree with the node.
//!
//! `--test-threads=1` is recommended: the tests share one node and mine blocks.

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::names::{
    build_bid_draft, build_open_draft, build_register_draft, build_reveal_draft,
};
use crate::commands::tx::{
    broadcast_tx_draft, build_send_hns_draft, refresh_tx_confirmations, sign_tx_draft,
    sync_wallet_state,
};
use crate::db;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{ChainSource, NodeRpcClient};
use crate::noncustodial::session::SignerSession;
use crate::AppState;

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
const PROFILE: &str = "regit1";
const NET: Network = Network::Regtest;

/// `Some((url, api_key))` when integration tests are enabled, else `None` (skip).
fn it_env() -> Option<(String, String)> {
    let url = std::env::var("HNS_IT_NODE_URL").ok().filter(|s| !s.trim().is_empty())?;
    let key = std::env::var("HNS_IT_NODE_API_KEY").unwrap_or_default();
    Some((url, key))
}

fn seed() -> [u8; 64] {
    hd::seed_from_mnemonic(MNEMONIC, "").unwrap()
}
fn master() -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed()).unwrap()
}

/// Account xpub (m/44'/coin'/0') encoded for regtest.
fn account_xpub() -> String {
    let path = hd::bip44_path(NET, 0, 0, 0);
    let account = master().derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account).to_base58check(NET)
}

/// Receive address + script-pubkey hex + pubkey hex for leaf 0/0 on regtest.
fn leaf00() -> (String, String, String) {
    let (_sk, pk, addr) = hd::derive_address(NET, &seed(), 0, 0, 0).unwrap();
    let spk = hex::encode(crate::noncustodial::address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

/// Migrate + seed a regtest profile owning leaf 0/0. No pre-seeded UTXO — the
/// wallet is funded by mining to its address, then `sync_wallet_state`.
fn seeded_conn_regtest(url: &str, api_key: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();

    let (addr, spk, pubkey) = leaf00();
    db::queries::insert_wallet_profile(
        &conn, PROFILE, "RegIT", "mnemonic_hot", "regtest", &account_xpub(), 0, false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", url).unwrap();
    db::queries::set_setting(&conn, "node_rpc_api_key", api_key).unwrap();

    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, 0, 0, 0, ?2, ?3, ?4)",
        params![PROFILE, addr, spk, pubkey],
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
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn unlock(app: &tauri::App<tauri::test::MockRuntime>) {
    let state = app.state::<AppState>();
    *state.signer.lock().unwrap() =
        Some(SignerSession::unlock(PROFILE.to_string(), NET, master(), 600_000));
}

fn client(url: &str, key: &str) -> NodeRpcClient {
    NodeRpcClient::new(url, key, ChainSource::LocalNode)
}

fn draft_status(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> db::queries::TxDraftRow {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    db::queries::get_tx_draft(&c, id).unwrap().unwrap()
}

/// The node's view of a name's auction state (`getnameinfo` → `info.state`).
async fn node_state(cl: &NodeRpcClient, name: &str) -> Option<String> {
    let raw = cl.get_name_info(name).await.ok()?;
    raw.get("info")?
        .get("state")?
        .as_str()
        .map(|s| s.to_string())
}

/// Mine up to `max_blocks` (a few at a time) to `addr` until the name reaches
/// `target` state. Returns true if reached.
async fn mine_until(
    cl: &NodeRpcClient,
    name: &str,
    target: &str,
    addr: &str,
    max_blocks: u32,
) -> bool {
    let mut mined = 0;
    loop {
        if node_state(cl, name).await.as_deref() == Some(target) {
            return true;
        }
        if mined >= max_blocks {
            return false;
        }
        cl.generate_to_address(2, addr).await.expect("mine");
        mined += 2;
    }
}

/// Build → unlock → sign → broadcast → mine 1, returning the draft id.
async fn execute(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    draft_id: String,
) {
    unlock(app);
    sign_tx_draft(app.state(), draft_id.clone()).await.expect("sign");
    let bc = broadcast_tx_draft(app.state(), draft_id.clone()).await.expect("broadcast");
    assert_eq!(bc.status, "broadcasted");
    cl.generate_to_address(1, addr).await.expect("mine 1");
}

#[tokio::test]
async fn live_send_builds_broadcasts_and_confirms() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_builds_broadcasts_and_confirms: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    // Fund: mine past coinbase maturity to the wallet's address.
    cl.generate_to_address(101, &addr).await.expect("fund");

    let sync = sync_wallet_state(app.state(), None).await.expect("sync");
    assert_eq!(sync["nodeReachable"], serde_json::json!(true), "{sync}");
    assert!(
        sync["utxoCount"].as_i64().unwrap_or(0) >= 1,
        "wallet must see coinbase coins after sync: {sync}"
    );

    // Send a small amount to self.
    let draft = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build send");
    execute(&app, &cl, &addr, draft.id.clone()).await;

    // The confirmation refresh advances broadcasted → confirmed with a height.
    let r = refresh_tx_confirmations(app.state(), None).await.expect("refresh");
    assert_eq!(r["confirmed"], serde_json::json!(1), "{r}");
    let row = draft_status(&app, &draft.id);
    assert_eq!(row.status, "confirmed");
    assert!(row.confirmation_height.is_some(), "confirmed draft records a height");
}

#[tokio::test]
async fn live_auction_open_bid_reveal_register() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_auction_open_bid_reveal_register: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    // Fund the wallet.
    cl.generate_to_address(101, &addr).await.expect("fund");
    sync_wallet_state(app.state(), None).await.expect("sync");

    // A per-run-unique name (avoid collisions with names already on the node).
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("cuait{tip}");

    // OPEN → advance to BIDDING.
    let open = build_open_draft(app.state(), name.clone(), Some(1)).await.expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(
        mine_until(&cl, &name, "BIDDING", &addr, 30).await,
        "name {name} did not reach BIDDING; state={:?}",
        node_state(&cl, &name).await
    );

    // BID → advance to REVEAL. Re-sync first so the change coin is spendable.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.clone(), 1_000_000, 2_000_000, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    assert!(
        mine_until(&cl, &name, "REVEAL", &addr, 30).await,
        "name {name} did not reach REVEAL; state={:?}",
        node_state(&cl, &name).await
    );

    // REVEAL → advance to CLOSED.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1)).await.expect("build reveal");
    execute(&app, &cl, &addr, reveal.id).await;
    assert!(
        mine_until(&cl, &name, "CLOSED", &addr, 40).await,
        "name {name} did not reach CLOSED; state={:?}",
        node_state(&cl, &name).await
    );

    // REGISTER the won name with a DNS record (proves the post-auction path).
    // Owned-name discovery is normally explorer-based (unavailable on regtest),
    // so seed the name as tracked; the sync then resolves its owner coin from
    // the node's getnameinfo, exactly as discovery would on mainnet.
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.execute(
            "INSERT OR IGNORE INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state)
             VALUES (?1, ?2, '', 'UNKNOWN')",
            params![PROFILE, name],
        )
        .unwrap();
    }
    sync_wallet_state(app.state(), None).await.expect("sync");
    let records = vec![serde_json::json!({"type":"TXT","txt":["cua-agent-verified"]})];
    let reg = build_register_draft(app.state(), name.clone(), Some(records), Some(1))
        .await
        .expect("build register");
    execute(&app, &cl, &addr, reg.id).await;

    // Final state is CLOSED (registered names stay CLOSED on-chain).
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
}
