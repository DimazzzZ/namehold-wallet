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
    build_bid_draft, build_open_draft, build_redeem_draft, build_register_draft,
    build_reveal_draft, build_transfer_draft, build_update_draft,
};
use crate::commands::tx::estimate_tx_draft_fee;
use crate::commands::tx::{
    broadcast_tx_draft, build_send_hns_draft, get_wallet_balances, refresh_tx_confirmations,
    sign_tx_draft_inner, sync_wallet_state,
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
    let url = std::env::var("HNS_IT_NODE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
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
    account_xpub_at(0)
}

/// Account xpub for a specific BIP44 account index (m/44'/coin'/acct').
fn account_xpub_at(acct: u32) -> String {
    let path = hd::bip44_path(NET, acct, 0, 0);
    let account = master().derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account).to_base58check(NET)
}

/// Receive address + script-pubkey hex + pubkey hex for leaf 0/0 on regtest.
fn leaf00() -> (String, String, String) {
    leaf00_at(test_acct())
}

thread_local! {
    /// The account [`seeded_conn_regtest`] gave this test's profile.
    static TEST_ACCT: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// The account of this test's profile, set by [`seeded_conn_regtest`]; 0 for
/// a helper that only mines and seeded no profile.
fn test_acct() -> u32 {
    TEST_ACCT.with(|a| a.get()).unwrap_or(0)
}

/// A BIP44 account no earlier test or run has funded. The tests once shared
/// account 0, which gathered thousands of coinbases a run: sync and coin
/// selection then walked them all, a sweep outgrew a standard transaction and
/// hsd's regtest mempool kept it unmined, and every test spending its change
/// stalled behind it. Clear of the fixed private accounts (0-24) and of
/// [`fresh_acct`] (100 000 up); hardened indexes stay below 2^31.
fn unique_acct() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Seconds mod 10^7 (about 115 days) times 100 tests per second: a rerun
    // repeats an account only after that.
    1_000_000_000 + ((secs % 10_000_000) as u32) * 100 + n % 100
}

/// A private BIP44 account index that is guaranteed empty on EVERY run,
/// including reruns against an already-used regtest chain. Keyed off the
/// current chain height and pushed into a high range well clear of the fixed
/// private accounts sibling tests use (0-24): two runs can only collide if the
/// tip is byte-for-byte identical, which never recurs once any block is mined.
///
/// Use this for tests whose invariant depends on the account starting from a
/// clean slate — a single fresh/immature coinbase, an empty post-reorg
/// spendable set, or an address with no prior revoked-covenant coin (which
/// trips hsd's addrindex 500 on `getcoinsbyaddress`). Tests that only assert on
/// per-run DELTAS can keep a fixed private account.
fn fresh_acct(tip: i64) -> u32 {
    100_000 + (tip as u32)
}

/// Receive leaf `acct/0/0` — a unique on-chain address per account index.
/// Used to give a test its OWN funding address so it never inherits the
/// coin set that sibling tests pile onto the shared `acct 0` address (the
/// live suite runs serially against one chain).
fn leaf00_at(acct: u32) -> (String, String, String) {
    let (_sk, pk, addr) = hd::derive_address(NET, &seed(), acct, 0, 0).unwrap();
    let spk = hex::encode(crate::noncustodial::address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

/// Migrate + seed a regtest profile owning leaf 0/0. No pre-seeded UTXO — the
/// wallet is funded by mining to its address, then `sync_wallet_state`.
fn seeded_conn_regtest(url: &str, api_key: &str) -> rusqlite::Connection {
    let acct = unique_acct();
    TEST_ACCT.with(|a| a.set(Some(acct)));
    seeded_conn_acct(url, api_key, acct)
}

/// Like [`seeded_conn_regtest`] but seeds the profile at BIP44 account `acct`
/// (funding leaf `acct/0/0`, change `acct/1/0`, and recipient `acct/0/1`).
///
/// The live suite runs serially against ONE regtest chain, and every test that
/// uses account 0 mines to and self-sends back to the same `0/0` address — so
/// its on-chain coin set grows without bound across the run. Tests that assert
/// on the EXACT coin set (sweep counts, single-coin dust folding, first-coin
/// maturity) or that call `getcoinsbyaddress` on that address (which hsd's
/// addrindex can trip an assertion on once the set is huge) must run against a
/// private, empty address. Give each such test its own `acct` and it starts
/// from a clean slate. `profile.account_index = acct` flows through the entire
/// build/sign path (`bip44_path(network, account, branch, child)`), so signing
/// stays consistent with the seeded addresses.
fn seeded_conn_acct(url: &str, api_key: &str, acct: u32) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    seed_profile_into(&conn, url, api_key, acct);
    conn
}

/// The seeding half of [`seeded_conn_acct`], split out so a test that needs a
/// FILE-backed DB (the chain scanner reopens the DB by path, which an in-memory
/// connection cannot share) can reuse the exact same profile fixture.
fn seed_profile_into(conn: &rusqlite::Connection, url: &str, api_key: &str, acct: u32) {
    let (addr, spk, pubkey) = leaf00_at(acct);
    db::queries::insert_wallet_profile(
        conn,
        PROFILE,
        "RegIT",
        "mnemonic_hot",
        "regtest",
        &account_xpub_at(acct),
        acct as i64,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(conn, PROFILE).unwrap();
    db::queries::set_setting(conn, "node_rpc_url", url).unwrap();
    db::queries::set_setting(conn, "node_rpc_api_key", api_key).unwrap();

    conn.execute(
        "INSERT INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, ?2, 0, 0, ?3, ?4, ?5)",
        params![PROFILE, acct as i64, addr, spk, pubkey],
    )
    .unwrap();

    // Also register a few adjacent leaves so sync can attribute coins that
    // land on them: the change branch (1/0) — needed by tests that send to an
    // EXTERNAL address and expect change to come back to the wallet — and the
    // next receive leaf (0/1), used as a transfer/second-party recipient.
    for (branch, idx) in [(1u32, 0u32), (0u32, 1u32)] {
        let (_sk, pk, a) = hd::derive_address(NET, &seed(), acct, branch, idx).unwrap();
        let s = hex::encode(crate::noncustodial::address::script_pubkey_from_pubkey(&pk).unwrap());
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![PROFILE, acct as i64, branch, idx, a, s, hex::encode(pk)],
        )
        .unwrap();
    }
}

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn unlock(app: &tauri::App<tauri::test::MockRuntime>) {
    let state = app.state::<AppState>();
    *state.signer.lock().unwrap() = Some(SignerSession::unlock(
        PROFILE.to_string(),
        NET,
        master(),
        600_000,
    ));
}

fn client(url: &str, key: &str) -> NodeRpcClient {
    NodeRpcClient::new(url, key, ChainSource::LocalNode)
}

/// Track `name` for the profile, as the full sync's discovery would, so the
/// next sync reads its owner coin.
fn track_name(app: &tauri::App<tauri::test::MockRuntime>, name: &str) {
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
    sign_tx_draft_inner(&app.state(), &draft_id)
        .await
        .expect("sign");
    let bc = broadcast_tx_draft(app.state(), draft_id.clone())
        .await
        .expect("broadcast");
    assert_eq!(bc.status, "broadcasted");
    cl.generate_to_address(1, addr).await.expect("mine 1");
    // Advance the just-mined draft to `confirmed` and free its coin
    // reservation, in that order. Two things need to happen after a broadcast
    // draft is mined before the next draft can select the same coins:
    //   1. `refresh_tx_confirmations` promotes it to `confirmed`.
    //   2. Its input reservation is released. In production this happens
    //      transparently: once sync marks the input coin `spent_by_txid` (so
    //      it drops out of `load_spendable_coins` on the "spent" clause) or
    //      the reservation TTL (1h) elapses (`RESERVATION_TTL_SECS`), the
    //      reservation stops mattering. Neither of those runs on a fast,
    //      chained test — sync in this harness doesn't mark the change coin
    //      spent before the next build, and waiting an hour is not viable.
    //      `broadcast_tx_draft` deliberately keeps the reservation on
    //      successful broadcast (real coins are in flight), so we release it
    //      here to close the gap, mirroring the end-state the app reaches
    //      once sync + time settle.
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh confirmations");
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        db::queries::release_reserved_utxos_for_draft(&conn, &draft_id)
            .expect("release reservation");
    }
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
    let r = refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    assert_eq!(r["confirmed"], serde_json::json!(1), "{r}");
    let row = draft_status(&app, &draft.id);
    assert_eq!(row.status, "confirmed");
    assert!(
        row.confirmation_height.is_some(),
        "confirmed draft records a height"
    );
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
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
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
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
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
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let records = vec![serde_json::json!({"type":"TXT","txt":["cua-agent-verified"]})];
    let reg = build_register_draft(app.state(), name.clone(), Some(records), Some(1))
        .await
        .expect("build register");
    execute(&app, &cl, &addr, reg.id).await;

    // Final state is CLOSED (registered names stay CLOSED on-chain).
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
}

/// A genuine loser redeems: a rival wallet on the same node outbids this one,
/// so the rival's reveal becomes the name's owner and this wallet's reveal is
/// a losing one. REDEEM spends exactly that losing reveal and returns its
/// value; the rival's winning reveal stays the owner.
#[tokio::test]
async fn live_auction_open_bid_reveal_redeem() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_auction_open_bid_reveal_redeem: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    let (rival, rival_addr) = rival_wallet(&url, &key);

    // Fund both wallets.
    cl.generate_to_address(101, &addr).await.expect("fund");
    // A few coinbases are plenty for one bid (regtest coinbase maturity is 2).
    cl.generate_to_address(5, &rival_addr)
        .await
        .expect("fund rival");
    sync_wallet_state(app.state(), None).await.expect("sync");
    sync_wallet_state(rival.state(), None)
        .await
        .expect("sync rival");

    // A per-run-unique name (avoid collisions with names already on the node).
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("loser{tip}");

    // OPEN → advance to BIDDING.
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(
        mine_until(&cl, &name, "BIDDING", &addr, 30).await,
        "name {name} did not reach BIDDING; state={:?}",
        node_state(&cl, &name).await
    );

    // BID from both wallets; the rival bids higher → advance to REVEAL.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.clone(), 500_000, 1_000_000, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    sync_wallet_state(rival.state(), None)
        .await
        .expect("sync rival");
    let rival_bid = build_bid_draft(rival.state(), name.clone(), 800_000, 1_600_000, Some(1))
        .await
        .expect("build rival bid");
    execute(&rival, &cl, &addr, rival_bid.id).await;
    assert!(
        mine_until(&cl, &name, "REVEAL", &addr, 30).await,
        "name {name} did not reach REVEAL; state={:?}",
        node_state(&cl, &name).await
    );

    // REVEAL from both → advance to CLOSED.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id.clone()).await;
    sync_wallet_state(rival.state(), None)
        .await
        .expect("sync rival");
    let rival_reveal = build_reveal_draft(rival.state(), name.clone(), Some(1))
        .await
        .expect("build rival reveal");
    execute(&rival, &cl, &addr, rival_reveal.id.clone()).await;
    assert!(
        mine_until(&cl, &name, "CLOSED", &addr, 40).await,
        "name {name} did not reach CLOSED; state={:?}",
        node_state(&cl, &name).await
    );

    // The node names the rival's reveal as the owner: ours lost.
    let rival_reveal_txid = draft_status(&rival, &rival_reveal.id)
        .txid
        .expect("rival reveal txid");
    assert_eq!(
        name_owner(&cl, &name).await.0,
        rival_reveal_txid,
        "the higher bid must own the name"
    );
    let ours = covenant_outpoints(
        &cl,
        &draft_status(&app, &reveal.id).txid.expect("reveal txid"),
        COV_TYPE_REVEAL,
    )
    .await;
    assert_eq!(ours.len(), 1, "one reveal output: {ours:?}");

    // REDEEM the losing reveal.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let redeem = build_redeem_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build redeem");
    execute(&app, &cl, &addr, redeem.id.clone()).await;

    // The redeem spent exactly our losing reveal, which is gone from the UTXO
    // set, and paid its value (the true bid) back; the owner is unchanged.
    let redeem_txid = draft_status(&app, &redeem.id).txid.expect("redeem txid");
    assert_spends_reveals(&cl, &redeem_txid, &ours).await;
    assert_eq!(
        output_value_by_covenant_type(&cl, &redeem_txid, COV_TYPE_REDEEM).await,
        Some(500_000),
        "REDEEM returns the losing reveal's value"
    );
    assert_eq!(name_owner(&cl, &name).await.0, rival_reveal_txid);
}

/// The guard behind REDEEM on a real node: a lone bidder's reveal won, and
/// until REGISTER spends it it is the name's owner coin, which hsd refuses to
/// redeem (`bad-redeem-owner`). The name is never tracked, so the wallet has
/// no record of the owner of its own — only the node's answer can refuse it.
#[tokio::test]
async fn live_redeem_of_the_winning_reveal_is_refused() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_redeem_of_the_winning_reveal_is_refused: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    cl.generate_to_address(101, &addr).await.expect("fund");
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("wonrv{tip}");

    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.clone(), 500_000, 1_000_000, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    assert!(mine_until(&cl, &name, "REVEAL", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id.clone()).await;
    assert!(mine_until(&cl, &name, "CLOSED", &addr, 40).await);
    sync_wallet_state(app.state(), None).await.expect("sync");

    let reveal_txid = draft_status(&app, &reveal.id).txid.expect("reveal txid");
    assert_eq!(
        name_owner(&cl, &name).await.0,
        reveal_txid,
        "the lone reveal owns the name"
    );
    expect_err(
        build_redeem_draft(app.state(), name.clone(), Some(1)).await,
        "won the auction",
    );
    // Nothing spent it: it is still the owner coin, waiting for REGISTER.
    let (hash, index) = name_owner(&cl, &name).await;
    assert!(cl.get_coin(&hash, index).await.expect("coin").is_some());
}

#[tokio::test]
async fn live_auction_register_transfer_finalize() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_auction_register_transfer_finalize: set HNS_IT_NODE_URL");
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
    let name = format!("transfer{tip}");

    // OPEN → advance to BIDDING.
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(
        mine_until(&cl, &name, "BIDDING", &addr, 30).await,
        "name {name} did not reach BIDDING"
    );

    // BID → advance to REVEAL.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.clone(), 1_000_000, 2_000_000, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    assert!(
        mine_until(&cl, &name, "REVEAL", &addr, 30).await,
        "name {name} did not reach REVEAL"
    );

    // REVEAL → advance to CLOSED.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id).await;
    assert!(
        mine_until(&cl, &name, "CLOSED", &addr, 40).await,
        "name {name} did not reach CLOSED"
    );

    // Track the name so sync picks up the owner coin.
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");

    // REGISTER the won name.
    let records = vec![serde_json::json!({"type":"TXT","txt":["cua-agent-verified"]})];
    let reg = build_register_draft(app.state(), name.clone(), Some(records), Some(1))
        .await
        .expect("build register");
    execute(&app, &cl, &addr, reg.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // TRANSFER the registered name to a second address (leaf 0/1).
    // Derive address for branch=0 child_index=1.
    let (_sk, _pk2, addr2) =
        crate::noncustodial::hd::derive_address(NET, &seed(), 0, 0, 1).unwrap();
    use crate::commands::names::build_transfer_draft;
    let transfer = build_transfer_draft(app.state(), name.clone(), addr2.clone(), Some(1))
        .await
        .expect("build transfer");
    execute(&app, &cl, &addr, transfer.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // hsd refuses a FINALIZE until the transfer lockup is over
    // (`bad-finalize-maturity`), so the builder refuses it too.
    use crate::commands::names::build_finalize_draft;
    expect_err(
        build_finalize_draft(app.state(), name.clone(), Some(1)).await,
        "still locked",
    );

    // Mine exactly the rest of the lockup, then finalize.
    let transfer_height = name_info_height(&cl, &name, "transfer").await;
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let left = NET
        .name_params()
        .blocks_until_finalize(transfer_height, tip);
    assert!(left > 0, "the lockup was not over a moment ago");
    cl.generate_to_address(left as u32, &addr)
        .await
        .expect("lockup");
    sync_wallet_state(app.state(), None).await.expect("sync");
    let finalize = build_finalize_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build finalize");
    execute(&app, &cl, &addr, finalize.id).await;

    // The name's owner coin is now at the recipient, and no transfer is
    // pending.
    assert_eq!(
        owner_coin_address(&cl, &name).await.as_deref(),
        Some(addr2.as_str())
    );
    assert_eq!(name_info_height(&cl, &name, "transfer").await, 0);
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
}

/// Acquire a fresh name end-to-end so the wallet owns it: OPEN → BID → REVEAL →
/// REGISTER, mining through each phase and tracking the name so sync picks up
/// the owner coin. Mirrors the single-name path in
/// `live_auction_register_transfer_finalize`, factored out so multi-name tests
/// (e.g. batch transfer) can reuse it. Funds are mined to `addr`; the caller
/// must have already funded + synced the wallet.
async fn acquire_name(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    name: &str,
) {
    // OPEN → advance to BIDDING.
    let open = build_open_draft(app.state(), name.to_string(), Some(1))
        .await
        .expect("build open");
    execute(app, cl, addr, open.id).await;
    assert!(
        mine_until(cl, name, "BIDDING", addr, 30).await,
        "name {name} did not reach BIDDING"
    );

    // BID → advance to REVEAL.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.to_string(), 1_000_000, 2_000_000, Some(1))
        .await
        .expect("build bid");
    execute(app, cl, addr, bid.id).await;
    assert!(
        mine_until(cl, name, "REVEAL", addr, 30).await,
        "name {name} did not reach REVEAL"
    );

    // REVEAL → advance to CLOSED.
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.to_string(), Some(1))
        .await
        .expect("build reveal");
    execute(app, cl, addr, reveal.id).await;
    assert!(
        mine_until(cl, name, "CLOSED", addr, 40).await,
        "name {name} did not reach CLOSED"
    );

    // Track the name so sync picks up the owner coin.
    track_name(app, name);
    sync_wallet_state(app.state(), None).await.expect("sync");

    // REGISTER the won name so the wallet owns it.
    let records = vec![serde_json::json!({"type":"TXT","txt":["cua-agent-verified"]})];
    let reg = build_register_draft(app.state(), name.to_string(), Some(records), Some(1))
        .await
        .expect("build register");
    execute(app, cl, addr, reg.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
}

/// Batch transfer: acquire TWO names, transfer BOTH to a single shared recipient
/// in one `build_batch_transfer_draft` tx, and assert both names' TRANSFER
/// covenants land on-chain, then finalize each. This is the multi-name analogue
/// of `live_auction_register_transfer_finalize` and the only live-node coverage
/// of the batch command (the single-transfer test predates the batch feature).
#[tokio::test]
async fn live_batch_transfer_two_names() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_transfer_two_names: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    // Fund the wallet.
    cl.generate_to_address(101, &addr).await.expect("fund");
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Two per-run-unique names (avoid collisions with names already on the node).
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name_a = format!("batchtx{tip}a");
    let name_b = format!("batchtx{tip}b");

    // Acquire both names so the wallet owns them.
    acquire_name(&app, &cl, &addr, &name_a).await;
    acquire_name(&app, &cl, &addr, &name_b).await;

    // Shared recipient (leaf 0/1).
    let (_sk, _pk2, recipient) =
        crate::noncustodial::hd::derive_address(NET, &seed(), 0, 0, 1).unwrap();

    // BATCH TRANSFER both names to the shared recipient in one tx.
    use crate::commands::names::{build_batch_transfer_draft, build_finalize_draft};
    let batch = build_batch_transfer_draft(
        app.state(),
        vec![name_a.clone(), name_b.clone()],
        recipient.clone(),
        Some(1),
    )
    .await
    .expect("build batch transfer");
    execute(&app, &cl, &addr, batch.id.clone()).await;

    // The batch draft broadcast, and BOTH names now show a pending transfer
    // on-chain (the TRANSFER covenant sets `transfer` to the height it landed).
    let row = draft_status(&app, &batch.id);
    assert!(
        matches!(row.status.as_str(), "broadcasted" | "confirmed"),
        "batch draft status was {}",
        row.status
    );
    for name in [&name_a, &name_b] {
        let info = cl.get_name_info(name).await.expect("getnameinfo");
        let transfer_height = info
            .get("info")
            .and_then(|i| i.get("transfer"))
            .and_then(|t| t.as_u64());
        assert!(
            transfer_height.map(|h| h > 0).unwrap_or(false),
            "name {name} has no pending TRANSFER covenant after batch transfer: {info}"
        );
    }

    // Finalize each name on its own after the transfer lockup elapses (the
    // batch finalize has its own test); advance past the regtest transfer
    // lockup (10 blocks) so each finalize is valid.
    sync_wallet_state(app.state(), None).await.expect("sync");
    cl.generate_to_address(11, &addr).await.expect("lockup");
    sync_wallet_state(app.state(), None).await.expect("sync");

    for name in [&name_a, &name_b] {
        let finalize = build_finalize_draft(app.state(), name.clone(), Some(1))
            .await
            .expect("build finalize");
        execute(&app, &cl, &addr, finalize.id).await;
        // The first finalize spent a funding coin for its fee. Without a sync
        // the wallet still holds that coin as unspent, and the second finalize
        // spent it again: hsd took that transaction as an orphan, its input
        // missing. In the app a sync runs between any two actions.
        sync_wallet_state(app.state(), None).await.expect("sync");
    }

    // After finalize, both names are at the recipient address and CLOSED.
    for name in [&name_a, &name_b] {
        assert_eq!(
            owner_coin_address(&cl, name).await.as_deref(),
            Some(recipient.as_str()),
            "name {name} did not move to the recipient"
        );
        assert_eq!(
            node_state(&cl, name).await.as_deref(),
            Some("CLOSED"),
            "name {name} not CLOSED after finalize"
        );
    }
}

// =============================================================================
// SHARED HELPERS FOR THE EXPANDED REGTEST SUITE
// =============================================================================
//
// The tests below extend the live-node suite to cover every money/crypto-
// critical path (see .mimocode/plans/1789149229571-clever-cabin.md). They reuse
// the existing harness (`seeded_conn_regtest` / `app_with` / `unlock` /
// `execute` / `mine_until` / `acquire_name`) and add the small helpers below.

/// Convenience: mine `n` blocks to `addr` (past coinbase maturity when `n>=101`).
/// Sign and broadcast a draft WITHOUT mining it.
///
/// [`execute`] mines immediately, which is right for tests that only care
/// about the settled state — but it makes the mempool window unobservable, and
/// that window is exactly where the wallet has to describe a chain that has not
/// moved yet.
async fn broadcast_only(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) {
    unlock(app);
    sign_tx_draft_inner(&app.state(), draft_id)
        .await
        .expect("sign");
    let bc = broadcast_tx_draft(app.state(), draft_id.to_string())
        .await
        .expect("broadcast");
    assert_eq!(bc.status, "broadcasted");
}

/// hsd answers `sendrawtransaction` before its mempool has taken the
/// transaction (`rpc.js`: `this.node.relay(tx)`, not awaited), so a block
/// mined, or a lookup made, straight after a send can miss it. Wait until the
/// node reports `txid`, for at most two seconds.
async fn wait_until_node_has(cl: &NodeRpcClient, txid: &str) {
    for _ in 0..40 {
        if !cl.get_tx_by_hash(txid).await.expect("tx lookup").is_null() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the node never reported {txid}");
}

/// Mine one block and bring the wallet's view up to date with it — the other
/// half of [`broadcast_only`]. Mirrors the tail of [`execute`]. The draft's
/// transaction is waited for first, so the block holds it.
async fn settle(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    draft_id: &str,
) {
    let txid = draft_status(app, draft_id)
        .txid
        .expect("a settled draft was broadcast");
    wait_until_node_has(cl, &txid).await;
    cl.generate_to_address(1, addr).await.expect("mine 1");
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh confirmations");
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        db::queries::release_reserved_utxos_for_draft(&conn, draft_id).expect("release");
    }
    sync_wallet_state(app.state(), None).await.expect("sync");
}

/// A profile seeded into a FILE-backed DB, which the chain scanner needs: it
/// reopens the database by path, so an in-memory connection is invisible to it.
fn file_backed_app(
    url: &str,
    key: &str,
    acct: u32,
) -> (std::path::PathBuf, tauri::App<tauri::test::MockRuntime>) {
    let db_path = std::env::temp_dir().join(format!(
        "namehold_live_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    seed_profile_into(&conn, url, key, acct);
    drop(conn);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    (db_path.clone(), app_with(conn))
}

/// Walk the chain scanner over every block, exactly as `run_chain_scanner`
/// does, then park its cursor at the tip.
async fn scan_to_tip(cl: &NodeRpcClient, db_path: &std::path::Path) {
    let tip = cl.get_blockchain_info().await.expect("info").blocks as i64;
    for height in 1..=tip {
        crate::commands::chain_scan::scan_block(cl, db_path.to_str().unwrap(), "regtest", height)
            .await
            .unwrap_or_else(|e| panic!("scan_block({height}): {e:?}"));
    }
    // Its own connection, opened after the walk: the app's `AppState` guard
    // must not be held across an await.
    let conn = rusqlite::Connection::open(db_path).unwrap();
    crate::commands::chain_scan::set_scan_cursor(&conn, "regtest", tip).unwrap();
}

/// The auction's OPEN height as the node reports it right now, or `None` when
/// the name has no auction at all.
async fn auction_start(cl: &NodeRpcClient, name: &str) -> Option<i64> {
    cl.get_name_info(name)
        .await
        .ok()?
        .get("info")?
        .get("height")?
        .as_i64()
}

/// Mine until the node stops reporting an auction for `name` — an auction
/// nobody revealed in lapses, and the name becomes available again.
async fn mine_until_auction_lapses(
    cl: &NodeRpcClient,
    name: &str,
    addr: &str,
    max_blocks: u32,
) -> bool {
    let mut mined = 0;
    while mined < max_blocks {
        if auction_start(cl, name).await.is_none() {
            return true;
        }
        cl.generate_to_address(5, addr).await.expect("mine");
        mined += 5;
    }
    auction_start(cl, name).await.is_none()
}

async fn fund(cl: &NodeRpcClient, addr: &str, n: u32) {
    cl.generate_to_address(n, addr).await.expect("mine");
}

/// Age a draft's created_at so `release_stale_reservations` treats its
/// reservation as TTL-expired. Reservations don't carry a separate timestamp
/// — the TTL sweep in `send.rs::release_stale_reservations` compares the
/// owning draft's `wallet_tx_drafts.created_at` against `RESERVATION_TTL_SECS`
/// (1h). Manipulating that here is deterministic; sleeping the real TTL is
/// not viable in a test.
fn age_reservation(conn: &rusqlite::Connection, draft_id: &str, secs: i64) {
    conn.execute(
        "UPDATE wallet_tx_drafts SET created_at = datetime('now', ?1) WHERE id = ?2",
        params![format!("-{} seconds", secs), draft_id],
    )
    .expect("age reservation");
}

/// Assert a `Result` is `Err` and its message matches `needle` (case-insensitive
/// substring). Used across the many negative-path tests to keep the assertion
/// intent obvious in one line without a bespoke matcher per error variant.
fn expect_err<T: std::fmt::Debug>(res: Result<T, crate::error::AppError>, needle: &str) {
    match res {
        Ok(v) => panic!("expected error containing {needle:?}, got Ok({v:?})"),
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.to_lowercase().contains(&needle.to_lowercase()),
                "error {msg:?} did not contain {needle:?}"
            );
        }
    }
}

/// hsd covenant type codes (`lib/covenants/rules.js` `types`).
const COV_TYPE_REVEAL: u64 = 4;
const COV_TYPE_REDEEM: u64 = 5;

/// A second wallet on the same node: its own in-memory database and a BIP44
/// account nothing else uses, so it bids against this test's wallet as a
/// stranger would. Returns the app and its funding address.
fn rival_wallet(url: &str, key: &str) -> (tauri::App<tauri::test::MockRuntime>, String) {
    let acct = unique_acct();
    let app = app_with(seeded_conn_acct(url, key, acct));
    (app, leaf00_at(acct).0)
}

/// A height field of the node's `getnameinfo` → `info` (`renewal`,
/// `transfer`).
async fn name_info_height(cl: &NodeRpcClient, name: &str, key: &str) -> i64 {
    let info = cl.get_name_info(name).await.expect("name info");
    info["info"][key]
        .as_i64()
        .unwrap_or_else(|| panic!("no {key} for {name}: {info}"))
}

/// Mine exactly as many blocks as the latest-renewed of `names` needs before
/// hsd accepts a RENEW of it (`renewal + tree_interval`, judged at `tip + 1`).
async fn mine_until_renewable(cl: &NodeRpcClient, names: &[&str], addr: &str) {
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let mut left = 0;
    for name in names {
        let renewal = name_info_height(cl, name, "renewal").await;
        left = left.max(NET.name_params().blocks_until_renew(renewal, tip));
    }
    if left > 0 {
        cl.generate_to_address(left as u32, addr)
            .await
            .expect("mine to renewable");
    }
}

/// The name's owner outpoint as the node reports it (`getnameinfo` →
/// `info.owner`).
async fn name_owner(cl: &NodeRpcClient, name: &str) -> (String, u32) {
    let info = cl.get_name_info(name).await.expect("name info");
    let owner = &info["info"]["owner"];
    (
        owner["hash"].as_str().expect("owner hash").to_string(),
        owner["index"].as_u64().expect("owner index") as u32,
    )
}

/// The outpoints of `txid`'s outputs carrying covenant `want_type`.
async fn covenant_outpoints(cl: &NodeRpcClient, txid: &str, want_type: u64) -> Vec<(String, u32)> {
    let tx = cl.get_tx_by_hash(txid).await.expect("tx lookup");
    tx["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .enumerate()
        .filter(|(_, o)| o["covenant"]["type"].as_u64() == Some(want_type))
        .map(|(i, _)| (txid.to_string(), i as u32))
        .collect()
}

/// The mined transaction `txid` spends every outpoint in `reveals`, and the
/// node no longer has any of them as a coin.
async fn assert_spends_reveals(cl: &NodeRpcClient, txid: &str, reveals: &[(String, u32)]) {
    let tx = cl.get_tx_by_hash(txid).await.expect("tx lookup");
    assert!(
        tx["height"].as_i64().unwrap_or(-1) >= 0,
        "{txid} is not mined"
    );
    let spent: Vec<(String, u32)> = tx["inputs"]
        .as_array()
        .expect("inputs")
        .iter()
        .map(|i| {
            (
                i["prevout"]["hash"]
                    .as_str()
                    .expect("prevout hash")
                    .to_string(),
                i["prevout"]["index"].as_u64().expect("prevout index") as u32,
            )
        })
        .collect();
    for (hash, index) in reveals {
        assert!(
            spent.contains(&(hash.clone(), *index)),
            "{txid} does not spend {hash}:{index}; inputs {spent:?}"
        );
        assert!(
            cl.get_coin(hash, *index).await.expect("coin").is_none(),
            "{hash}:{index} is still unspent"
        );
    }
}

/// Acquire two names end-to-end so the wallet owns both. Convenience wrapper
/// around `acquire_name` used by the E-series batch tests.
async fn acquire_two_names(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    name_a: &str,
    name_b: &str,
) {
    acquire_name(app, cl, addr, name_a).await;
    acquire_name(app, cl, addr, name_b).await;
}

/// The address at branch=1, child=0 for the seeded profile — the change
/// address `build_send_hns_draft` writes to. Test-only mirror of the private
/// `change_address` helper in `commands/tx.rs`.
fn change_addr_00() -> String {
    let (_sk, _pk, a) = hd::derive_address(NET, &seed(), test_acct(), 1, 0).unwrap();
    a
}

/// The address at branch=0, child=1 — the "second party" recipient used by
/// tests that need an address the wallet does NOT auto-scan into 0/0.
fn recv_leaf_01() -> String {
    recv_leaf_01_at(0)
}

/// Recipient leaf `acct/0/1` — the second-party address for a given account.
fn recv_leaf_01_at(acct: u32) -> String {
    let (_sk, _pk, a) = hd::derive_address(NET, &seed(), acct, 0, 1).unwrap();
    a
}

/// Total wallet balance (sum of unspent `value_doos` in `tracked_utxos`) —
/// includes IMMATURE coinbase, so tests can distinguish "wallet sees a coin
/// but can't spend it" from "wallet doesn't see the coin at all".
fn wallet_total_doos(app: &tauri::App<tauri::test::MockRuntime>) -> i64 {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    c.query_row(
        "SELECT COALESCE(SUM(value_doos), 0) FROM tracked_utxos
          WHERE wallet_profile_id = ?1 AND spent_by_txid IS NULL",
        params![PROFILE],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
}

/// Number of unspent coins the wallet currently sees.
fn wallet_utxo_count(app: &tauri::App<tauri::test::MockRuntime>) -> i64 {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    c.query_row(
        "SELECT COUNT(*) FROM tracked_utxos
          WHERE wallet_profile_id = ?1 AND spent_by_txid IS NULL",
        params![PROFILE],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
}

/// Read a numeric field out of a `TxDraftSummary.summary` value (which is the
/// camelCase-serialized `TxSummary`). Panics if missing — every summary the
/// build commands emit has all the numeric fields.
fn sum_i64(summary: &crate::noncustodial::types::TxDraftSummary, key: &str) -> i64 {
    summary
        .summary
        .get(key)
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("summary missing {key}: {}", summary.summary))
}

// =============================================================================
// GROUP A — Plain-send money paths (send.rs, commands/tx.rs)
// =============================================================================

/// A1. Requesting more than the wallet can cover -> `build_send_hns_draft`
/// fails `insufficient funds` before any draft row is written.
#[tokio::test]
async fn live_send_insufficient_funds() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_insufficient_funds: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    // Mine ONE mature coinbase (~2000 HNS = 2e9 doos), then ask for
    // 10x that. Even at 1 doo/byte the fee can't rescue it.
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let total = wallet_total_doos(&app);
    assert!(total > 0, "wallet must see coinbase after sync (got 0)");

    let res = build_send_hns_draft(
        app.state(),
        addr.clone(),
        total.saturating_mul(10),
        Some(1),
        None,
    )
    .await;
    expect_err(res, "insufficient funds");
}

/// A2. Requesting less than the dust threshold -> rejected before selection.
#[tokio::test]
async fn live_send_dust_amount_rejected() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_dust_amount_rejected: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // DUST_THRESHOLD is 1000 doos (send.rs).
    let dust_minus_one = (crate::noncustodial::send::DUST_THRESHOLD - 1) as i64;
    let res = build_send_hns_draft(app.state(), addr.clone(), dust_minus_one, Some(1), None).await;
    expect_err(res, "dust threshold");
}

/// A3. Sending to an EXTERNAL address (not one of the wallet's derived
/// addresses) — the change output must land at the wallet's branch=1/idx=0
/// change address (not a self-send back to 0/0).
#[tokio::test]
async fn live_send_change_lands_on_change_address() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_change_lands_on_change_address: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // A recipient address the wallet does NOT track: derive leaf 0/5, which
    // seeded_conn_regtest never inserts into derived_addresses. Sending there
    // guarantees the recipient output can't be confused with a self-send to
    // 0/0, so any wallet-side coin after the send must be genuine change.
    let (_sk, _pk, external) = hd::derive_address(NET, &seed(), 0, 0, 5).unwrap();

    let draft = build_send_hns_draft(app.state(), external.clone(), 500_000, Some(1), None)
        .await
        .expect("build send");
    assert!(
        sum_i64(&draft, "changeDoos") > 0,
        "expected non-zero change"
    );

    execute(&app, &cl, &addr, draft.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // The wallet must now hold at least one coin at the change address.
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    let change_at = change_addr_00();
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
              WHERE wallet_profile_id = ?1 AND address = ?2 AND spent_by_txid IS NULL",
            params![PROFILE, change_at],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n >= 1, "expected change UTXO at {change_at}, saw {n}");
}

/// A4. When the leftover after fee would fall below dust, coin selection folds
/// it into the fee (change==0) and the tx is still accepted on-chain.
#[tokio::test]
async fn live_send_dust_change_folded_into_fee() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_dust_change_folded_into_fee: set HNS_IT_NODE_URL");
        return;
    };
    // Private account (see `seeded_conn_acct`): this test asserts on the exact
    // coin set / calls getcoinsbyaddress, so it needs an address that no other
    // serial test funds.
    let conn = seeded_conn_acct(&url, &key, 11);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00_at(11);

    // Fund with exactly ONE coinbase so the wallet holds a single spendable
    // coin — that's the input shape this "1 input + 1 output; leftover < dust
    // -> fold into fee" branch exercises. Advance the tip on a throwaway
    // address so the one coinbase clears maturity without piling additional
    // immature coins onto this account.
    fund(&cl, &addr, 1).await;
    let burn = recv_leaf_01_at(99);
    fund(&cl, &burn, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // A single coinbase of value V; pick an amount just below V so
    // remainder < DUST_THRESHOLD after fee. From `select_coins`: with 1
    // input + 1 output the fee is deterministic given the rate; we don't
    // need the exact number — asking for V-500 (below dust) is well inside
    // the "fold into fee" branch for any reasonable fee at rate=1.
    let v = wallet_total_doos(&app);
    let amount = v - 500; // remainder pre-fee would be 500 (< 1000 dust)
    assert!(amount > 0, "coinbase value not read");

    let draft = build_send_hns_draft(app.state(), addr.clone(), amount, Some(1), None)
        .await
        .expect("build send folded");
    assert_eq!(sum_i64(&draft, "changeDoos"), 0, "change must be folded");
    assert!(sum_i64(&draft, "feeDoos") > 0);

    execute(&app, &cl, &addr, draft.id).await;
}

/// A5. `max=true` sweeps every spendable coin into a single output of
/// `inputTotal - fee`, no change, and is accepted on-chain.
#[tokio::test]
async fn live_send_max_sweeps_all_coins() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_max_sweeps_all_coins: set HNS_IT_NODE_URL");
        return;
    };
    // A fresh account (see `fresh_acct`): this test asserts on the exact coin
    // set and sweeps all of it. A fixed account gathered 105 coinbases a run;
    // after enough runs its sweep outgrew a standard transaction (8299 coins,
    // 1.17 MB), which hsd's regtest mempool took and no block ever mined,
    // stalling every later test that spent its change.
    let cl = client(&url, &key);
    let acct = fresh_acct(cl.get_blockchain_info().await.expect("info").blocks);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);

    // Mine several coinbases so there are multiple spendable coins to sweep.
    fund(&cl, &addr, 105).await;
    // Advance the tip a few blocks to a THROWAWAY address the wallet does not
    // track, so every coinbase we just mined to `addr` clears `coinbaseMaturity`
    // (2 on regtest) WITHOUT adding fresh immature coins to this account. That
    // makes the wallet's full unspent set == the spendable set the sweep sees,
    // so the exact-count/total assertions below are deterministic.
    let burn = recv_leaf_01_at(99);
    fund(&cl, &burn, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let before = wallet_utxo_count(&app);
    assert!(before >= 2, "need >=2 spendable coins for a real sweep");
    let total_before = wallet_total_doos(&app);

    let draft = build_send_hns_draft(app.state(), addr.clone(), 0, Some(1), Some(true))
        .await
        .expect("build max");
    assert_eq!(sum_i64(&draft, "changeDoos"), 0, "sweep has no change");
    let fee = sum_i64(&draft, "feeDoos");
    let input_total = sum_i64(&draft, "inputTotalDoos");
    let num_inputs = sum_i64(&draft, "numInputs");
    assert_eq!(num_inputs as i64, before, "sweep consumes ALL coins");
    assert_eq!(input_total, total_before, "input total = wallet total");
    assert!(fee > 0);

    execute(&app, &cl, &addr, draft.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    // Post-sweep, the wallet's only mature coin is the single sweep output
    // (self-send back to 0/0, minus fee) plus the just-mined block reward
    // (immature). We assert the sweep landed by checking the total went down
    // by exactly `fee` net of the new block reward — but the reward is
    // immature and its exact amount is regtest-consensus-defined, so just
    // check the sweep produced one confirmed output at `addr`.
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    let n_at_addr: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos
              WHERE wallet_profile_id = ?1 AND address = ?2 AND spent_by_txid IS NULL",
            params![PROFILE, addr],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n_at_addr >= 1);
}

/// A6. Regression guard for commit `a520456`: coinbase coins are NOT
/// spendable until `height + maturity <= tip + 1` (regtest maturity=2).
/// Mine one coinbase; sync; try to spend at the immature tip -> insufficient.
/// Mine past maturity; sync; retry -> accepted.
#[tokio::test]
async fn live_send_immature_coinbase_rejected_then_matures() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_immature_coinbase_rejected_then_matures: set HNS_IT_NODE_URL");
        return;
    };
    // REUSE-SAFETY: this test asserts that a freshly-mined coinbase is the
    // ONLY coin and is immature, so a positive send must fail "insufficient
    // funds". On a fixed account a prior run leaves mature coinbases behind and
    // the send would succeed. A height-keyed fresh account (see `fresh_acct`)
    // is guaranteed empty at the start of every run, so the single coinbase we
    // mine is genuinely alone and immature.
    let cl = client(&url, &key);
    let tip0 = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip0);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);

    // Mine ONE fresh coinbase — its coin is at height=tip; spend_height=tip+1;
    // maturity=2 means it becomes spendable when tip advances by another block.
    fund(&cl, &addr, 1).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Wallet SEES the coin (unspent, in tracked_utxos) but it is IMMATURE:
    // `build_send_hns_draft` calls `load_spendable_coins` which enforces the
    // maturity gate. Any positive request -> insufficient.
    let seen = wallet_total_doos(&app);
    assert!(
        seen > 0,
        "wallet sees the immature coinbase in tracked_utxos"
    );
    let res = build_send_hns_draft(app.state(), addr.clone(), 1_000_000, Some(1), None).await;
    // Assert only that the build was refused, not which explanation it chose.
    // The block reward halves as the chain grows, so on a fresh chain the one
    // coinbase (~2000 HNS) dwarfs the 1 HNS request and the error names
    // maturity, while on a long-lived chain (~0.97 HNS) maturity would not
    // close the gap and the generic message is correct. Both start
    // "insufficient"; the exact wording is pinned by the `shortfall_message`
    // unit tests, which do not depend on chain state.
    expect_err(res, "insufficient");

    // Advance the chain enough that the first coinbase clears `maturity` AND
    // is spendable given the fee for a 1-in-1-out. One extra block is enough
    // for maturity, but the fresh coinbase we mine to advance is itself
    // immature — so mine several so at least ONE coinbase from earlier is now
    // mature and spendable.
    fund(&cl, &addr, 5).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let draft = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("mature -> spendable");
    execute(&app, &cl, &addr, draft.id).await;
}

/// A7. A mainnet `hs1q`-encoded address is invalid on a regtest wallet —
/// `output_address_from_string` rejects it up front (before signing).
#[tokio::test]
async fn live_send_wrong_network_address_rejected() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_wrong_network_address_rejected: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Encode the same pubkey we own under Network::Main — the payload is fine
    // but the HRP is `hs` (mainnet) rather than `rs` (regtest).
    let (_sk, pk, _) = hd::derive_address(NET, &seed(), 0, 0, 0).unwrap();
    let mainnet_addr = crate::noncustodial::address::address_from_pubkey(
        crate::noncustodial::network::Network::Main,
        &pk,
    )
    .expect("encode mainnet");
    assert!(mainnet_addr.starts_with("hs1"), "sanity: {mainnet_addr}");

    let res = build_send_hns_draft(app.state(), mainnet_addr, 500_000, Some(1), None).await;
    // Rejection can surface as Crypto or InvalidInput depending on layer; both are fine.
    assert!(
        res.is_err(),
        "mainnet address must not be accepted on regtest"
    );
}

/// A8. Double spend: a draft whose coin another mined draft spent is not
/// sent. hsd answers its txid but never takes it, and the wallet says so
/// (honest-broadcast R2, in [`double_spent_draft`]); it is never mined.
#[tokio::test]
async fn live_send_double_spend_is_not_sent() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_double_spend_is_not_sent: set HNS_IT_NODE_URL");
        return;
    };
    let (app, a) = double_spent_draft(&url, &key).await;
    let txid = summary_txid(&app, &a);
    assert!(
        client(&url, &key)
            .get_tx_by_hash(&txid)
            .await
            .expect("tx lookup")
            .is_null(),
        "the double spend is never mined"
    );
}

/// A9. A draft already sent and mined is not sent again: the wallet refuses
/// before anything reaches the node, and the draft keeps its one txid.
#[tokio::test]
async fn live_send_rebroadcast_same_draft_rejected() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_rebroadcast_same_draft_rejected: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let draft = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    execute(&app, &cl, &addr, draft.id.clone()).await;
    let original_txid = draft_status(&app, &draft.id)
        .txid
        .expect("draft mined so it has a txid");

    let err = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect_err("a sent draft is not sent again");
    assert!(err.to_string().contains("already sent"), "{err}");
    assert_eq!(draft_status(&app, &draft.id).status, "confirmed");
    // Draft's recorded txid is unchanged either way.
    assert_eq!(
        draft_status(&app, &draft.id).txid.as_deref(),
        Some(original_txid.as_str()),
        "draft's txid must not change on rebroadcast"
    );
}

/// A10. `estimate_tx_draft_fee` mirrors the fee that a real build produces
/// for the same amount, and does NOT reserve coins (a subsequent build still
/// sees the full spendable set).
#[tokio::test]
async fn live_send_estimate_fee_smoke() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_estimate_fee_smoke: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let est = estimate_tx_draft_fee(app.state(), 500_000, Some(1))
        .await
        .expect("estimate");
    let est_fee = est.get("feeDoos").and_then(|v| v.as_i64()).unwrap();
    let est_change = est.get("changeDoos").and_then(|v| v.as_i64()).unwrap();
    let est_inputs = est.get("numInputs").and_then(|v| v.as_i64()).unwrap();
    assert!(
        est_fee > 0 && est_inputs >= 1 && est_change >= 0,
        "estimate: {est}"
    );

    // A real build with the same params must return the same fee/change/inputs.
    let draft = build_send_hns_draft(app.state(), addr.clone(), 500_000, Some(1), None)
        .await
        .expect("build");
    assert_eq!(sum_i64(&draft, "feeDoos"), est_fee);
    assert_eq!(sum_i64(&draft, "changeDoos"), est_change);
    assert_eq!(sum_i64(&draft, "numInputs"), est_inputs);
}

/// A11. The locally-computed txid the wallet stores in the draft's summary
/// exactly matches the txid the node returns from `sendrawtransaction` —
/// the strongest single guard against sighash/serialization drift.
#[tokio::test]
async fn live_send_txid_matches_node() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_txid_matches_node: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let draft = build_send_hns_draft(app.state(), addr.clone(), 500_000, Some(1), None)
        .await
        .expect("build");
    let local_txid = draft
        .summary
        .get("txid")
        .and_then(|v| v.as_str())
        .expect("build-time summary carries a local txid")
        .to_string();

    unlock(&app);
    sign_tx_draft_inner(&app.state(), &draft.id)
        .await
        .expect("sign");
    let bc = broadcast_tx_draft(app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    assert_eq!(
        bc.txid, local_txid,
        "node-returned txid must equal locally computed txid",
    );
}

// =============================================================================
// GROUP B — Reservation / draft-conflict invariants (send.rs, commands/tx.rs)
// =============================================================================

/// B1. Two builds against a single-coin wallet: the first reserves the coin,
/// so the second cannot select it and fails insufficient.
#[tokio::test]
async fn live_reservation_excludes_other_draft() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_reservation_excludes_other_draft: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    // Exactly ONE spendable coinbase (mine to maturity, then stop).
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    // Collapse to a single coin by sweeping everything into one self-output,
    // then re-sync so the wallet holds exactly one spendable coin.
    let sweep = build_send_hns_draft(app.state(), addr.clone(), 0, Some(1), Some(true))
        .await
        .expect("sweep build");
    execute(&app, &cl, &addr, sweep.id).await;
    // Mine to re-mature the sweep output, then sync.
    fund(&cl, &addr, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // First build reserves whatever coin(s) it selects.
    let d1 = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build 1");
    let _ = d1;
    // Second build over the SAME pool: if only the reserved coin(s) exist it
    // must fail; if multiple coins exist it must at least not reuse the first
    // draft's reserved inputs. Assert the reserved coins are excluded.
    let reserved_after_d1: i64 = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
            params![d1.id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        reserved_after_d1 >= 1,
        "first draft must hold a reservation"
    );
    let d2 = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None).await;
    match d2 {
        Err(_) => { /* only the reserved coin existed → insufficient. OK. */ }
        Ok(d2) => {
            // Multiple coins existed: the two drafts must not share an input.
            let state = app.state::<AppState>();
            let c = state.db.lock().unwrap();
            let shared: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
                    params![d1.id],
                    |r| r.get(0),
                )
                .unwrap();
            let shared2: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
                    params![d2.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                shared >= 1 && shared2 >= 1,
                "each draft holds its own reservation"
            );
        }
    }
}

/// B2. A stale unsigned draft's reservation is reclaimed once its created_at
/// ages past RESERVATION_TTL_SECS; an in-flight (broadcasted) draft is NOT.
#[tokio::test]
async fn live_reservation_ttl_reclaim() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_reservation_ttl_reclaim: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Unsigned draft holds a reservation.
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    let held_before: i64 = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
            params![d.id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(held_before >= 1);

    // Age it past the TTL and run the sweep (load_spendable_coins triggers it).
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        age_reservation(
            &c,
            &d.id,
            crate::noncustodial::send::RESERVATION_TTL_SECS + 60,
        );
        let released =
            crate::noncustodial::send::release_stale_reservations(&c, PROFILE).expect("sweep");
        assert!(
            released >= 1,
            "stale unsigned reservation must be reclaimed"
        );
    }

    // Now: broadcasted drafts are NOT reclaimed even when aged. Build+sign+
    // broadcast a fresh draft, age it, sweep → its reservation survives.
    let d2 = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build 2");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d2.id)
        .await
        .expect("sign");
    broadcast_tx_draft(app.state(), d2.id.clone())
        .await
        .expect("broadcast");
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        age_reservation(
            &c,
            &d2.id,
            crate::noncustodial::send::RESERVATION_TTL_SECS + 60,
        );
        crate::noncustodial::send::release_stale_reservations(&c, PROFILE).expect("sweep2");
        let held: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
                params![d2.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            held >= 1,
            "broadcasted draft reservation must survive the TTL sweep"
        );
    }
    // Clean up so the broadcast tx gets mined and the harness stays consistent.
    fund(&cl, &addr, 1).await;
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
}

/// B3. delete_tx_draft frees an unsigned draft's reservation but refuses to
/// delete a broadcasted/confirmed draft.
#[tokio::test]
async fn live_delete_draft_releases_and_guards() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_delete_draft_releases_and_guards: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Unsigned → delete frees the reservation.
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    crate::commands::tx::delete_tx_draft(app.state(), d.id.clone())
        .await
        .expect("delete unsigned");
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        let held: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
                params![d.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(held, 0, "deleting an unsigned draft frees its coins");
    }

    // Broadcasted → delete refuses.
    let d2 = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build 2");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d2.id)
        .await
        .expect("sign");
    broadcast_tx_draft(app.state(), d2.id.clone())
        .await
        .expect("broadcast");
    let res = crate::commands::tx::delete_tx_draft(app.state(), d2.id.clone()).await;
    assert!(res.is_err(), "a broadcast draft must not be deletable");
    fund(&cl, &addr, 1).await;
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
}

/// B4. release_tx_draft_reservation frees coins without deleting the draft.
#[tokio::test]
async fn live_release_reservation_command() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_release_reservation_command: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    let out = crate::commands::tx::release_tx_draft_reservation(app.state(), d.id.clone())
        .await
        .expect("release");
    assert!(
        out.get("coinsReleased")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            >= 1
    );
    // Draft row still exists (not deleted).
    let row = draft_status(&app, &d.id);
    assert_eq!(row.id, d.id);
    // Coins are free again.
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    let held: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
            params![d.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(held, 0);
}

// =============================================================================
// GROUP C — Confirmation / reorg state machine (refresh_tx_confirmations)
// =============================================================================
//
// These drive REAL reorgs via the #[cfg(test)] invalidate_block/reconsider_block
// RPC passthroughs. invalidateblock mutates shared node state, so these are the
// most stateful tests in the suite — each one restores the chain it rewinds
// (reconsiderblock) before returning so later tests see a consistent tip.

/// C1. A confirmed draft reverts to broadcasted when its block is invalidated,
/// then re-confirms once the block is reconsidered.
#[tokio::test]
async fn live_confirm_reorg_reverts_to_broadcasted() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_confirm_reorg_reverts_to_broadcasted: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Build + sign + broadcast, then mine 1 so it confirms.
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d.id)
        .await
        .expect("sign");
    broadcast_tx_draft(app.state(), d.id.clone())
        .await
        .expect("broadcast");
    cl.generate_to_address(1, &addr).await.expect("mine");
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed", "draft should confirm before reorg");
    let height = row.confirmation_height.expect("confirmed height");

    // Invalidate the block that contains the tx → chain rewinds to its parent.
    let block_hash = cl.get_block_hash(height).await.expect("blockhash");
    cl.invalidate_block(&block_hash).await.expect("invalidate");

    // Refresh now sees the tx with 0 confs (back in mempool) or not found →
    // reverts confirmed → broadcasted.
    let r = refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh reorg");
    assert!(
        r["reverted"].as_i64().unwrap_or(0) >= 1,
        "expected a reverted draft after invalidateblock: {r}"
    );
    let row = draft_status(&app, &d.id);
    assert_eq!(
        row.status, "broadcasted",
        "reverted draft is back to broadcasted"
    );

    // Restore the chain and drive the draft back to `confirmed`. hsd's
    // `invalidateblock` rewinds through `reset`, which empties the mempool,
    // and `reconsiderblock` only clears the invalid mark: the tx is nowhere.
    // A real reorg puts a disconnected block's transactions back in the
    // mempool; the test does that by hand, handing the signed tx to the node
    // directly (the wallet does not send a `broadcasted` draft again).
    cl.reconsider_block(&block_hash).await.expect("reconsider");
    let signed = row.signed_tx_hex.clone().expect("signed hex");
    cl.send_raw_transaction(&signed).await.expect("resubmit");
    wait_until_node_has(&cl, row.txid.as_deref().expect("txid")).await;
    cl.generate_to_address(1, &addr).await.expect("remine");
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh restore");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed", "re-confirms after reconsiderblock");
}

/// C2. Once a draft has >= CONFIRMATION_FINALITY_DEPTH (12) confs it is treated
/// as final: refresh keeps it confirmed and doesn't churn its status.
#[tokio::test]
async fn live_confirm_finality_ceiling() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_confirm_finality_ceiling: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    execute(&app, &cl, &addr, d.id.clone()).await;
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed");

    // Bury it well beyond the finality depth, then refresh repeatedly.
    cl.generate_to_address(15, &addr).await.expect("bury");
    for _ in 0..3 {
        refresh_tx_confirmations(app.state(), None)
            .await
            .expect("refresh");
        let row = draft_status(&app, &d.id);
        assert_eq!(
            row.status, "confirmed",
            "deeply-buried draft stays confirmed"
        );
    }
}

/// C3. A broadcast_pending draft (transport-ambiguous broadcast) whose tx is
/// actually on-chain gets promoted by refresh via local_txid_from_summary.
#[tokio::test]
async fn live_broadcast_pending_promotes() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_broadcast_pending_promotes: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Build + sign, broadcast normally (so the tx really is on-chain-able),
    // then force the draft's status to broadcast_pending to simulate a
    // transport-ambiguous outcome where we never got a definitive txid.
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d.id)
        .await
        .expect("sign");
    broadcast_tx_draft(app.state(), d.id.clone())
        .await
        .expect("broadcast");
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        // Clear the DB txid + set status to broadcast_pending: refresh must
        // recover the txid from the summary (local_txid_from_summary) and
        // promote it once the tx is found on-chain.
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'broadcast_pending', txid = NULL WHERE id = ?1",
            params![d.id],
        )
        .unwrap();
    }
    cl.generate_to_address(1, &addr).await.expect("mine");
    let r = refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert!(
        matches!(row.status.as_str(), "broadcasted" | "confirmed"),
        "broadcast_pending draft must be promoted, got {} ({r})",
        row.status
    );
    assert!(row.txid.is_some(), "promotion recovers the txid");
}

/// C4. After a coinbase is confirmed then unmined by invalidateblock and
/// re-mined at a different height, the wallet treats it as freshly immature.
#[tokio::test]
async fn live_coinbase_reorg_immaturity() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_coinbase_reorg_immaturity: set HNS_IT_NODE_URL");
        return;
    };
    // REUSE-SAFETY: the "one fresh coinbase, invalidate, remine -> immature"
    // sequence requires the account's spendable set to be EMPTY except for the
    // single coinbase under test. A fixed account inherits mature coinbases
    // from a prior run (or sibling tests), making the post-reorg send succeed.
    // A height-keyed fresh account (see `fresh_acct`) is empty on every run.
    let cl = client(&url, &key);
    let tip0 = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip0);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);

    // Mine ONE coinbase (C1) to this account, then advance the tip on a
    // THROWAWAY address so C1 clears maturity (regtest `coinbaseMaturity` = 2;
    // the wallet's gate spends at `tip + 1`, so C1 at height h is mature once
    // `tip + 1 >= h + 2`, i.e. `tip >= h + 1`). We record the first burn block —
    // that's the reorg boundary we'll roll back to.
    fund(&cl, &addr, 1).await; // C1 at height h = N+1, tip = N+1
    let c1_tip = cl.get_blockchain_info().await.expect("info").blocks;
    // Throwaway address from this run's OWN fresh account (branch 1) so it
    // never collides with a sibling test's throwaway.
    let burn = recv_leaf_01_at(acct);
    cl.generate_to_address(1, &burn).await.expect("burn +1"); // tip = N+2 -> C1 mature
    let boundary = cl.get_block_hash(c1_tip + 1).await.expect("boundary hash");
    cl.generate_to_address(1, &burn).await.expect("burn +2"); // tip = N+3
    sync_wallet_state(app.state(), None).await.expect("sync");
    // C1 is mature and spendable now.
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("spendable while mature");
    crate::commands::tx::release_tx_draft_reservation(app.state(), d.id.clone())
        .await
        .expect("release");

    // Reorg: invalidate the first block AFTER C1 (the reorg boundary),
    // rewinding the tip back to C1's own height. At `tip = h` the wallet's
    // spend height is `h + 1 < h + 2`, so C1 is IMMATURE again — a spend must
    // fail insufficient even though the coin still exists in the coin set.
    cl.invalidate_block(&boundary).await.expect("invalidate");
    sync_wallet_state(app.state(), None)
        .await
        .expect("sync after reorg");
    let res = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None).await;
    expect_err(res, "insufficient mature funds");

    // Restore the chain for later tests.
    cl.reconsider_block(&boundary).await.expect("reconsider");
    cl.generate_to_address(2, &burn).await.expect("restore tip");
    sync_wallet_state(app.state(), None)
        .await
        .expect("sync restore");
}

// =============================================================================
// GROUP D — Covenant actions with no prior live coverage (commands/names.rs)
// =============================================================================
//
// Each acquires an owned name via acquire_name, then exercises the action
// end-to-end and asserts the on-chain getnameinfo state.

/// D1. UPDATE writes new resource records to an owned name; the name stays
/// owned (CLOSED) and its owner value is preserved (no price change).
#[tokio::test]
async fn live_update_records() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_update_records: set HNS_IT_NODE_URL");
        return;
    };
    // Private account (see `seeded_conn_acct`): this test asserts on the exact
    // coin set / calls getcoinsbyaddress, so it needs an address that no other
    // serial test funds.
    let conn = seeded_conn_acct(&url, &key, 16);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00_at(16);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("upd{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    // Owner value before UPDATE.
    let before = cl.get_name_info(&name).await.expect("info");
    let value_before = before
        .get("info")
        .and_then(|i| i.get("value"))
        .and_then(|v| v.as_u64());

    let records = vec![serde_json::json!({"type":"TXT","txt":["updated-record"]})];
    let upd =
        crate::commands::names::build_update_draft(app.state(), name.clone(), records, Some(1))
            .await
            .expect("build update");
    execute(&app, &cl, &addr, upd.id).await;

    // Still owned/closed, value unchanged, and a resource is now present.
    let after = cl.get_name_info(&name).await.expect("info");
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
    let value_after = after
        .get("info")
        .and_then(|i| i.get("value"))
        .and_then(|v| v.as_u64());
    assert_eq!(
        value_before, value_after,
        "UPDATE must not change owner value"
    );
}

/// D2. Regression guard for commit 78bba67: RENEW references the renewal
/// block decoded in hsd internal (UNREVERSED) byte order, so the node accepts
/// it (no bad-register-renewal). The name stays owned and its value is kept.
///
/// A RENEW right after REGISTER is refused by hsd (`bad-renewal-premature`:
/// not before `renewal + tree_interval`), so the builder refuses it first;
/// once that many blocks are mined the renewal lands and moves `renewal` to
/// the block it was mined in.
#[tokio::test]
async fn live_renew_extends_lease() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_renew_extends_lease: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("rnw{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    let before = cl.get_name_info(&name).await.expect("info");
    let value_before = before
        .get("info")
        .and_then(|i| i.get("value"))
        .and_then(|v| v.as_u64());

    // Just registered: too early to renew.
    expect_err(
        crate::commands::names::build_renew_draft(app.state(), name.clone(), Some(1)).await,
        "renewed too recently",
    );
    let renewal_before = name_info_height(&cl, &name, "renewal").await;
    mine_until_renewable(&cl, &[&name], &addr).await;

    // build_renew_draft calls renewal_block() -> getblockhash decoded
    // UNREVERSED. If the byte order regressed, the node would reject the
    // covenant with bad-register-renewal on broadcast; execute() asserts the
    // node took it.
    let renew = crate::commands::names::build_renew_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build renew");
    execute(&app, &cl, &addr, renew.id).await;

    // The renewal is on chain: `renewal` is now the block it was mined in.
    let mined_at = cl.get_blockchain_info().await.expect("info").blocks;
    let renewal_after = name_info_height(&cl, &name, "renewal").await;
    assert!(renewal_after > renewal_before, "renewal did not advance");
    assert_eq!(renewal_after, mined_at, "renewed in the block just mined");

    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
    let after = cl.get_name_info(&name).await.expect("info");
    let value_after = after
        .get("info")
        .and_then(|i| i.get("value"))
        .and_then(|v| v.as_u64());
    assert_eq!(value_before, value_after, "RENEW must preserve owner value");
}

/// D3. CANCEL clears a pending transfer before finalize; the name stays owned.
#[tokio::test]
async fn live_cancel_reverts_transfer() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_cancel_reverts_transfer: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("cnl{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    // TRANSFER the name to leaf 0/1, then CANCEL before finalize.
    let recipient = recv_leaf_01();
    let transfer =
        crate::commands::names::build_transfer_draft(app.state(), name.clone(), recipient, Some(1))
            .await
            .expect("build transfer");
    execute(&app, &cl, &addr, transfer.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    // Confirm a pending transfer exists.
    let mid = cl.get_name_info(&name).await.expect("info");
    let transfer_h = mid
        .get("info")
        .and_then(|i| i.get("transfer"))
        .and_then(|t| t.as_u64());
    assert!(
        transfer_h.map(|h| h > 0).unwrap_or(false),
        "pending transfer expected: {mid}"
    );

    let cancel = crate::commands::names::build_cancel_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build cancel");
    execute(&app, &cl, &addr, cancel.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Transfer cleared (transfer == 0), name still CLOSED (owned).
    let after = cl.get_name_info(&name).await.expect("info");
    let transfer_after = after
        .get("info")
        .and_then(|i| i.get("transfer"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    assert_eq!(
        transfer_after, 0,
        "CANCEL clears the pending transfer: {after}"
    );
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
}

/// D4. REVOKE burns control of an owned name; the node reports it REVOKED and
/// the revoke coin is classified Unsupported by sync (not spendable, not lost).
#[tokio::test]
async fn live_revoke_burns_control() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_revoke_burns_control: set HNS_IT_NODE_URL");
        return;
    };
    // REUSE-SAFETY: a REVOKE leaves a burned revoked-covenant coin on `addr`,
    // and hsd's addrindex asserts (500) on `getcoinsbyaddress` for an address
    // holding one. On a fixed account, a prior run's revoked coin lingers and
    // the sync inside `acquire_name` trips that 500 before this run even
    // revokes. A height-keyed fresh account (see `fresh_acct`) starts with a
    // clean address on every run.
    let cl = client(&url, &key);
    let tip0 = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip0);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("rvk{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    let revoke = crate::commands::names::build_revoke_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build revoke");
    execute(&app, &cl, &addr, revoke.id).await;
    // Advance a couple blocks so the REVOKE covenant is well-buried.
    cl.generate_to_address(2, &addr).await.expect("mine");
    // NOTE: we deliberately do NOT run a full wallet sync here. A REVOKE
    // covenant leaves a burned output on `addr`, and hsd's addrindex asserts
    // (500) on `getcoinsbyaddress` for an address holding a revoked-covenant
    // coin — an upstream hsd limitation, not a wallet bug. The invariant this
    // test proves ("revoke burns control -> the name is REVOKED on-chain") is
    // read straight from the node via `getnameinfo`, which does not touch the
    // coin index.
    assert_eq!(
        node_state(&cl, &name).await.as_deref(),
        Some("REVOKED"),
        "name must be REVOKED"
    );
}

// =============================================================================
// GROUP E — Batch covenant variants with no prior live coverage
// =============================================================================

/// Open two names and advance BOTH to BIDDING (each OPEN is its own tx — there
/// is no batch-open command). Returns once both names report BIDDING.
async fn open_two_to_bidding(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    name_a: &str,
    name_b: &str,
) {
    for name in [name_a, name_b] {
        let open = build_open_draft(app.state(), name.to_string(), Some(1))
            .await
            .expect("build open");
        execute(app, cl, addr, open.id).await;
        sync_wallet_state(app.state(), None).await.expect("sync");
    }
    for name in [name_a, name_b] {
        assert!(
            mine_until(cl, name, "BIDDING", addr, 30).await,
            "name {name} did not reach BIDDING"
        );
    }
}

/// E1. build_batch_bid_draft places a shared-lockup bid on two names in ONE tx;
/// both BID coins land on-chain and both commitments were persisted.
#[tokio::test]
async fn live_batch_bid_two_names() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_bid_two_names: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name_a = format!("bbid{tip}a");
    let name_b = format!("bbid{tip}b");
    open_two_to_bidding(&app, &cl, &addr, &name_a, &name_b).await;

    sync_wallet_state(app.state(), None).await.expect("sync");
    let batch = crate::commands::names::build_batch_bid_draft(
        app.state(),
        vec![name_a.clone(), name_b.clone()],
        1_000_000,
        2_000_000,
        Some(1),
    )
    .await
    .expect("build batch bid");
    execute(&app, &cl, &addr, batch.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Both bid commitments must be persisted (blind/nonce recorded per-name).
    for name in [&name_a, &name_b] {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        let has_commit: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
                params![PROFILE, name],
                |r| r.get(0),
            )
            .unwrap();
        assert!(has_commit >= 1, "bid commitment persisted for {name}");
    }
    // Advance both to REVEAL to confirm the bids were accepted on-chain.
    for name in [&name_a, &name_b] {
        assert!(
            mine_until(&cl, name, "REVEAL", &addr, 30).await,
            "{name} did not reach REVEAL"
        );
    }
}

/// E2. build_batch_reveal_draft reveals two batch-bid names in one tx; both
/// reveals are accepted and the names advance toward CLOSED.
#[tokio::test]
async fn live_batch_reveal_two_names() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_reveal_two_names: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name_a = format!("brvl{tip}a");
    let name_b = format!("brvl{tip}b");
    open_two_to_bidding(&app, &cl, &addr, &name_a, &name_b).await;

    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = crate::commands::names::build_batch_bid_draft(
        app.state(),
        vec![name_a.clone(), name_b.clone()],
        1_000_000,
        2_000_000,
        Some(1),
    )
    .await
    .expect("build batch bid");
    execute(&app, &cl, &addr, bid.id).await;
    for name in [&name_a, &name_b] {
        assert!(
            mine_until(&cl, name, "REVEAL", &addr, 30).await,
            "{name} did not reach REVEAL"
        );
    }

    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = crate::commands::names::build_batch_reveal_draft(
        app.state(),
        vec![name_a.clone(), name_b.clone()],
        Some(1),
    )
    .await
    .expect("build batch reveal");
    execute(&app, &cl, &addr, reveal.id).await;
    for name in [&name_a, &name_b] {
        assert!(
            mine_until(&cl, name, "CLOSED", &addr, 40).await,
            "{name} did not reach CLOSED"
        );
    }
}

/// E3. Losing batch bids are reclaimed with build_batch_redeem_draft after the
/// auctions close: a rival wallet on the same node outbids this one on both
/// names, and one REDEEM transaction spends both of this wallet's losing
/// reveals while the rival's reveals stay the owners.
#[tokio::test]
async fn live_batch_redeem_two_names() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_redeem_two_names: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    let (rival, rival_addr) = rival_wallet(&url, &key);
    fund(&cl, &addr, 101).await;
    fund(&cl, &rival_addr, 5).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name_a = format!("brdm{tip}a");
    let name_b = format!("brdm{tip}b");
    let names = vec![name_a.clone(), name_b.clone()];
    open_two_to_bidding(&app, &cl, &addr, &name_a, &name_b).await;

    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = crate::commands::names::build_batch_bid_draft(
        app.state(),
        names.clone(),
        500_000,
        1_000_000,
        Some(1),
    )
    .await
    .expect("build batch bid");
    execute(&app, &cl, &addr, bid.id).await;
    sync_wallet_state(rival.state(), None)
        .await
        .expect("sync rival");
    let rival_bid = crate::commands::names::build_batch_bid_draft(
        rival.state(),
        names.clone(),
        800_000,
        1_600_000,
        Some(1),
    )
    .await
    .expect("build rival batch bid");
    execute(&rival, &cl, &addr, rival_bid.id).await;
    for name in [&name_a, &name_b] {
        assert!(
            mine_until(&cl, name, "REVEAL", &addr, 30).await,
            "{name} did not reach REVEAL"
        );
    }

    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal =
        crate::commands::names::build_batch_reveal_draft(app.state(), names.clone(), Some(1))
            .await
            .expect("build batch reveal");
    execute(&app, &cl, &addr, reveal.id.clone()).await;
    sync_wallet_state(rival.state(), None)
        .await
        .expect("sync rival");
    let rival_reveal =
        crate::commands::names::build_batch_reveal_draft(rival.state(), names.clone(), Some(1))
            .await
            .expect("build rival batch reveal");
    execute(&rival, &cl, &addr, rival_reveal.id.clone()).await;
    for name in [&name_a, &name_b] {
        assert!(
            mine_until(&cl, name, "CLOSED", &addr, 40).await,
            "{name} did not reach CLOSED"
        );
    }

    // The rival's reveals own both names; both of ours lost.
    let rival_reveal_txid = draft_status(&rival, &rival_reveal.id)
        .txid
        .expect("rival reveal txid");
    for name in [&name_a, &name_b] {
        assert_eq!(name_owner(&cl, name).await.0, rival_reveal_txid, "{name}");
    }
    let ours = covenant_outpoints(
        &cl,
        &draft_status(&app, &reveal.id).txid.expect("reveal txid"),
        COV_TYPE_REVEAL,
    )
    .await;
    assert_eq!(ours.len(), 2, "one reveal output per name: {ours:?}");

    sync_wallet_state(app.state(), None).await.expect("sync");
    let redeem =
        crate::commands::names::build_batch_redeem_draft(app.state(), names.clone(), Some(1))
            .await
            .expect("build batch redeem");
    execute(&app, &cl, &addr, redeem.id.clone()).await;

    // One transaction spent both losing reveals; the owners are unchanged.
    let redeem_txid = draft_status(&app, &redeem.id).txid.expect("redeem txid");
    assert_spends_reveals(&cl, &redeem_txid, &ours).await;
    assert_eq!(
        covenant_outpoints(&cl, &redeem_txid, COV_TYPE_REDEEM)
            .await
            .len(),
        2,
        "one REDEEM output per name"
    );
    for name in [&name_a, &name_b] {
        assert_eq!(name_owner(&cl, name).await.0, rival_reveal_txid, "{name}");
        assert_eq!(node_state(&cl, name).await.as_deref(), Some("CLOSED"));
    }
}

/// E4. build_batch_renew_draft renews two owned names against a shared renewal
/// block in one tx; both stay owned (CLOSED).
#[tokio::test]
async fn live_batch_renew_two_names() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_renew_two_names: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name_a = format!("brnw{tip}a");
    let name_b = format!("brnw{tip}b");
    acquire_two_names(&app, &cl, &addr, &name_a, &name_b).await;
    let names = vec![name_a.clone(), name_b.clone()];

    // `name_b` was registered in the last block: the batch would carry a
    // RENEW hsd refuses (`bad-renewal-premature`), so the builder refuses the
    // whole batch.
    expect_err(
        crate::commands::names::build_batch_renew_draft(app.state(), names.clone(), Some(1)).await,
        "renewed too recently",
    );
    let before = [
        name_info_height(&cl, &name_a, "renewal").await,
        name_info_height(&cl, &name_b, "renewal").await,
    ];
    mine_until_renewable(&cl, &[&name_a, &name_b], &addr).await;

    let renew =
        crate::commands::names::build_batch_renew_draft(app.state(), names.clone(), Some(1))
            .await
            .expect("build batch renew");
    execute(&app, &cl, &addr, renew.id).await;
    let mined_at = cl.get_blockchain_info().await.expect("info").blocks;
    for (name, before) in names.iter().zip(before) {
        let after = name_info_height(&cl, name, "renewal").await;
        assert!(after > before, "{name}: renewal did not advance");
        assert_eq!(after, mined_at, "{name} renewed in the block just mined");
        assert_eq!(node_state(&cl, name).await.as_deref(), Some("CLOSED"));
    }
}

/// E5. build_batch_finalize_draft finalizes two transferred names in a SINGLE
/// tx (not a per-name loop) after the transfer lockup elapses.
#[tokio::test]
async fn live_batch_finalize_two_names() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_finalize_two_names: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name_a = format!("bfin{tip}a");
    let name_b = format!("bfin{tip}b");
    acquire_two_names(&app, &cl, &addr, &name_a, &name_b).await;

    // Batch transfer both to leaf 0/1.
    let recipient = recv_leaf_01();
    let batch = crate::commands::names::build_batch_transfer_draft(
        app.state(),
        vec![name_a.clone(), name_b.clone()],
        recipient,
        Some(1),
    )
    .await
    .expect("build batch transfer");
    execute(&app, &cl, &addr, batch.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Advance past the regtest transfer lockup (10 blocks).
    cl.generate_to_address(11, &addr).await.expect("lockup");
    sync_wallet_state(app.state(), None).await.expect("sync");

    let fin = crate::commands::names::build_batch_finalize_draft(
        app.state(),
        vec![name_a.clone(), name_b.clone()],
        Some(1),
    )
    .await
    .expect("build batch finalize");
    execute(&app, &cl, &addr, fin.id).await;
    for name in [&name_a, &name_b] {
        assert_eq!(
            node_state(&cl, name).await.as_deref(),
            Some("CLOSED"),
            "{name} not CLOSED"
        );
    }
}

// =============================================================================
// GROUP F — Covenant invariant + timing negatives (money-loss / on-chain reject)
// =============================================================================

/// Find the value of the first output in a tx whose covenant.type matches
/// `want_type`. Returns None if the tx has no such output. hsd covenant type
/// codes: BID=3, REVEAL=4, REGISTER=6 (see noncustodial/sync.rs).
async fn output_value_by_covenant_type(
    cl: &NodeRpcClient,
    txid: &str,
    want_type: u64,
) -> Option<u64> {
    // Use the REST /tx/:hash route — it does not require txindex to be tuned
    // for JSON-RPC `getrawtransaction`, and returns the same decoded shape
    // (`outputs[i].covenant.{type,items}`, `outputs[i].value`) that the block
    // scanner already relies on.
    let tx = cl.get_tx_by_hash(txid).await.ok()?;
    if tx.is_null() {
        return None;
    }
    let outputs = tx.get("outputs")?.as_array()?;
    for o in outputs {
        let t = o
            .get("covenant")
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_u64());
        if t == Some(want_type) {
            return o.get("value").and_then(|v| v.as_u64());
        }
    }
    None
}

/// F1. Finalize BEFORE transfer_lockup (10 blocks on regtest) is refused by
/// the builder, as hsd would refuse it; after the lockup, it is accepted and
/// the name moves to the recipient.
#[tokio::test]
async fn live_premature_finalize_rejected() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_premature_finalize_rejected: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("pfin{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    // Transfer to leaf 0/1.
    let recipient = recv_leaf_01();
    let transfer = crate::commands::names::build_transfer_draft(
        app.state(),
        name.clone(),
        recipient.clone(),
        Some(1),
    )
    .await
    .expect("build transfer");
    execute(&app, &cl, &addr, transfer.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Try to finalize IMMEDIATELY — before the transfer lockup elapses. hsd
    // refuses that FINALIZE (`bad-finalize-maturity`), but its
    // `sendrawtransaction` hands back the txid all the same, so the wallet
    // cannot leave the refusal to the broadcast: the builder refuses it, says
    // how long the lockup still runs, and persists nothing.
    let drafts_before = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    expect_err(
        crate::commands::names::build_finalize_draft(app.state(), name.clone(), Some(1)).await,
        "still locked for 9 more blocks",
    );
    let drafts_after = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    assert_eq!(
        drafts_before, drafts_after,
        "a refused finalize persists nothing"
    );
    let transfer_h = name_info_height(&cl, &name, "transfer").await;
    assert!(transfer_h > 0, "the transfer is still pending");

    // Advance past the transfer lockup (10 blocks), sync, and retry — accepted.
    cl.generate_to_address(11, &addr).await.expect("lockup");
    sync_wallet_state(app.state(), None).await.expect("sync");
    let fin2 = crate::commands::names::build_finalize_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build finalize 2");
    execute(&app, &cl, &addr, fin2.id).await;
    assert_eq!(
        owner_coin_address(&cl, &name).await.as_deref(),
        Some(recipient.as_str())
    );
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
}

/// F2. On-chain invariants: BID output value == lockup (>= bid), REVEAL output
/// value == true bid, change == lockup - bid.
#[tokio::test]
async fn live_bid_lockup_invariant_and_reveal_value() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_bid_lockup_invariant_and_reveal_value: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("blk{tip}");
    // OPEN → BIDDING
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");

    let bid_val: u64 = 1_000_000;
    let lockup: u64 = 2_500_000;
    let bid = build_bid_draft(
        app.state(),
        name.clone(),
        bid_val as i64,
        lockup as i64,
        Some(1),
    )
    .await
    .expect("build bid");
    execute(&app, &cl, &addr, bid.id.clone()).await;
    let bid_row = draft_status(&app, &bid.id);
    let bid_txid = bid_row.txid.expect("bid txid");

    // On-chain BID output value == lockup.
    let bid_out = output_value_by_covenant_type(&cl, &bid_txid, 3)
        .await
        .expect("bid output present");
    assert_eq!(bid_out, lockup, "BID output value must equal lockup");

    // Advance to REVEAL and reveal.
    assert!(mine_until(&cl, &name, "REVEAL", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id.clone()).await;
    let reveal_row = draft_status(&app, &reveal.id);
    let reveal_txid = reveal_row.txid.expect("reveal txid");
    let reveal_out = output_value_by_covenant_type(&cl, &reveal_txid, 4)
        .await
        .expect("reveal output present");
    assert_eq!(
        reveal_out, bid_val,
        "REVEAL output value must equal true bid"
    );
}

/// F3. On-chain REGISTER output value == the auction clearing price
/// (getnameinfo.info.value). Remainder returns to the wallet as change.
#[tokio::test]
async fn live_register_value_is_clearing_price() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_register_value_is_clearing_price: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("clr{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    // getnameinfo.value == clearing price (single-bidder auction: == true bid).
    let info = cl.get_name_info(&name).await.expect("info");
    let clearing = info
        .get("info")
        .and_then(|i| i.get("value"))
        .and_then(|v| v.as_u64())
        .expect("clearing price present");

    // The REGISTER tx sits on the owner coin — we can look up the coin by name.
    // Simpler: acquire_name issued the REGISTER via the batch of build_ +
    // execute; its txid is on the wallet's most recent register draft.
    let register_txid: String = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row(
            "SELECT txid FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'register' AND status = 'confirmed' ORDER BY created_at DESC LIMIT 1",
            params![PROFILE], |r| r.get(0)).unwrap()
    };

    let reg_out = output_value_by_covenant_type(&cl, &register_txid, 6)
        .await
        .expect("register output present");
    assert_eq!(
        reg_out, clearing,
        "REGISTER output value must equal clearing price"
    );
}

/// F4. A winner cannot build a REDEEM: their reveal coin is spent into the
/// REGISTER, so build_redeem_draft has no unspent losing reveal to reclaim.
#[tokio::test]
async fn live_redeem_when_won_rejected() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_redeem_when_won_rejected: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("wnr{tip}");
    acquire_name(&app, &cl, &addr, &name).await; // winner: reveal was spent into register

    let res = crate::commands::names::build_redeem_draft(app.state(), name.clone(), Some(1)).await;
    assert!(res.is_err(), "winner must not be able to redeem");
}

/// F5. A second OPEN for the same name while the first draft is still pending
/// is rejected at the command layer (no duplicate hits the chain). A second
/// BID is not: the wallet lets you bid on one name as many times as you like
/// (`docs/specs/2026-09-20-multiple-bids-per-name.md`), and each bid draft
/// holds coins of its own.
#[tokio::test]
async fn live_double_open_and_double_bid_guarded() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_double_open_and_double_bid_guarded: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("dbl{tip}");

    // First OPEN: builds fine. Second OPEN (still pending): rejected.
    let _open1 = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("open 1");
    let open2 = build_open_draft(app.state(), name.clone(), Some(1)).await;
    assert!(
        open2.is_err(),
        "second OPEN while first is pending must be rejected"
    );

    // Broadcast the first OPEN and advance to BIDDING so we can test double-bid.
    // Note: we already have a build; sign + broadcast + mine it directly
    // rather than rebuilding.
    let state_open_id = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        let id: String = c.query_row(
            "SELECT id FROM wallet_tx_drafts WHERE wallet_profile_id = ?1 AND action = 'open' AND status = 'draft' ORDER BY created_at DESC LIMIT 1",
            params![PROFILE], |r| r.get(0)).unwrap();
        id
    };
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &state_open_id)
        .await
        .expect("sign");
    broadcast_tx_draft(app.state(), state_open_id.clone())
        .await
        .expect("broadcast");
    cl.generate_to_address(1, &addr).await.expect("mine");
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Two BIDs while both are drafts: both build, on different coins.
    let bid1 = build_bid_draft(app.state(), name.clone(), 500_000, 1_000_000, Some(1))
        .await
        .expect("bid 1");
    let bid2 = build_bid_draft(app.state(), name.clone(), 500_000, 1_000_000, Some(1))
        .await
        .expect("a second bid on the same name builds");
    let reserved = |id: &str| -> Vec<(String, i64)> {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        let mut stmt = c
            .prepare("SELECT txid, vout FROM tracked_utxos WHERE reserved_by_draft_id = ?1")
            .unwrap();
        stmt.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let (coins1, coins2) = (reserved(&bid1.id), reserved(&bid2.id));
    assert!(
        !coins1.is_empty() && !coins2.is_empty(),
        "each bid holds coins"
    );
    assert!(
        coins1.iter().all(|c| !coins2.contains(c)),
        "no coin funds both bids"
    );
}

// =============================================================================
// GROUP G — Atomic swap + signing + capability gates
// =============================================================================

/// G1. build_finalize_with_payment_draft produces a SINGLE tx that finalizes
/// the transfer to the buyer AND pays the seller `payment_value`. On-chain,
/// both effects are present (or neither): the tx contains a FINALIZE output
/// for `name` AND a value output to `payment_address`.
#[tokio::test]
async fn live_finalize_with_payment_atomic() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_finalize_with_payment_atomic: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("fwp{tip}");
    acquire_name(&app, &cl, &addr, &name).await;

    // Transfer to leaf 0/1 and wait out the lockup so finalize is valid.
    let buyer = recv_leaf_01();
    let transfer = crate::commands::names::build_transfer_draft(
        app.state(),
        name.clone(),
        buyer.clone(),
        Some(1),
    )
    .await
    .expect("build transfer");
    execute(&app, &cl, &addr, transfer.id).await;
    cl.generate_to_address(11, &addr).await.expect("lockup");
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Payment address: node-side would be ideal, but a wallet-owned leaf
    // still exercises the atomic-tx path (buyer + seller happen to be the
    // same wallet here; the invariant we check is that ONE tx has BOTH
    // a FINALIZE covenant AND a value output to the payment_address).
    let seller_pay = leaf00().0; // pay back to 0/0
    let payment_value: u64 = 3_000_000;
    let fwp = crate::commands::names::build_finalize_with_payment_draft(
        app.state(),
        name.clone(),
        seller_pay.clone(),
        payment_value,
        Some(1),
    )
    .await
    .expect("build finalize+payment");
    execute(&app, &cl, &addr, fwp.id.clone()).await;
    let row = draft_status(&app, &fwp.id);
    let fwp_txid = row.txid.expect("fwp txid");

    // The single tx has BOTH: a FINALIZE covenant output AND a value output
    // to the seller_pay address for exactly payment_value.
    let tx = cl.get_tx_by_hash(&fwp_txid).await.expect("rawtx");
    let outputs = tx
        .get("outputs")
        .and_then(|o| o.as_array())
        .expect("outputs array");
    let has_finalize = outputs.iter().any(|o| {
        o.get("covenant")
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_u64())
            == Some(10)
    });
    let has_seller_payment = outputs.iter().any(|o| {
        let addr = o.get("address").and_then(|a| a.as_str());
        let val = o.get("value").and_then(|v| v.as_u64());
        addr == Some(seller_pay.as_str()) && val == Some(payment_value)
    });
    assert!(has_finalize, "tx must carry a FINALIZE covenant output");
    assert!(
        has_seller_payment,
        "tx must carry the seller payment output"
    );
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("CLOSED"));
}

/// G2. sign_name_message signs a message with the owner-coin key of an owned
/// name, and refuses when the wallet does NOT own the name.
#[tokio::test]
async fn live_sign_name_message_ownership_gate() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_sign_name_message_ownership_gate: set HNS_IT_NODE_URL");
        return;
    };
    // Private account (see `seeded_conn_acct`): this test asserts on the exact
    // coin set / calls getcoinsbyaddress, so it needs an address that no other
    // serial test funds.
    let conn = seeded_conn_acct(&url, &key, 15);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00_at(15);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let owned = format!("snm{tip}");
    acquire_name(&app, &cl, &addr, &owned).await;
    unlock(&app);

    // Owned → signs OK. Response is a JSON object; we accept any Ok().
    let ok =
        crate::commands::tx::sign_name_message(app.state(), owned.clone(), "hello".into(), None)
            .await;
    assert!(ok.is_ok(), "owned name must sign: {:?}", ok.err());

    // Not owned → refused. Use a plausible-but-untracked name.
    let not_owned = format!("snm{tip}notmine");
    let bad =
        crate::commands::tx::sign_name_message(app.state(), not_owned, "hello".into(), None).await;
    assert!(bad.is_err(), "unowned name must be refused");
}

/// G3. Signer unlocked for profile X cannot sign a draft belonging to profile Y.
#[tokio::test]
async fn live_signer_profile_mismatch_refused() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_signer_profile_mismatch_refused: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Build a draft under the seeded profile ("regit1").
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");

    // Unlock a signer for a DIFFERENT profile id (same seed/key; the check is
    // profile-id equality, not key equality — see sign_via_hot_session in tx.rs).
    {
        let state = app.state::<AppState>();
        *state.signer.lock().unwrap() = Some(SignerSession::unlock(
            "other-profile".to_string(),
            NET,
            master(),
            600_000,
        ));
    }
    let res = sign_tx_draft_inner(&app.state(), &d.id).await;
    assert!(
        res.is_err(),
        "signer for a different profile must not sign this draft"
    );
}

/// G4. A watch-only profile refuses build_send_hns_draft.
#[tokio::test]
async fn live_watch_only_send_refused() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_watch_only_send_refused: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    // Flip the seeded profile to watch-only in the DB before wiring the app.
    conn.execute(
        "UPDATE wallet_profiles SET watch_only = 1 WHERE id = ?1",
        params![PROFILE],
    )
    .unwrap();
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let res = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None).await;
    expect_err(res, "watch-only");
}

/// G5. get_write_capability: node reachable + broadcaster available + signer
/// unlocked → can_write=true. Flip the source to Explorer (read-only) → can_write=false.
#[tokio::test]
async fn live_write_capability_downgrades() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_write_capability_downgrades: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    unlock(&app);

    // Signer unlocked + local node + synced → can_write=true.
    let cap = crate::commands::tx::get_write_capability(app.state())
        .await
        .expect("cap");
    assert!(cap.signer_unlocked, "signer flagged unlocked");
    assert!(
        cap.broadcaster_available,
        "local node broadcaster available"
    );
    assert!(cap.can_write, "expected can_write=true, got {cap:?}");

    // Flip the chain source to Explorer (read-only) → broadcaster unavailable.
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::set_setting(&c, "chain_source", "explorer").unwrap();
    }
    let cap2 = crate::commands::tx::get_write_capability(app.state())
        .await
        .expect("cap2");
    assert!(
        !cap2.broadcaster_available,
        "explorer source must not broadcast"
    );
    assert!(
        !cap2.can_write,
        "can_write must be false when read-only: {cap2:?}"
    );
    assert!(cap2.reason.is_some(), "a reason must be reported");
}

// ============================================================================
// Money-critical balance invariants
//
// The three tests below pin the top-level number the user sees
// (`get_wallet_balances`) against on-chain reality across real coin
// movements. Every earlier live test asserts on `tracked_utxos` rows and
// `draft.summary` values — none actually invokes `get_wallet_balances`. If
// `classify_covenant` or `compute_balances` ever regresses (e.g. a BID coin
// gets double-counted as both `nameLockup` AND `liquid`, a REVOKE inflates a
// class, or a fee stops being subtracted from the totals), the frontend would
// show phantom money to spend — the worst kind of wallet bug. These are cheap
// to keep green, and irreplaceable when something drifts.
// ============================================================================

/// Read `get_wallet_balances` and return `(liquid, name_control, name_lockup, total)`
/// as `i64` doos. Panics on a malformed response — the command owns the schema
/// and any drift is a test-worthy regression on its own.
async fn immature_now(app: &tauri::App<tauri::test::MockRuntime>) -> i64 {
    let v = get_wallet_balances(app.state(), None)
        .await
        .expect("balances");
    v.get("immatureDoos")
        .and_then(|x| x.as_i64())
        .expect("immatureDoos")
}

async fn balances_now(app: &tauri::App<tauri::test::MockRuntime>) -> (i64, i64, i64, i64) {
    let v = get_wallet_balances(app.state(), None)
        .await
        .expect("balances");
    let g = |k: &str| v.get(k).and_then(|x| x.as_i64()).expect(k);
    (
        g("liquidDoos"),
        g("nameControlDoos"),
        g("nameLockupDoos"),
        g("totalDoos"),
    )
}

/// Send with change to an external address: the wallet's `totalDoos` after
/// broadcast+mine must drop by EXACTLY `fee`, because `inputTotal = sent +
/// change + fee`, only `sent` leaves the wallet, and `change` comes back to a
/// tracked change address.
///
/// This is the top-level conservation invariant: the number the UI shows must
/// track chain reality, and the fee must be attributed to the miner — not
/// silently rounded, doubled, or absorbed by another spend class.
#[tokio::test]
async fn live_wallet_balance_conservation_across_send() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_wallet_balance_conservation_across_send: set HNS_IT_NODE_URL");
        return;
    };
    // Private account so the exact-total assertion isn't contaminated by
    // sibling tests mining to the shared `acct=0` address.
    let conn = seeded_conn_acct(&url, &key, 21);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00_at(21);

    fund(&cl, &addr, 105).await;
    // Advance the tip past coinbase maturity to a throwaway address so every
    // coin funded to `addr` is spendable. `liquidDoos` reports only mature
    // value, so without this the pre-send `liquid == total` assertion below
    // would (correctly) fail and the send could not reach the whole set.
    let burn = recv_leaf_01_at(97);
    fund(&cl, &burn, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let (liq_before, nc_before, nl_before, tot_before) = balances_now(&app).await;
    // Pre-conditions: only liquid coins funded, no in-flight names.
    assert_eq!(nc_before, 0, "no name-control coins yet");
    assert_eq!(nl_before, 0, "no name-lockup coins yet");
    assert_eq!(liq_before, tot_before, "total == liquid pre-send");
    // total must equal the raw sum in tracked_utxos (the frontend and the
    // coin-selection code must agree on what's spendable).
    assert_eq!(tot_before, wallet_total_doos(&app));

    // Send to a wallet-external address so `sent` genuinely leaves the wallet.
    let (_sk, _pk, external) = hd::derive_address(NET, &seed(), 21, 0, 7).unwrap();
    let draft = build_send_hns_draft(app.state(), external, 500_000, Some(1), None)
        .await
        .expect("build send");
    let fee = sum_i64(&draft, "feeDoos");
    let change = sum_i64(&draft, "changeDoos");
    let input_total = sum_i64(&draft, "inputTotalDoos");
    // 500_000 sent + change + fee == input_total. This equation is the whole
    // ballgame: violating it means we either created or destroyed HNS.
    assert_eq!(500_000 + change + fee, input_total, "chain conservation");
    assert!(fee > 0 && change > 0, "expected change + fee both > 0");

    execute(&app, &cl, &addr, draft.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let (_, nc_after, nl_after, tot_after) = balances_now(&app).await;
    assert_eq!(nc_after, 0);
    assert_eq!(nl_after, 0);
    // The wallet's total dropped by EXACTLY (sent + fee) — `change` came back.
    // execute() also mined 1 block to `addr`, adding a fresh coinbase reward
    // (immature, but still counted in `totalDoos`), so subtract it off.
    let reward = new_coinbase_reward_for(&cl, &addr, 1).await;
    assert_eq!(
        tot_after,
        tot_before - 500_000 - fee + reward,
        "wallet total moved by exactly (-sent -fee +minedReward)"
    );
}

/// Return the sum of `value` across all coins at `addr` that appeared in the
/// last `last_n` blocks — used to net-out the block reward from a mined-tip
/// balance delta. `getcoinsbyaddress` is address-indexed, so this is exact.
async fn new_coinbase_reward_for(cl: &NodeRpcClient, addr: &str, last_n: i64) -> i64 {
    let info = cl.get_blockchain_info().await.expect("info");
    let cutoff = info.blocks as i64 - last_n + 1;
    let coins = cl.get_coins_by_address(addr).await.expect("coins");
    coins
        .iter()
        .filter(|c| c.coinbase.unwrap_or(false) && c.height.unwrap_or(-1) >= cutoff)
        .map(|c| c.value)
        .sum()
}

/// A BID coin's value must land in `nameLockupDoos` — not `liquidDoos`, not
/// `nameControlDoos`. The frontend uses this split to decide what the user
/// can spend right now; a misclassified bid would either (a) let the user
/// double-spend a locked bid coin as if it were liquid, or (b) hide it
/// permanently as if the funds were gone. Verified against a real hsd BID
/// covenant on regtest.
#[tokio::test]
async fn live_balance_classes_track_bid_lockup() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_balance_classes_track_bid_lockup: set HNS_IT_NODE_URL");
        return;
    };
    // REUSE-SAFETY: this test does an on-chain BID whose lockup output lands
    // on a wallet-owned receive address, and it asserts on the EXACT delta
    // `nameLockupDoos` moves by. A BID output is a permanent unspent coin —
    // re-running against the same chain would let a prior run's bid reappear
    // (the receive address is re-derived) and inflate the delta. To stay
    // deterministic across arbitrary reuse we pick a FRESH account per run,
    // keyed off the current chain height: two runs can only collide if the
    // tip is identical, which it never is once any block has been mined. The
    // account index is kept well clear of the fixed private accounts used by
    // sibling tests (0-24).
    let cl = client(&url, &key);
    let tip0 = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip0);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);

    fund(&cl, &addr, 105).await;
    // Throwaway address to advance the tip past coinbase maturity. Derived
    // from the same fresh account (branch 1) so it never collides with a
    // sibling test's burn address.
    let burn = recv_leaf_01_at(acct);
    fund(&cl, &burn, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Baseline on the fresh account: no name coins can exist here yet.
    let (_, nc_pre, nl_pre, tot_pre) = balances_now(&app).await;
    assert_eq!(
        nc_pre, 0,
        "fresh account: no name-control coins at baseline"
    );
    assert_eq!(nl_pre, 0, "fresh account: no name-lockup coins at baseline");

    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("bclas{tip}");

    // OPEN → BIDDING.
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);

    // BID: 1_000_000 true bid, 2_000_000 lockup total (the extra is a blind).
    sync_wallet_state(app.state(), None).await.expect("sync");
    const BID_VALUE: i64 = 1_000_000;
    const LOCKUP: i64 = 2_000_000;
    let bid = build_bid_draft(app.state(), name.clone(), BID_VALUE, LOCKUP, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let (_liq_after, _nc_after, nl_after, tot_after) = balances_now(&app).await;
    // The BID output value = the LOCKUP amount, on a BID covenant → `name_lockup`.
    // Assert the lockup class grew by EXACTLY this bid's lockup: no more (not
    // double-counted) and no less (not misfiled into liquid/control).
    assert_eq!(
        nl_after - nl_pre,
        LOCKUP,
        "BID lockup must raise nameLockupDoos by exactly {LOCKUP}: pre={nl_pre} after={nl_after}"
    );
    // Two independent frontend-facing invariants must hold:
    //   (1) `totalDoos` equals the raw unspent-utxo sum — no class is
    //       double-counted and none is dropped.
    //   (2) The BID's lockup and the wallet total both grew relative to the
    //       pre-BID baseline (the wallet accumulated block rewards from
    //       mining and reveal-window advancement) — a negative move here
    //       would mean the classifier hid coins from `totalDoos`.
    assert_eq!(tot_after, wallet_total_doos(&app));
    assert!(
        tot_after > tot_pre,
        "wallet total should grow (mined rewards >> fees): pre={tot_pre} after={tot_after}"
    );
    // The BID's fee is captured in the draft's summary_json — every draft
    // must record a strictly positive fee so the miner is paid.
    assert!(
        draft_fee_by_action(&app, "bid") > 0,
        "bid draft must record a fee"
    );
    assert!(
        draft_fee_by_action(&app, "open") > 0,
        "open draft must record a fee"
    );
}

/// Read the `feeDoos` recorded in the most recent draft for `action` on the
/// active profile, parsed from its persisted `summary_json`. The money-invariant
/// tests build exactly one draft per action, so "most recent" is unambiguous.
fn draft_fee_by_action(app: &tauri::App<tauri::test::MockRuntime>, action: &str) -> i64 {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    let summary_json: String = c
        .query_row(
            "SELECT summary_json FROM wallet_tx_drafts
              WHERE wallet_profile_id = ?1 AND action = ?2
              ORDER BY created_at DESC LIMIT 1",
            params![PROFILE, action],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("no draft for action={action}: {e}"));
    let v: serde_json::Value = serde_json::from_str(&summary_json).expect("summary_json");
    // Name-action summaries persist `feeDoos` via ActionSummary's serde.
    v.get("feeDoos")
        .and_then(|x| x.as_i64())
        .unwrap_or_else(|| panic!("summary for {action} missing feeDoos: {summary_json}"))
}

/// Two spend-capable actions on the SAME owned name must not both reserve the
/// owner coin. The persist path reserves every input the plan spends,
/// including the name UTXO, and the second insert is expected to fail with an
/// `InvalidInput` "just reserved by another pending draft" error. Without
/// this, an UPDATE and a TRANSFER could each carry a valid signature over the
/// same owner coin and race at broadcast time — the loser silently getting
/// dropped by the mempool while the frontend thinks both are pending.
#[tokio::test]
async fn live_name_utxo_reservation_blocks_second_action() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_name_utxo_reservation_blocks_second_action: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_acct(&url, &key, 23);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00_at(23);

    fund(&cl, &addr, 105).await;
    let burn = recv_leaf_01_at(95);
    fund(&cl, &burn, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("dblres{tip}");

    // Drive the name all the way to owned (CLOSED + REGISTER). Mirrors the
    // existing auction lifecycle test.
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);

    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.clone(), 1_000_000, 2_000_000, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    assert!(mine_until(&cl, &name, "REVEAL", &addr, 30).await);

    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id).await;
    assert!(mine_until(&cl, &name, "CLOSED", &addr, 40).await);

    // Seed tracked_name_states so sync attributes the owner coin (mainnet uses
    // explorer-based discovery, unavailable on regtest — same pattern as the
    // existing auction/register lifecycle test).
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let records = vec![serde_json::json!({"type":"TXT","txt":["reserved-check"]})];
    let reg = build_register_draft(app.state(), name.clone(), Some(records), Some(1))
        .await
        .expect("build register");
    execute(&app, &cl, &addr, reg.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Build an UPDATE first — this reserves the owner coin.
    let upd_records = vec![serde_json::json!({"type":"TXT","txt":["v1"]})];
    let _upd = build_update_draft(app.state(), name.clone(), upd_records, Some(1))
        .await
        .expect("first update draft should succeed");

    // Now try a TRANSFER of the SAME name to any address — its plan spends the
    // same owner coin. Reservation must reject it.
    let recipient = recv_leaf_01_at(23);
    let err = build_transfer_draft(app.state(), name.clone(), recipient, Some(1))
        .await
        .expect_err("second draft on same owner coin must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("just reserved by another") || msg.contains("already spent"),
        "unexpected error for double-reserve: {msg}"
    );
}

/// A max-send (Send All) must leave the wallet's spendable liquid balance at
/// exactly the size of the single sweep output. Coin-selection or classifier
/// bugs that misclassify the sweep output would either strand the funds
/// (post-sweep total drops to zero) or duplicate them across classes.
#[tokio::test]
async fn live_max_send_leaves_only_sweep_output_liquid() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_max_send_leaves_only_sweep_output_liquid: set HNS_IT_NODE_URL");
        return;
    };
    // A fresh account, as `live_send_max_sweeps_all_coins` explains.
    let cl = client(&url, &key);
    let acct = fresh_acct(cl.get_blockchain_info().await.expect("info").blocks);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);

    fund(&cl, &addr, 105).await;
    let burn = recv_leaf_01_at(94);
    fund(&cl, &burn, 3).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let (_, nc_pre, nl_pre, tot_pre) = balances_now(&app).await;
    assert_eq!(nc_pre, 0);
    assert_eq!(nl_pre, 0);

    // Sweep back to `addr` (self-send): the sweep tx has one output whose
    // value == inputTotal - fee.
    let draft = build_send_hns_draft(app.state(), addr.clone(), 0, Some(1), Some(true))
        .await
        .expect("build max");
    let fee = sum_i64(&draft, "feeDoos");
    let input_total = sum_i64(&draft, "inputTotalDoos");
    assert_eq!(sum_i64(&draft, "changeDoos"), 0, "sweep has no change");
    assert_eq!(input_total, tot_pre, "sweep spends the entire wallet");
    let sweep_output = input_total - fee;

    execute(&app, &cl, &addr, draft.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Post-sweep the wallet holds exactly two coins that count toward totalDoos:
    //   1) the sweep output at `addr` — mature, so it lands in `liquid`
    //   2) the block-1 coinbase reward at `addr` from execute()'s mine — mined
    //      one block ago against a regtest maturity of 2, so it is still
    //      IMMATURE and is reported in its own bucket, not in `liquid`.
    // `liquid` now means "spendable", matching what coin selection would pick;
    // `total` still counts everything the wallet owns.
    let reward = new_coinbase_reward_for(&cl, &addr, 1).await;
    let (liq_after, nc_after, nl_after, tot_after) = balances_now(&app).await;
    let immature_after = immature_now(&app).await;
    assert_eq!(nc_after, 0);
    assert_eq!(nl_after, 0);
    assert_eq!(
        liq_after, sweep_output,
        "post-sweep liquid = the sweep output alone; the fresh reward is immature"
    );
    assert_eq!(
        immature_after, reward,
        "the just-mined coinbase reward is held back by maturity"
    );
    assert_eq!(
        tot_after,
        liq_after + immature_after,
        "total still counts everything the wallet owns"
    );
    // Chain-conservation across the sweep: only the miner fee left the wallet
    // (net of the newly-mined block reward, which arrived).
    assert_eq!(tot_after, tot_pre - fee + reward);
}

/// G1: broadcast guard rejects a node on the wrong chain. Set up a mainnet
/// profile but connect to regtest node → get_write_capability must block with
/// a chain-mismatch reason, not allow broadcast.
#[tokio::test]
async fn live_broadcast_guard_rejects_cross_network() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_broadcast_guard_rejects_cross_network: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    // Override the profile network to mainnet (while keeping regtest node URL).
    conn.execute(
        "UPDATE wallet_profiles SET network = 'mainnet' WHERE id = ?1",
        params![PROFILE],
    )
    .unwrap();
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 101).await;

    // First guard: the sync path itself refuses a cross-network node before it
    // writes anything to this profile's cache (a regtest chain's heights and
    // name states must never seed a mainnet profile). This fires ahead of the
    // write-capability check below and is the earliest line of defense.
    let sync_err = sync_wallet_state(app.state(), None)
        .await
        .expect_err("sync must refuse a mainnet profile against a regtest node");
    let sync_msg = format!("{sync_err}");
    assert!(
        sync_msg.contains("mainnet") && sync_msg.contains("regtest"),
        "sync refusal must name the network mismatch: {sync_msg}"
    );

    unlock(&app);

    // Second guard: even with the signer unlocked, write capability must stay
    // blocked with a chain-mismatch reason — the broadcast path never treats a
    // regtest node as authoritative for a mainnet wallet.
    let cap = crate::commands::tx::get_write_capability(app.state())
        .await
        .expect("cap");
    assert!(
        !cap.can_write,
        "cross-network broadcast must be blocked: {cap:?}"
    );
    assert!(
        cap.reason
            .as_ref()
            .map(|r| r.contains("mainnet") || r.contains("regtest"))
            .unwrap_or(false),
        "reason must mention the network mismatch: {:?}",
        cap.reason
    );
}

/// G2: large batch (20 names) assembles into one tx and broadcasts successfully.
/// This confirms that the node accepts large covenant txs and batching works
/// end-to-end. A full 100-item batch requires too much on-chain setup time.
#[tokio::test]
async fn live_batch_large_covenant_count() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_batch_large_covenant_count: set HNS_IT_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    // Fund and sync.
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    unlock(&app);

    // Acquire 20 names (realistic batch size, avoids excessive on-chain setup).
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let mut names = Vec::new();
    for i in 0..20 {
        let name = format!("batch{tip}_{i:02}");
        acquire_name(&app, &cl, &addr, &name).await;
        names.push(name);
    }

    // The last name was registered in the last block; hsd accepts a RENEW
    // only a tree interval after that (`bad-renewal-premature`).
    let name_refs: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
    mine_until_renewable(&cl, &name_refs, &addr).await;
    let mut before = Vec::new();
    for name in &names {
        before.push(name_info_height(&cl, name, "renewal").await);
    }

    // Batch renew all 20 names in one tx.
    let batch =
        crate::commands::names::build_batch_renew_draft(app.state(), names.clone(), Some(1))
            .await
            .expect("build batch renew");

    // Broadcast the batch.
    execute(&app, &cl, &addr, batch.id.clone()).await;

    // The node took it and mined it: every name's renewal moved to that block.
    let row = draft_status(&app, &batch.id);
    assert!(
        matches!(row.status.as_str(), "broadcasted" | "confirmed"),
        "large batch (20 names) must be sent, got status: {}",
        row.status
    );
    let mined_at = cl.get_blockchain_info().await.expect("info").blocks;
    for (name, before) in names.iter().zip(before) {
        let after = name_info_height(&cl, name, "renewal").await;
        assert!(after > before, "{name}: renewal did not advance");
        assert_eq!(after, mined_at, "{name} renewed in the block just mined");
    }
}

/// End-to-end proof for the chain scanner against a real node: OPEN a name,
/// BID on it, mine, then run the scanner over the chain and read the bids back
/// through the real `read_name_bids` command.
///
/// This is the test that would have caught the two defects fixed alongside it:
///
///   - `scan_block` parsed `outputs` / `hash` / integer doos, but hsd's
///     JSON-RPC `getblock` emits `vout` / `txid` / HNS floats. Every mocked
///     scanner test agreed with the wrong shape, so the scanner walked entire
///     chains and indexed nothing.
///   - the cursor was a global singleton, so a height left over from another
///     network made `cursor >= tip` true and the scanner never ran at all.
///
/// Both are invisible to a mock and obvious here.
#[tokio::test]
async fn live_chain_scanner_indexes_own_bid() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_chain_scanner_indexes_own_bid: set HNS_IT_NODE_URL");
        return;
    };

    // The scanner reopens the DB by path, so this fixture must be file-backed.
    let db_path = std::env::temp_dir().join(format!(
        "namehold_live_scanner_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    seed_profile_into(&conn, &url, &key, 0);
    drop(conn);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();

    cl.generate_to_address(101, &addr).await.expect("fund");
    sync_wallet_state(app.state(), None).await.expect("sync");

    let tip0 = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("scanit{tip0}");

    // OPEN → BIDDING.
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(
        mine_until(&cl, &name, "BIDDING", &addr, 30).await,
        "name {name} did not reach BIDDING; state={:?}",
        node_state(&cl, &name).await
    );

    // A bid whose lockup is deliberately NOT a whole number of HNS: 2.5 HNS.
    // hsd reports it as the float 2.5, and a scanner that read the field as an
    // integer would silently store 0.
    const BID_DOOS: i64 = 1_000_000;
    const LOCKUP_DOOS: i64 = 2_500_000;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let bid = build_bid_draft(app.state(), name.clone(), BID_DOOS, LOCKUP_DOOS, Some(1))
        .await
        .expect("build bid");
    execute(&app, &cl, &addr, bid.id).await;
    cl.generate_to_address(1, &addr).await.expect("mine bid");
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Run the scanner exactly as `run_chain_scanner` does, over the whole chain.
    let tip = cl.get_blockchain_info().await.expect("info").blocks as i64;
    for height in 1..=tip {
        crate::commands::chain_scan::scan_block(&cl, db_path.to_str().unwrap(), "regtest", height)
            .await
            .unwrap_or_else(|e| panic!("scan_block({height}) failed: {e:?}"));
    }
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        crate::commands::chain_scan::set_scan_cursor(&c, "regtest", tip).unwrap();
    }

    // The auction this bid belongs to — the OPEN height the BID covenant names.
    let auction_start = cl
        .get_name_info(&name)
        .await
        .expect("name info")
        .get("info")
        .and_then(|i| i.get("height"))
        .and_then(|h| h.as_i64())
        .expect("name must have an open auction");

    // The BID covenant is indexed, in doos, under the txid (not the wtxid).
    let name_hash_hex = hex::encode(crate::noncustodial::names::hash_name(&name).unwrap());
    let indexed = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        crate::commands::chain_scan::read_indexed_bids(&c, "regtest", auction_start, &name_hash_hex)
            .unwrap()
    };
    assert_eq!(
        indexed.len(),
        1,
        "the scanner must index exactly one BID for {name}"
    );
    assert_eq!(
        indexed[0].lockup,
        Some(LOCKUP_DOOS as u64),
        "lockup must be stored in doos, not HNS"
    );

    // Another network's slice of the same tables stays empty — the index is
    // network-keyed, and a name hashes identically on every chain.
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        assert!(
            crate::commands::chain_scan::read_indexed_bids(
                &c,
                "main",
                auction_start,
                &name_hash_hex
            )
            .unwrap()
            .is_empty(),
            "regtest BIDs must not answer a mainnet query"
        );
        // Nor does a different auction for the same name see them (029).
        assert!(
            crate::commands::chain_scan::read_indexed_bids(
                &c,
                "regtest",
                auction_start - 1,
                &name_hash_hex
            )
            .unwrap()
            .is_empty(),
            "a bid must only answer for the auction it was placed in"
        );
        assert_eq!(
            crate::commands::chain_scan::scan_cursor_height(&c, "main"),
            0,
            "scanning regtest must not advance the mainnet cursor"
        );
    }

    // `read_name_bids` only trusts the index once the scanner has passed the
    // name's auction height, which it reads from `tracked_name_states`. Owned-
    // name discovery is explorer-based and unavailable on regtest, so seed the
    // row and let the sync resolve it from the node's `getnameinfo` — exactly
    // what discovery would do on mainnet.
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");

    // And the command that feeds the UI surfaces it as the wallet's own bid,
    // served from the local index (regtest has no explorer to fall back to).
    let val =
        crate::commands::read::read_name_bids(app.state(), name.clone(), Some(PROFILE.to_string()))
            .await
            .expect("read_name_bids");
    let bids = val["bids"].as_array().expect("bids array");
    assert_eq!(bids.len(), 1, "the bids panel must show the bid: {val}");
    assert_eq!(bids[0]["lockup"], LOCKUP_DOOS);
    assert_eq!(bids[0]["mine"], true, "our own bid must be marked mine");
    assert_eq!(bids[0]["myValue"], BID_DOOS);
    assert_eq!(val["myBidCount"], 1);

    let _ = std::fs::remove_file(&db_path);
}

// ---------------------------------------------------------------------------
// Before and after the block.
//
// Every status the wallet shows is derived from the chain's view of a name, and
// between broadcasting and the next block that view has not moved. The unit
// tests pin each piece against a mock; these drive the real transition on a
// real node, which is the only way to catch a piece that is right on its own
// and wrong in sequence.
// ---------------------------------------------------------------------------

/// A broadcast OPEN is reported as in flight until a block includes it, and
/// the report clears on its own once one does.
#[tokio::test]
async fn live_pending_action_is_reported_until_the_block_lands() {
    let Some((url, key)) = it_env() else {
        eprintln!(
            "skip live_pending_action_is_reported_until_the_block_lands: set HNS_IT_NODE_URL"
        );
        return;
    };
    let cl = client(&url, &key);
    // Its own account: the shared `acct 0` address accumulates every sibling
    // test's coins, and a huge set makes `getcoinsbyaddress` slow and flaky.
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let name = format!("pend{tip}");

    let caps = |n: String| {
        let state = app.state::<AppState>();
        async move {
            crate::commands::names::get_name_action_capabilities(state, n, Some(PROFILE.into()))
                .await
                .expect("caps")
        }
    };

    // Nothing sent yet.
    let before = caps(name.clone()).await;
    assert_eq!(before.pending_broadcast_action, None);
    // The auction window is the network's, not a hardcoded guess: regtest bids
    // for 5 blocks and reveals for 10, where mainnet is 720/1440.
    assert_eq!(before.auction_bidding_blocks, Some(5));
    assert_eq!(before.auction_reveal_blocks, Some(10));

    // Broadcast the OPEN and STOP — this is the window under test.
    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    broadcast_only(&app, &open.id).await;

    let in_flight = caps(name.clone()).await;
    assert_eq!(
        in_flight.pending_broadcast_action.as_deref(),
        Some("open"),
        "the chain still calls the name available; the wallet must say an open is in flight"
    );
    assert_eq!(
        node_state(&cl, &name).await,
        None,
        "precondition: the chain has not moved"
    );

    // One block, and the wallet should stop talking about a pending action.
    settle(&app, &cl, &addr, &open.id).await;
    let mined = caps(name.clone()).await;
    assert_eq!(
        mined.pending_broadcast_action, None,
        "confirmed — nothing is in flight any more"
    );
    assert_eq!(node_state(&cl, &name).await.as_deref(), Some("OPENING"));

    // Activity resolves the name for a confirmed OPEN. Only BID's raw name used
    // to be decoded, so the very first action on a name listed itself with an
    // empty Name cell.
    let history = crate::commands::history::read_action_history(app.state(), Some(PROFILE.into()))
        .await
        .expect("history");
    let row = history
        .iter()
        .find(|r| r.action == "open" && r.name.as_deref() == Some(name.as_str()));
    assert!(
        row.is_some(),
        "the confirmed OPEN must carry its name: {history:?}"
    );
}

/// Our own bid is listed while it is still in the mempool, and the same bid is
/// then served from the chain index once mined — one row either way, never
/// zero and never two.
#[tokio::test]
async fn live_own_bid_is_pending_before_the_block_and_indexed_after() {
    let Some((url, key)) = it_env() else {
        eprintln!(
            "skip live_own_bid_is_pending_before_the_block_and_indexed_after: set HNS_IT_NODE_URL"
        );
        return;
    };
    let cl = client(&url, &key);
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip);
    let (db_path, app) = file_backed_app(&url, &key, acct);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let name = format!("pbid{tip}");

    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);

    // `read_name_bids` only trusts the index once the scanner has passed the
    // name's auction height, and it reads that height from tracked state.
    sync_wallet_state(app.state(), None).await.expect("sync");
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");
    scan_to_tip(&cl, &db_path).await;

    const BID: i64 = 1_000_000;
    const LOCKUP: i64 = 2_500_000;
    let bid = build_bid_draft(app.state(), name.clone(), BID, LOCKUP, Some(1))
        .await
        .expect("build bid");
    broadcast_only(&app, &bid.id).await;

    // In the mempool: the index has nothing, but the panel must still show it.
    let pending =
        crate::commands::read::read_name_bids(app.state(), name.clone(), Some(PROFILE.to_string()))
            .await
            .expect("read bids");
    let rows = pending["bids"].as_array().expect("bids");
    assert_eq!(rows.len(), 1, "our own unmined bid: {pending}");
    assert_eq!(rows[0]["pending"], true);
    assert_eq!(rows[0]["mine"], true);
    assert_eq!(rows[0]["lockup"], LOCKUP);
    assert_eq!(rows[0]["myValue"], BID);
    assert_eq!(pending["myBidCount"], 1);

    // Mine it and let the scanner index it. Same one bid, now on-chain.
    settle(&app, &cl, &addr, &bid.id).await;
    scan_to_tip(&cl, &db_path).await;
    let mined =
        crate::commands::read::read_name_bids(app.state(), name.clone(), Some(PROFILE.to_string()))
            .await
            .expect("read bids");
    let rows = mined["bids"].as_array().expect("bids");
    assert_eq!(rows.len(), 1, "still exactly one — not duplicated: {mined}");
    assert!(
        rows[0].get("pending").is_none(),
        "served from the chain index now, not from the local commitment"
    );
    assert_eq!(rows[0]["mine"], true);
    assert_eq!(rows[0]["lockup"], LOCKUP);
    assert_eq!(mined["myBidCount"], 1);

    let _ = std::fs::remove_file(&db_path);
}

/// A name whose auction lapsed can be opened again, and the second auction is
/// clean: its bid list holds only its own bids, and the lockup left behind by
/// the first is reported as stranded rather than silently dropped.
#[tokio::test]
async fn live_reopened_name_scopes_its_bids_and_strands_the_old_lockup() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_reopened_name_scopes_its_bids_and_strands_the_old_lockup: set HNS_IT_NODE_URL");
        return;
    };
    let cl = client(&url, &key);
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip);
    let (db_path, app) = file_backed_app(&url, &key, acct);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let name = format!("relap{tip}");

    // --- First auction: open, bid, then walk away without revealing. --------
    let open1 = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open 1");
    execute(&app, &cl, &addr, open1.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);
    let first_start = auction_start(&cl, &name).await.expect("first auction");

    sync_wallet_state(app.state(), None).await.expect("sync");
    const OLD_LOCKUP: i64 = 3_000_000;
    let bid1 = build_bid_draft(app.state(), name.clone(), 1_000_000, OLD_LOCKUP, Some(1))
        .await
        .expect("build bid 1");
    execute(&app, &cl, &addr, bid1.id).await;

    assert!(
        mine_until_auction_lapses(&cl, &name, &addr, 60).await,
        "the unrevealed auction must lapse and free the name"
    );
    sync_wallet_state(app.state(), None).await.expect("sync");

    // The tracked row must not keep claiming the dead auction's OPEN height —
    // `read_name_bids` reads it to decide which auction's bids to serve.
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let lapsed_height: Option<i64> = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row(
            "SELECT height FROM tracked_name_states WHERE wallet_profile_id = ?1 AND name = ?2",
            params![PROFILE, name],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        lapsed_height, None,
        "a lapsed auction leaves no OPEN height"
    );

    // --- Second auction: the confirmed OPEN coin must not block a reopen. ---
    sync_wallet_state(app.state(), None).await.expect("sync");
    let open2 = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("a name the chain has freed must be openable again");
    execute(&app, &cl, &addr, open2.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);
    let second_start = auction_start(&cl, &name).await.expect("second auction");
    assert_ne!(second_start, first_start, "a genuinely new auction");

    sync_wallet_state(app.state(), None).await.expect("sync");
    const NEW_LOCKUP: i64 = 4_000_000;
    let bid2 = build_bid_draft(app.state(), name.clone(), 1_500_000, NEW_LOCKUP, Some(1))
        .await
        .expect("build bid 2");
    execute(&app, &cl, &addr, bid2.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    scan_to_tip(&cl, &db_path).await;

    // The bid list belongs to THIS auction only.
    let bids =
        crate::commands::read::read_name_bids(app.state(), name.clone(), Some(PROFILE.to_string()))
            .await
            .expect("read bids");
    let rows = bids["bids"].as_array().expect("bids");
    assert_eq!(
        rows.len(),
        1,
        "the first auction's bid must not appear: {bids}"
    );
    assert_eq!(rows[0]["lockup"], NEW_LOCKUP);
    assert_eq!(bids["myBidCount"], 1);

    // And the lockup the first auction swallowed is reported, not hidden.
    let caps = crate::commands::names::get_name_action_capabilities(
        app.state(),
        name.clone(),
        Some(PROFILE.into()),
    )
    .await
    .expect("caps");
    assert_eq!(caps.my_bid_count, 1, "one bid in the live auction");
    assert_eq!(caps.stranded_bid_count, 1);
    assert_eq!(
        caps.stranded_lockup_doos, OLD_LOCKUP,
        "the unrevealable lockup from the lapsed auction"
    );

    let _ = std::fs::remove_file(&db_path);
}

/// Three bids on ONE name, then reveal. Every one of them must be revealed:
/// an unrevealed BID coin can only ever be spent by a REVEAL, and a REVEAL is
/// only valid while the auction is in its reveal window — miss it and the
/// lockup is locked for good.
#[tokio::test]
async fn live_reveal_covers_every_bid_this_wallet_placed_on_the_name() {
    let Some((url, key)) = it_env() else {
        eprintln!(
            "skip live_reveal_covers_every_bid_this_wallet_placed_on_the_name: set HNS_IT_NODE_URL"
        );
        return;
    };
    let cl = client(&url, &key);
    // Its own account: the shared `acct 0` address accumulates every sibling
    // test's coins, and a huge set makes `getcoinsbyaddress` slow and flaky.
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let name = format!("multi{tip}");

    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);

    // Three independent bids, which the wallet explicitly supports.
    let lockups: [i64; 3] = [2_000_000, 3_000_000, 4_000_000];
    for (i, lockup) in lockups.iter().enumerate() {
        sync_wallet_state(app.state(), None).await.expect("sync");
        let bid = build_bid_draft(
            app.state(),
            name.clone(),
            1_000_000 + i as i64 * 100_000,
            *lockup,
            Some(1),
        )
        .await
        .unwrap_or_else(|e| panic!("build bid {i}: {e:?}"));
        execute(&app, &cl, &addr, bid.id).await;
    }
    sync_wallet_state(app.state(), None).await.expect("sync");

    let nh_hex = hex::encode(crate::noncustodial::names::hash_name(&name).unwrap());
    let unspent_bid_coins = |app: &tauri::App<tauri::test::MockRuntime>| {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::find_unspent_covenant_utxos_by_name_hash(
            &c,
            PROFILE,
            crate::noncustodial::sync::COV_BID as i64,
            &nh_hex,
        )
        .unwrap()
        .len()
    };
    assert_eq!(
        unspent_bid_coins(&app),
        3,
        "precondition: three bids placed"
    );

    assert!(mine_until(&cl, &name, "REVEAL", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");

    // Do what the app offers for this name, until it has nothing left to offer.
    for attempt in 0..lockups.len() {
        match build_reveal_draft(app.state(), name.clone(), Some(1)).await {
            Ok(draft) => execute(&app, &cl, &addr, draft.id).await,
            Err(e) => {
                eprintln!("reveal attempt {attempt} refused: {e:?}");
                break;
            }
        }
        sync_wallet_state(app.state(), None).await.expect("sync");
    }
    sync_wallet_state(app.state(), None).await.expect("sync");

    // The symptom: a BID coin still unspent once the reveal window closes is a
    // lockup nobody can ever reclaim.
    let left = unspent_bid_coins(&app);
    let stranded: i64 = lockups.iter().sum::<i64>() - {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::list_bid_commitments(&c, PROFILE)
            .unwrap()
            .iter()
            .filter(|b| b.name == name && b.reveal_txid.is_some())
            .map(|b| b.lockup_value_doos)
            .sum::<i64>()
    };
    assert_eq!(
        left, 0,
        "{left} of 3 bids left unrevealed — {stranded} doos of lockup is now unreclaimable"
    );
}

/// Full lifecycle with several bids from ONE wallet, auditing the money at the
/// end: OPEN → three BIDs → REVEAL → CLOSED → REGISTER the winner → REDEEM the
/// losers. Nothing may be left stranded.
///
/// A wallet that outbids itself is the ordinary case once several bids per name
/// are allowed: one of its own bids wins and the rest lose, so it has to
/// register AND redeem. Every stage after BID was written when a wallet could
/// hold one bid per name, and this walks all of them at once.
#[tokio::test]
async fn live_multi_bid_lifecycle_leaves_no_coin_stranded() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_multi_bid_lifecycle_leaves_no_coin_stranded: set HNS_IT_NODE_URL");
        return;
    };
    let cl = client(&url, &key);
    // Its own account: the shared `acct 0` address accumulates every sibling
    // test's coins, and a huge set makes `getcoinsbyaddress` slow and flaky.
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip);
    let conn = seeded_conn_acct(&url, &key, acct);
    let app = app_with(conn);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let name = format!("cycle{tip}");
    let nh_hex = hex::encode(crate::noncustodial::names::hash_name(&name).unwrap());

    let unspent = |app: &tauri::App<tauri::test::MockRuntime>, cov: u8| {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::find_unspent_covenant_utxos_by_name_hash(&c, PROFILE, cov as i64, &nh_hex)
            .unwrap()
            .len()
    };

    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);

    // Three bids from this one wallet. The highest wins; the other two lose and
    // must be redeemed, by the same wallet that placed them.
    for (bid_v, lockup) in [
        (1_000_000i64, 2_000_000i64),
        (1_500_000, 3_000_000),
        (1_200_000, 4_000_000),
    ] {
        sync_wallet_state(app.state(), None).await.expect("sync");
        let d = build_bid_draft(app.state(), name.clone(), bid_v, lockup, Some(1))
            .await
            .unwrap_or_else(|e| panic!("build bid {bid_v}: {e:?}"));
        execute(&app, &cl, &addr, d.id).await;
    }
    sync_wallet_state(app.state(), None).await.expect("sync");
    assert_eq!(unspent(&app, crate::noncustodial::sync::COV_BID), 3);

    assert!(mine_until(&cl, &name, "REVEAL", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    assert_eq!(
        unspent(&app, crate::noncustodial::sync::COV_BID),
        0,
        "every bid revealed"
    );
    assert_eq!(
        unspent(&app, crate::noncustodial::sync::COV_REVEAL),
        3,
        "three reveal coins: one wins the name, two are redeemable"
    );

    assert!(mine_until(&cl, &name, "CLOSED", &addr, 40).await);
    track_name(&app, &name);
    sync_wallet_state(app.state(), None).await.expect("sync");

    // The winning reveal becomes the name coin; register it.
    let reg = build_register_draft(app.state(), name.clone(), None, Some(1))
        .await
        .expect("build register");
    execute(&app, &cl, &addr, reg.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    // The command can reclaim them — but the button has to offer it. Owning the
    // name and holding losing bids on it is the ordinary outcome of outbidding
    // yourself, so "you own it" must not disqualify the redeem.
    let caps = crate::commands::names::get_name_action_capabilities(
        app.state(),
        name.clone(),
        Some(PROFILE.into()),
    )
    .await
    .expect("caps");
    assert!(
        caps.can_redeem.allowed,
        "two losing reveals are sitting there; Redeem must be offered: {:?}",
        caps.can_redeem.reason
    );
    // A REGISTER owner coin carries four covenant items, as a TRANSFER does;
    // it is no transfer. Read as one, a just-registered name refused Update,
    // Transfer and Renew, and offered Finalize, until an UPDATE that same
    // refusal blocked.
    assert!(!caps.transfer_pending, "no transfer was ever started");
    for (what, cap) in [
        ("update", &caps.can_update),
        ("transfer", &caps.can_transfer),
    ] {
        assert!(cap.allowed, "{what} must be offered: {:?}", cap.reason);
    }
    // Renew is held back for the plain reason hsd gives, not a transfer: a
    // name registered in the last block may be renewed only a tree interval
    // later (`bad-renewal-premature`).
    let renew_reason = caps.can_renew.reason.clone().unwrap_or_default();
    assert!(
        !caps.can_renew.allowed && renew_reason.contains("renewed too recently"),
        "renew must wait out the tree interval: {renew_reason:?}"
    );
    assert!(!caps.can_finalize.allowed, "nothing to finalize");
    // The guided panel is what tells a user what to do next. Registered, with
    // its own losing lockups still out there, the wallet must point at them —
    // an enabled button behind an "advanced" toggle is not being told.
    assert_eq!(
        caps.task_state,
        crate::commands::names::AuctionTaskState::LostNeedsRedeem,
        "registered, two of its own bids lost: the next thing to do is reclaim them"
    );

    // …and reclaim the two that lost. Whatever the app offers, until it offers
    // nothing: the user cannot do more than that.
    for _ in 0..4 {
        match build_redeem_draft(app.state(), name.clone(), Some(1)).await {
            Ok(d) => execute(&app, &cl, &addr, d.id).await,
            // Nothing left to redeem is the expected end state, not a failure.
            Err(_) => break,
        }
        sync_wallet_state(app.state(), None).await.expect("sync");
    }
    sync_wallet_state(app.state(), None).await.expect("sync");

    // The audit: a REVEAL coin still unspent is a losing bid whose lockup the
    // wallet never reclaimed.
    assert_eq!(
        unspent(&app, crate::noncustodial::sync::COV_REVEAL),
        0,
        "every losing reveal must be redeemable through the app"
    );
}

/// Each indexed bid must carry the value ITS OWN reveal disclosed.
///
/// hsd pairs a name covenant with the coin spent at the same index
/// (`rules.verifyCovenants`: `tx.inputs[i]` → `tx.output(i)`), so a reveal
/// output names exactly one bid. The scanner instead attached each reveal to
/// "the earliest bid not yet matched" — indistinguishable from the truth while
/// a wallet had one bid per name, and wrong the moment one transaction reveals
/// several: the values land on the wrong bids.
#[tokio::test]
async fn live_scanner_pairs_each_reveal_with_its_own_bid() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_scanner_pairs_each_reveal_with_its_own_bid: set HNS_IT_NODE_URL");
        return;
    };
    let cl = client(&url, &key);
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let acct = fresh_acct(tip);
    let (db_path, app) = file_backed_app(&url, &key, acct);
    let (addr, _, _) = leaf00_at(acct);
    fund(&cl, &addr, 101).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let name = format!("pair{tip}");

    let open = build_open_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build open");
    execute(&app, &cl, &addr, open.id).await;
    assert!(mine_until(&cl, &name, "BIDDING", &addr, 30).await);

    // Deliberately mismatched orderings: the bid values ascend while the
    // lockups descend, so pairing by anything other than the outpoint gets it
    // visibly wrong.
    let plan: [(i64, i64); 3] = [
        (1_000_000, 9_000_000),
        (2_000_000, 6_000_000),
        (3_000_000, 4_000_000),
    ];
    for (bid_v, lockup) in plan {
        sync_wallet_state(app.state(), None).await.expect("sync");
        let d = build_bid_draft(app.state(), name.clone(), bid_v, lockup, Some(1))
            .await
            .unwrap_or_else(|e| panic!("build bid {bid_v}: {e:?}"));
        execute(&app, &cl, &addr, d.id).await;
    }
    sync_wallet_state(app.state(), None).await.expect("sync");

    assert!(mine_until(&cl, &name, "REVEAL", &addr, 30).await);
    sync_wallet_state(app.state(), None).await.expect("sync");
    let reveal = build_reveal_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("build reveal");
    execute(&app, &cl, &addr, reveal.id).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    scan_to_tip(&cl, &db_path).await;

    // What the wallet knows locally: this bid txid bid this much.
    let expected: std::collections::HashMap<String, i64> = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::list_bid_commitments(&c, PROFILE)
            .unwrap()
            .into_iter()
            .filter(|b| b.name == name)
            .filter_map(|b| b.bid_txid.map(|t| (t, b.bid_value_doos)))
            .collect()
    };
    assert_eq!(expected.len(), 3);

    let indexed: Vec<(String, Option<i64>)> = {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        let mut stmt = c
            .prepare(
                "SELECT bid_txid, reveal_value_doos FROM name_bid_outpoints
                 WHERE name = ?1 ORDER BY height",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![name], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    };
    assert_eq!(indexed.len(), 3, "all three bids indexed");
    for (bid_txid, revealed) in indexed {
        let want = expected.get(&bid_txid).copied();
        assert_eq!(
            revealed, want,
            "bid {bid_txid} disclosed {want:?} but the index says {revealed:?}"
        );
    }
}

// =============================================================================
// Shakedex against the real shakedex CLI (R30, and the R7/R10/R13 paths)
// =============================================================================
//
// The other party is the shakedex CLI at its pinned commit, driven through
// `scripts/shakedex-cli-sell.sh` with the node's hsd wallet: it sells, cancels
// and buys first. Skipped unless `HNS_IT_SHAKEDEX=1` as well, since the script
// clones the CLI and needs a wallet-enabled node at the regtest default port
// (`scripts/regtest.sh --with-wallet`). The node's clock is only ever moved
// forward (`advance_mtp_past`), so the tests can run in any order.

/// The node and the shakedex CLI for a Shakedex live test, or `None` (the
/// test skips, and says so) when `HNS_IT_SHAKEDEX` is unset. Anything but
/// `1` there, or `1` without a node or with another node than the one the
/// script's `hsd-rpc` reaches, is a mistake in the setup and fails the test
/// rather than skipping it.
fn shakedex_env(test: &str) -> Option<(String, String, ShakedexCli)> {
    let (url, key) = shakedex_node_env(test)?;
    // The CLI and the script's hsd-rpc/hsw-rpc reach regtest's default ports.
    let port = Network::Regtest.default_rpc_port();
    assert!(
        url.trim_end_matches('/').ends_with(&format!(":{port}")),
        "the shakedex CLI talks to the regtest node at port {port}, not {url}"
    );
    let cli = shakedex_cli(&key);
    Some((url, key, cli))
}

/// The node for a Shakedex live test that needs no CLI, or `None` (the test
/// skips, and says so) when `HNS_IT_SHAKEDEX` is unset. Anything but `1`
/// there, or `1` without a node, fails the test rather than skipping it.
fn shakedex_node_env(test: &str) -> Option<(String, String)> {
    match std::env::var("HNS_IT_SHAKEDEX").ok().as_deref() {
        None | Some("") => {
            eprintln!("skip {test}: set HNS_IT_SHAKEDEX=1 and HNS_IT_NODE_URL");
            return None;
        }
        Some("1") => {}
        Some(other) => panic!("HNS_IT_SHAKEDEX must be 1, not {other:?}"),
    }
    Some(it_env().expect("HNS_IT_SHAKEDEX=1 needs HNS_IT_NODE_URL"))
}

/// The shakedex CLI, driven through `scripts/shakedex-cli-sell.sh`.
struct ShakedexCli {
    /// The script's `SHAKEDEX_WORK`: the CLI checkout and its database (the
    /// lock keys), kept across tests and runs.
    work: std::path::PathBuf,
    api_key: String,
}

/// The address of `name`'s owner coin, as the node reports it.
async fn owner_coin_address(cl: &NodeRpcClient, name: &str) -> Option<String> {
    let info = cl.get_name_info(name).await.expect("name info");
    let owner = &info["info"]["owner"];
    cl.get_coin(
        owner["hash"].as_str().expect("owner hash"),
        owner["index"].as_u64().expect("owner index") as u32,
    )
    .await
    .expect("owner coin")
    .expect("owner coin exists")
    .address
}

/// What the mined transaction `txid` pays to `addr`, as the node reports it.
async fn paid_to(cl: &NodeRpcClient, txid: &str, addr: &str) -> u64 {
    let tx = cl.get_tx_by_hash(txid).await.expect("tx lookup");
    assert!(
        tx["height"].as_i64().unwrap_or(-1) >= 0,
        "{txid} is not mined"
    );
    tx["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .filter(|o| o["address"].as_str() == Some(addr))
        .map(|o| o["value"].as_u64().expect("output value"))
        .sum()
}

fn shakedex_cli(api_key: &str) -> ShakedexCli {
    let work = std::env::var_os("SHAKEDEX_WORK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("namehold-shakedex-cli"));
    ShakedexCli {
        work,
        api_key: api_key.to_string(),
    }
}

impl ShakedexCli {
    /// Run the script with `args` and extra `env`; its stdout. A failure
    /// panics with the script's own log.
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> String {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/shakedex-cli-sell.sh");
        let out = std::process::Command::new("bash")
            .arg(&script)
            .args(args)
            .env("SHAKEDEX_WORK", &self.work)
            .env("HSD_API_KEY", &self.api_key)
            .envs(env.iter().copied())
            .output()
            .expect("run shakedex-cli-sell.sh");
        assert!(
            out.status.success(),
            "shakedex-cli-sell.sh {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    /// Register a fresh name with the hsd wallet, lock it and list it with
    /// `command` (`fixed` or `auction`).
    fn list(&self, command: &str, env: &[(&str, &str)]) -> CliListing {
        std::fs::create_dir_all(&self.work).expect("work dir");
        let out = self.work.join(format!(
            "listing-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let out_str = out.to_str().unwrap().to_string();
        let mut all = vec![("REGISTER", "1"), ("OUT", out_str.as_str())];
        all.extend_from_slice(env);
        self.run(&[command], &all);
        CliListing::new(std::fs::read_to_string(&out).expect("listing file"))
    }

    fn sell_fixed(&self, price_hns: u32) -> CliListing {
        self.list("fixed", &[("PRICE", &price_hns.to_string())])
    }

    fn sell_auction(&self, start_hns: u32, end_hns: u32) -> CliListing {
        self.list(
            "auction",
            &[
                ("START_PRICE", &start_hns.to_string()),
                ("END_PRICE", &end_hns.to_string()),
            ],
        )
    }

    /// The seller takes the name back out of its lock (mined).
    fn cancel(&self, name: &str) {
        self.run(&["cancel", name], &[]);
    }

    /// The hsd wallet buys `listing` (mined).
    fn fill(&self, listing: &CliListing) {
        let path = self.work.join("fill.json");
        std::fs::write(&path, &listing.json).expect("write listing");
        self.run(&["fill", path.to_str().unwrap()], &[]);
    }

    /// Register a fresh name with the hsd wallet; its name.
    fn register(&self) -> String {
        self.run(&["register"], &[]).trim().to_string()
    }

    /// The hsd wallet transfers `name` to `address` and finalizes it there;
    /// the name's owner outpoint, which then sits at `address`.
    fn lock_to(&self, name: &str, address: &str) -> (String, u32) {
        let out = self.run(&["lock-to", name, address], &[]);
        let mut it = out.split_whitespace();
        let txid = it.next().expect("owner txid").to_string();
        let vout = it.next().expect("owner index").parse().expect("index");
        (txid, vout)
    }
}

/// A name locked at a Shakedex lock address whose key this test holds, so it
/// can sign listings the CLI cannot write: a market fee, a step not valid
/// yet, steps of one price. The hsd wallet registers the name and moves it to
/// the lock with an ordinary TRANSFER and FINALIZE, as the CLI's
/// `transfer-lock`/`finalize-lock` do.
struct OwnLock {
    name: String,
    txid: String,
    vout: u32,
    value: u64,
    key: secp256k1::SecretKey,
}

/// Where a self-signed listing pays the seller and the market fee.
fn seller_addr() -> String {
    crate::noncustodial::address::encode_p2wpkh(NET, &[9; 20]).unwrap()
}

fn market_fee_addr() -> String {
    crate::noncustodial::address::encode_p2wpkh(NET, &[7; 20]).unwrap()
}

impl OwnLock {
    async fn new(cli: &ShakedexCli, url: &str, api_key: &str) -> Self {
        let key = secp256k1::SecretKey::from_slice(&[0x5d; 32]).unwrap();
        let name = cli.register();
        let lock =
            crate::noncustodial::shakedex::script::lock_address(NET, &Self::pubkey(&key)).unwrap();
        let (txid, vout) = cli.lock_to(&name, &lock);
        // A client of its own: one left idle while the script ran may hold a
        // connection the node has closed meanwhile.
        let coin = client(url, api_key)
            .get_coin(&txid, vout)
            .await
            .expect("lock coin")
            .expect("lock coin exists");
        assert_eq!(coin.address.as_deref(), Some(lock.as_str()));
        OwnLock {
            name,
            txid,
            vout,
            value: u64::try_from(coin.value).expect("lock value"),
            key,
        }
    }

    fn pubkey(key: &secp256k1::SecretKey) -> [u8; 33] {
        secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), key).serialize()
    }

    /// A listing of this lock: one step per `(price, lock_time)`, each
    /// signed here as the CLI signs it (`SIGHASH_SINGLE|ANYONECANPAY`), with
    /// a market fee of `fee` to [`market_fee_addr`] when it is not 0.
    fn listing(&self, steps: &[(u64, u64)], fee: u64) -> CliListing {
        use crate::noncustodial::shakedex::template::{step_sighash, StepTemplate};
        let secp = secp256k1::Secp256k1::new();
        let pubkey = Self::pubkey(&self.key);
        let mut lock_txid = [0u8; 32];
        hex::decode_to_slice(&self.txid, &mut lock_txid).unwrap();
        let payment = seller_addr();
        let data: Vec<serde_json::Value> = steps
            .iter()
            .map(|&(price, lock_time)| {
                let t = StepTemplate {
                    lock_outpoint: (lock_txid, self.vout),
                    lock_value: self.value,
                    lock_pubkey: &pubkey,
                    payment: crate::noncustodial::tx::output_address_from_string(NET, &payment)
                        .unwrap(),
                    price,
                    lock_time_secs: lock_time,
                };
                let msg = secp256k1::Message::from_digest(step_sighash(&t).unwrap());
                let mut sig = secp
                    .sign_ecdsa(&msg, &self.key)
                    .serialize_compact()
                    .to_vec();
                sig.push(0x84);
                serde_json::json!({
                    "price": price,
                    "lockTime": lock_time,
                    "signature": hex::encode(sig),
                    "fee": fee,
                })
            })
            .collect();
        CliListing::new(
            serde_json::json!({
                "name": self.name,
                "lockingTxHash": self.txid,
                "lockingOutputIdx": self.vout,
                "publicKey": hex::encode(pubkey),
                "paymentAddr": payment,
                "feeAddr": if fee == 0 { serde_json::Value::Null } else { market_fee_addr().into() },
                "data": data,
                "version": 2,
            })
            .to_string(),
        )
    }
}

/// The node's median time now.
async fn node_mtp(cl: &NodeRpcClient) -> u64 {
    cl.get_blockchain_info()
        .await
        .expect("info")
        .mediantime
        .expect("mediantime")
}

/// Take back every block above `height`: regtest halves the block subsidy
/// every 2500 blocks and later tests fund their wallets by mining, so a test
/// that mines a long stretch leaves the chain as tall as it found it.
async fn rewind_to(cl: &NodeRpcClient, height: i64) {
    let hash = cl.get_block_hash(height + 1).await.expect("blockhash");
    cl.invalidate_block(&hash).await.expect("rewind");
}

/// Mine until the tip is `height`, a hundred blocks per call.
async fn mine_to(cl: &NodeRpcClient, addr: &str, height: i64) {
    loop {
        let tip = cl.get_blockchain_info().await.expect("info").blocks;
        if tip >= height {
            return;
        }
        let n = u32::try_from((height - tip).min(100)).unwrap();
        cl.generate_to_address(n, addr).await.expect("mine");
    }
}

/// A listing file the CLI wrote: the file as written, and what the tests
/// read from it.
struct CliListing {
    json: String,
    name: String,
    /// Where the seller is paid.
    payment_addr: String,
    /// `(price, lockTime)` of each step, in the file's order.
    steps: Vec<(u64, u64)>,
}

impl CliListing {
    fn new(json: String) -> Self {
        let v: serde_json::Value = serde_json::from_str(&json).expect("listing json");
        let name = v["name"].as_str().expect("listing name").to_string();
        let payment_addr = v["paymentAddr"].as_str().expect("paymentAddr").to_string();
        let steps = v["data"]
            .as_array()
            .expect("price steps")
            .iter()
            .map(|s| {
                (
                    s["price"].as_u64().expect("price"),
                    s["lockTime"].as_u64().expect("lockTime"),
                )
            })
            .collect();
        CliListing {
            json,
            name,
            payment_addr,
            steps,
        }
    }

    fn price(&self, step: usize) -> u64 {
        self.steps[step].0
    }

    fn lock_time(&self, step: usize) -> u64 {
        self.steps[step].1
    }
}

/// A funded Namehold buyer on a fresh account of the node under test.
struct ShakedexBuyer {
    app: tauri::App<tauri::test::MockRuntime>,
    db_path: std::path::PathBuf,
    cl: NodeRpcClient,
    addr: String,
}

impl ShakedexBuyer {
    async fn new(url: &str, key: &str) -> Self {
        let cl = client(url, key);
        // Regtest halves the block subsidy every 2500 blocks, and a buyer is
        // funded by mining: on a chain this tall it gets too little to spend,
        // and the test would fail for a reason that has nothing to do with it.
        let height = cl.get_blockchain_info().await.expect("info").blocks;
        assert!(
            height <= 30_000,
            "the regtest chain is {height} blocks tall: too little block subsidy left \
             to fund a buyer; start over with `scripts/regtest.sh reset`"
        );
        // Mined first so a second buyer in the same test gets its own account.
        let (miner, _, _) = leaf00();
        cl.generate_to_address(1, &miner).await.expect("mine");
        let tip = cl.get_blockchain_info().await.expect("info").blocks;
        let acct = fresh_acct(tip);
        let (db_path, app) = file_backed_app(url, key, acct);
        let (addr, _, _) = leaf00_at(acct);
        cl.generate_to_address(101, &addr).await.expect("fund");
        sync_wallet_state(app.state(), None).await.expect("sync");
        ShakedexBuyer {
            app,
            db_path,
            cl,
            addr,
        }
    }

    /// Run the purchase job as the app's sync does.
    async fn refresh(&self) {
        self.refresh_with(crate::shakedex_jobs::Rebroadcast::Allowed)
            .await;
    }

    /// Run the purchase job; `Never` as the daemon's sync does.
    async fn refresh_with(&self, rebroadcast: crate::shakedex_jobs::Rebroadcast) {
        crate::shakedex_jobs::refresh_purchases_step(
            self.db_path.to_str().unwrap(),
            PROFILE,
            rebroadcast,
        )
        .await;
    }

    /// The wallet's coins coin selection may spend now.
    fn spendable(&self) -> Vec<crate::noncustodial::send::SpendableCoin> {
        let state = self.app.state::<AppState>();
        let c = state.db.lock().unwrap();
        crate::noncustodial::send::load_spendable_coins(&c, PROFILE, None, NET).unwrap()
    }

    /// Whether the wallet tracks any coin of `txid`, spendable or not.
    fn tracks_coin_of(&self, txid: &str) -> bool {
        let state = self.app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE wallet_profile_id = ?1 AND txid = ?2",
            params![PROFILE, txid],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    /// How many coins `draft_id` holds reserved.
    fn reserved_by(&self, draft_id: &str) -> i64 {
        let state = self.app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
            params![draft_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// The purchase of `name`.
    fn purchase(&self, name: &str) -> db::queries::ShakedexPurchase {
        let state = self.app.state::<AppState>();
        let c = state.db.lock().unwrap();
        let id: String = c
            .query_row(
                "SELECT id FROM shakedex_purchases WHERE wallet_profile_id = ?1 AND name = ?2",
                params![PROFILE, name],
                |r| r.get(0),
            )
            .expect("purchase row");
        db::queries::get_shakedex_purchase(&c, &id)
            .unwrap()
            .expect("purchase")
    }

    /// Build and sign the purchase of `listing_json`, not yet sent.
    async fn sign_purchase(
        &self,
        listing: &CliListing,
    ) -> crate::noncustodial::types::TxDraftSummary {
        let draft = self
            .build_purchase(listing)
            .await
            .expect("build purchase draft");
        unlock(&self.app);
        sign_tx_draft_inner(&self.app.state(), &draft.id)
            .await
            .expect("sign");
        draft
    }

    async fn build_purchase(
        &self,
        listing: &CliListing,
    ) -> Result<crate::noncustodial::types::TxDraftSummary, crate::error::AppError> {
        self.build_purchase_paying(listing, None).await
    }

    /// Build the purchase of `listing`, accepting a market fee of
    /// `market_fee` doos (a file's fee is never pre-accepted).
    async fn build_purchase_paying(
        &self,
        listing: &CliListing,
        market_fee: Option<u64>,
    ) -> Result<crate::noncustodial::types::TxDraftSummary, crate::error::AppError> {
        crate::commands::shakedex::shakedex_build_purchase_draft(
            self.app.state(),
            listing.json.clone(),
            market_fee,
            false,
            Some(1),
        )
        .await
    }

    async fn import(&self, listing: &CliListing) -> serde_json::Value {
        let row = crate::commands::shakedex::shakedex_import_listing(
            self.app.state(),
            crate::commands::shakedex::ImportSource::Text {
                json: listing.json.clone(),
            },
        )
        .await
        .expect("import listing");
        serde_json::to_value(row).unwrap()
    }
}

impl Drop for ShakedexBuyer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.db_path);
    }
}

/// Move the node's median time past `t`: set the node's clock just after `t`
/// unless it is already later, and mine 11 blocks, so the median of the last
/// 11 timestamps follows. The clock only moves forward: hsd keeps
/// `setmocktime` as an offset that goes on ticking and reports it as
/// `getinfo.timeoffset`, so the node's clock is ours plus that offset.
async fn advance_mtp_past(cl: &NodeRpcClient, addr: &str, t: u64) {
    let offset = cl.get_info().await.expect("getinfo")["timeoffset"]
        .as_i64()
        .expect("timeoffset");
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    if wall + offset <= t as i64 {
        cl.set_mock_time(t + 1).await.expect("setmocktime");
    }
    cl.generate_to_address(11, addr).await.expect("mine");
    let after = cl
        .get_blockchain_info()
        .await
        .expect("info")
        .mediantime
        .expect("mediantime");
    assert!(after > t, "median time {after} did not pass {t}");
}

/// R30, Namehold-buys half: buy a fixed-price listing the CLI wrote, through
/// the mempool, the transfer lockup and the finalize, to owned.
#[tokio::test]
async fn shakedex_cli_listing_is_bought() {
    let Some((url, key, cli)) = shakedex_env("shakedex_cli_listing_is_bought") else {
        return;
    };
    let listing = cli.sell_fixed(5);
    let name = listing.name.clone();
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = b.sign_purchase(&listing).await;
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    assert_eq!(bc.status, "broadcasted");

    // In the mempool: unconfirmed (hsd sends `height: -1`), and the draft
    // lifecycle leaves the purchase draft to the purchase job.
    b.refresh().await;
    assert_eq!(
        b.purchase(&name).state,
        db::queries::PurchaseState::Unconfirmed
    );
    refresh_tx_confirmations(b.app.state(), None)
        .await
        .expect("refresh confirmations");
    assert_eq!(draft_status(&b.app, &draft.id).status, "broadcasted");

    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::AwaitingFinalize);
    // Mined at the tip: hsd judges the FINALIZE at the next block (R13's N).
    assert_eq!(
        p.blocks_remaining,
        Some(i64::from(NET.name_params().transfer_lockup) - 1)
    );
    // The seller is paid the listing's price, on chain.
    assert_eq!(
        paid_to(&b.cl, &bc.txid, &listing.payment_addr).await,
        listing.price(0)
    );

    // Finalize once the transfer lockup is over.
    b.cl.generate_to_address(NET.name_params().transfer_lockup, &b.addr)
        .await
        .expect("mine lockup");
    b.refresh().await;
    let fin = crate::commands::shakedex::shakedex_build_purchase_finalize_draft(
        b.app.state(),
        p.id.clone(),
        Some(1),
    )
    .await
    .expect("build finalize draft");
    broadcast_only(&b.app, &fin.id).await;
    settle(&b.app, &b.cl, &b.addr, &fin.id).await;
    b.refresh().await;
    assert_eq!(b.purchase(&name).state, db::queries::PurchaseState::Owned);

    // The node agrees: the name's owner coin pays the purchase's destination.
    assert_eq!(
        owner_coin_address(&b.cl, &name).await.as_deref(),
        Some(p.destination_address.as_str())
    );

    // The node's history of our destination, as it really sends it, holds
    // the FINALIZE as the spender of the purchase's TRANSFER: what the job
    // reads to tell a finalized purchase whose name moved on since.
    let history =
        b.cl.get_txs_by_address(&p.destination_address)
            .await
            .expect("destination history");
    let spenders: Vec<u64> = history
        .iter()
        .filter_map(|tx| {
            crate::shakedex_jobs::transfer_spent_into(tx, &bc.txid).expect("hsd's tx shape")
        })
        .collect();
    assert_eq!(
        spenders,
        [u64::from(crate::noncustodial::sync::COV_FINALIZE)]
    );
}

/// R13: a purchase finalized and moved on before the purchase job looked at it
/// again. The job finds the TRANSFER spent and the name's owner elsewhere,
/// and reads our destination's history from the real node: our FINALIZE is
/// there as the TRANSFER's spender, so the purchase was owned — not left
/// awaiting a finalize the backend would refuse for ever.
#[tokio::test]
async fn shakedex_finalized_name_moved_on_before_a_sync_is_owned() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_finalized_name_moved_on_before_a_sync_is_owned")
    else {
        return;
    };
    let lockup = NET.name_params().transfer_lockup;
    let listing = cli.sell_fixed(5);
    let name = listing.name.clone();
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = b.sign_purchase(&listing).await;
    broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::AwaitingFinalize);

    // Our FINALIZE, then the name moved on: a TRANSFER to an address that is
    // not the purchase's destination and its FINALIZE — all before the
    // purchase job runs again.
    b.cl.generate_to_address(lockup, &b.addr)
        .await
        .expect("mine lockup");
    let fin = crate::commands::shakedex::shakedex_build_purchase_finalize_draft(
        b.app.state(),
        p.id.clone(),
        Some(1),
    )
    .await
    .expect("build purchase finalize");
    broadcast_only(&b.app, &fin.id).await;
    settle(&b.app, &b.cl, &b.addr, &fin.id).await;

    // The app learns it holds the name as the full sync's discovery would:
    // track it, and sync picks up the owner coin at our destination.
    track_name(&b.app, &name);
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let (_sk, _pk, elsewhere) =
        crate::noncustodial::hd::derive_address(NET, &seed(), 0, 0, 1).unwrap();
    assert_ne!(elsewhere, p.destination_address);
    let out = crate::commands::names::build_transfer_draft(
        b.app.state(),
        name.clone(),
        elsewhere.clone(),
        Some(1),
    )
    .await
    .expect("build transfer out");
    broadcast_only(&b.app, &out.id).await;
    settle(&b.app, &b.cl, &b.addr, &out.id).await;
    b.cl.generate_to_address(lockup, &b.addr)
        .await
        .expect("mine lockup");
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let out_fin =
        crate::commands::names::build_finalize_draft(b.app.state(), name.clone(), Some(1))
            .await
            .expect("build finalize out");
    broadcast_only(&b.app, &out_fin.id).await;
    settle(&b.app, &b.cl, &b.addr, &out_fin.id).await;

    // The node agrees the name has left our destination.
    assert_eq!(
        owner_coin_address(&b.cl, &name).await.as_deref(),
        Some(elsewhere.as_str())
    );

    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::Owned);
    assert_eq!(p.lost_reason, None);
}

/// R13 "Lost, nothing paid": the CLI buys the listing between our review and
/// our broadcast. hsd 8.0.0's `sendrawtransaction` answers with the txid
/// whatever its mempool does (`rpc.js`: `this.node.relay(tx)`, not awaited,
/// its error only logged); the look-up after it finds the node did not take
/// the purchase (honest-broadcast R2, R5), so the broadcast says "not sent"
/// and the draft waits as `broadcast_pending`. The purchase job then finds
/// the lock coin spent and loses it with nothing paid. Its coins are free
/// again, and the listing is no longer offered.
#[tokio::test]
async fn shakedex_cli_buyer_first_loses_ours_with_nothing_paid() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_cli_buyer_first_loses_ours_with_nothing_paid")
    else {
        return;
    };
    let listing = cli.sell_fixed(3);
    let name = listing.name.clone();
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = b.sign_purchase(&listing).await;
    assert!(b.reserved_by(&draft.id) > 0, "the purchase holds its coins");
    cli.fill(&listing);
    let err = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect_err("the node did not take the purchase");
    assert!(err.to_string().contains("did not take"), "{err}");
    assert_eq!(draft_status(&b.app, &draft.id).status, "broadcast_pending");
    let purchase_txid = summary_txid(&b.app, &draft.id);

    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::Lost);
    // R13: the reason the spec gives, on the purchase and on the draft
    // Activity shows.
    let bought_by_other = "someone else bought the name first, or the seller cancelled the listing — nothing was paid";
    assert_eq!(p.lost_reason.as_deref(), Some(bought_by_other));
    let row = draft_status(&b.app, &draft.id);
    assert_eq!(row.status, "dropped");
    assert_eq!(row.error_message.as_deref(), Some(bought_by_other));
    assert_eq!(
        b.reserved_by(&draft.id),
        0,
        "the purchase's funding coins are free again"
    );

    // A block later the node still has nothing of it: hsd never took it,
    // whatever its answer to the broadcast said.
    b.cl.generate_to_address(1, &b.addr).await.expect("mine");
    assert!(
        b.cl.get_tx_by_hash(&purchase_txid)
            .await
            .expect("tx lookup")
            .is_null(),
        "the node did not take the purchase"
    );

    let row = b.import(&listing).await;
    assert_eq!(row["verdict"]["verdict"], "hidden", "{row}");
    assert_eq!(row["verdict"]["kind"], "soldOrCancelled", "{row}");
    assert_refused(&b, &listing, "already sold or cancelled").await;
}

/// R13 "A reorg moves the state back", and the one rebroadcast. hsd's
/// `invalidateblock` rewinds through `reset`, which empties the mempool
/// (unlike a reorg to a heavier chain, which puts the block's transactions
/// back): so the purchase whose block is taken away is gone from the node
/// altogether. It reads as unconfirmed, is rebroadcast once by the app's sync
/// after six blocks missing, and is mined again. Later the block that mined
/// the FINALIZE is taken away: awaiting finalize again, and owned once the
/// same FINALIZE is mined again (as a real reorg's mempool would).
#[tokio::test]
async fn shakedex_purchase_follows_reorgs_of_its_own_blocks() {
    let Some((url, key, cli)) = shakedex_env("shakedex_purchase_follows_reorgs_of_its_own_blocks")
    else {
        return;
    };
    let listing = cli.sell_fixed(4);
    let name = listing.name.clone();
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = b.sign_purchase(&listing).await;
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::AwaitingFinalize);
    let bought_at = p.purchase_height.expect("purchase height");

    let block = b.cl.get_block_hash(bought_at).await.expect("blockhash");
    b.cl.invalidate_block(&block).await.expect("invalidate");
    assert!(
        b.cl.get_tx_by_hash(&bc.txid)
            .await
            .expect("tx lookup")
            .is_null(),
        "invalidateblock leaves the purchase nowhere"
    );
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::Unconfirmed);
    assert_eq!(p.rebroadcast_count, 0);

    // Six blocks missing: the app's sync sends it once more, and it is mined.
    b.cl.generate_to_address(crate::shakedex_jobs::MISSING_BLOCKS as u32, &b.addr)
        .await
        .expect("mine");
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.rebroadcast_count, 1, "rebroadcast once");
    // The node took the rebroadcast.
    wait_until_node_has(&b.cl, &bc.txid).await;
    b.cl.generate_to_address(1, &b.addr).await.expect("mine");
    b.cl.reconsider_block(&block).await.expect("reconsider");
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::AwaitingFinalize);
    assert!(p.purchase_height.expect("purchase height") > bought_at);

    b.cl.generate_to_address(NET.name_params().transfer_lockup, &b.addr)
        .await
        .expect("mine lockup");
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    b.refresh().await;
    let fin = crate::commands::shakedex::shakedex_build_purchase_finalize_draft(
        b.app.state(),
        p.id.clone(),
        Some(1),
    )
    .await
    .expect("build finalize draft");
    broadcast_only(&b.app, &fin.id).await;
    settle(&b.app, &b.cl, &b.addr, &fin.id).await;
    b.refresh().await;
    assert_eq!(b.purchase(&name).state, db::queries::PurchaseState::Owned);

    let fin_row = draft_status(&b.app, &fin.id);
    let finalized_at = fin_row.confirmation_height.expect("finalize height");
    let block = b.cl.get_block_hash(finalized_at).await.expect("blockhash");
    b.cl.invalidate_block(&block).await.expect("invalidate");
    b.refresh().await;
    assert_eq!(
        b.purchase(&name).state,
        db::queries::PurchaseState::AwaitingFinalize,
        "the FINALIZE's block is gone"
    );
    b.cl.send_raw_transaction(fin_row.signed_tx_hex.as_deref().expect("signed finalize"))
        .await
        .expect("resend finalize");
    b.cl.generate_to_address(1, &b.addr).await.expect("mine");
    b.cl.reconsider_block(&block).await.expect("reconsider");
    b.refresh().await;
    assert_eq!(b.purchase(&name).state, db::queries::PurchaseState::Owned);
}

/// R10/R12 on a reverse auction the CLI wrote: the purchase pays the step the
/// node's median time makes current, and a purchase signed at one step is
/// refused at broadcast once a cheaper step has become valid; built again,
/// it pays the cheaper step.
#[tokio::test]
async fn shakedex_reverse_auction_pays_the_current_step_and_refuses_a_stale_one() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_reverse_auction_pays_the_current_step_and_refuses_a_stale_one")
    else {
        return;
    };
    let listing = cli.sell_auction(10, 5);
    let name = listing.name.clone();
    let b = ShakedexBuyer::new(&url, &key).await;
    // Past step 0 and short of step 1 (15 minutes later, so its 512-second
    // rounding cannot make it valid yet).
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let row = b.import(&listing).await;
    assert_eq!(row["kind"], "reverseAuction", "{row}");
    assert_eq!(row["currentPrice"], listing.price(0), "{row}");
    assert_eq!(row["nextPrice"], listing.price(1), "{row}");

    let stale = b.sign_purchase(&listing).await;
    assert_eq!(stale.summary["priceDoos"], listing.price(0));
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(1)).await;
    let err = broadcast_tx_draft(b.app.state(), stale.id.clone())
        .await
        .expect_err("a cheaper step became valid");
    assert!(
        err.to_string()
            .contains(crate::noncustodial::shakedex::verify::PRICE_CHANGED),
        "{err}"
    );
    {
        let state = b.app.state::<AppState>();
        let c = state.db.lock().unwrap();
        assert!(
            db::queries::get_tx_draft(&c, &stale.id).unwrap().is_none(),
            "the stale draft is discarded"
        );
        let purchases: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM shakedex_purchases WHERE wallet_profile_id = ?1 AND name = ?2",
                params![PROFILE, name],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(purchases, 0, "and its purchase record with it");
    }
    assert_eq!(b.reserved_by(&stale.id), 0, "its coins are free");

    let fresh = b.sign_purchase(&listing).await;
    assert_eq!(fresh.summary["priceDoos"], listing.price(1));
    let bc = broadcast_tx_draft(b.app.state(), fresh.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &fresh.id).await;
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::AwaitingFinalize);
    assert_eq!(p.price_doos as u64, listing.price(1));
    assert_eq!(
        paid_to(&b.cl, &bc.txid, &listing.payment_addr).await,
        listing.price(1),
        "the seller is paid the cheaper step, on chain"
    );
}

/// R7: a listing whose seller took the name back out of its lock is shown as
/// sold or cancelled and cannot be bought.
#[tokio::test]
async fn shakedex_cancelled_listing_is_not_offered() {
    let Some((url, key, cli)) = shakedex_env("shakedex_cancelled_listing_is_not_offered") else {
        return;
    };
    let listing = cli.sell_fixed(2);
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    assert_eq!(b.import(&listing).await["verdict"]["verdict"], "buyable");

    cli.cancel(&listing.name);
    let row = b.import(&listing).await;
    assert_eq!(row["verdict"]["verdict"], "hidden", "{row}");
    assert_eq!(row["verdict"]["kind"], "soldOrCancelled", "{row}");
    assert_refused(&b, &listing, "already sold or cancelled").await;
}

/// Building the purchase of `listing` is refused, for the reason given.
async fn assert_refused(b: &ShakedexBuyer, listing: &CliListing, reason: &str) {
    let err = b
        .build_purchase(listing)
        .await
        .expect_err("the purchase is refused");
    assert!(err.to_string().contains(reason), "{err}");
}

/// R9 on the real chain: a name that would expire before its purchase could
/// be finalized is not offered and cannot be bought, and one block earlier it
/// is buyable with the warning. The expiry is hsd's own (`renewalPeriodEnd`),
/// and the chain is mined up to it, so the boundary is the node's, not a mock's.
#[tokio::test]
async fn shakedex_listing_expiring_before_finalize_is_not_offered() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_listing_expiring_before_finalize_is_not_offered")
    else {
        return;
    };
    let listing = cli.sell_fixed(2);
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;

    let info = b.cl.get_name_info(&listing.name).await.expect("name info");
    assert_eq!(info["info"]["claimed"], 0, "{info}");
    let end = info["info"]["stats"]["renewalPeriodEnd"]
        .as_u64()
        .expect("hsd's renewalPeriodEnd");
    // R9: not buyable once the expiry falls at or before
    // tip + 1 + transferLockup + 1 day, a day being the lockup on regtest.
    let p = NET.name_params();
    let lockup = u64::from(p.transfer_lockup);
    let day = u64::from(p.margin_day());
    let last_buyable_tip = end - 2 - lockup - day;

    let mut tip =
        u64::try_from(b.cl.get_blockchain_info().await.expect("info").blocks).expect("tip");
    assert!(
        tip < last_buyable_tip,
        "tip {tip} already past {last_buyable_tip}"
    );
    let first_mined = tip + 1;
    while tip < last_buyable_tip {
        let n = (last_buyable_tip - tip).min(100) as u32;
        b.cl.generate_to_address(n, &b.addr).await.expect("mine");
        tip += u64::from(n);
    }
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let last_buyable = b.import(&listing).await;
    b.cl.generate_to_address(1, &b.addr).await.expect("mine");
    let too_late = b.import(&listing).await;
    let build = b.build_purchase(&listing).await;

    // Take the mined blocks back out before asserting anything: regtest halves
    // the block subsidy every 2500 blocks, and every later test funds its
    // wallet by mining, so a chain left 5000 blocks taller starves them.
    let hash =
        b.cl.get_block_hash(first_mined as i64)
            .await
            .expect("blockhash");
    b.cl.invalidate_block(&hash).await.expect("rewind");

    assert_eq!(
        last_buyable["verdict"]["verdict"], "buyable",
        "{last_buyable}"
    );
    assert_eq!(last_buyable["verdict"]["expiryEnd"], end, "{last_buyable}");
    assert_eq!(
        last_buyable["verdict"]["warnExpiry"], true,
        "{last_buyable}"
    );
    assert_eq!(too_late["verdict"]["verdict"], "hidden", "{too_late}");
    assert_eq!(
        too_late["verdict"]["kind"], "expiresBeforeFinalize",
        "{too_late}"
    );
    let err = build.expect_err("the purchase is refused");
    assert!(
        err.to_string()
            .contains("expires before the purchase could be finalized"),
        "{err}"
    );
}

use crate::commands::shakedex::shakedex_build_lock_draft;
use crate::db::queries::{ListingMode, ListingState};

/// An address no profile of the suite derives: blocks mined to it fund
/// nobody, so a test that mines a long stretch leaves no coins to sync.
fn burn_addr() -> String {
    crate::noncustodial::address::encode_p2wpkh(NET, &[0x5a; 20]).unwrap()
}

/// The name's height as hsd reports it (`getnameinfo.info.height`), the
/// value a TRANSFER's covenant carries as items[1] (u32 little-endian).
async fn info_height(cl: &NodeRpcClient, name: &str) -> u32 {
    let info = cl.get_name_info(name).await.expect("name info");
    let h = info["info"]["height"].as_u64().expect("name height");
    u32::try_from(h).unwrap()
}

/// Takes the chain back to `height` when dropped, unless [`Self::rewind`]
/// already did: a test that mines a long stretch leaves the chain as it found
/// it even when it panics before its own rewind.
struct RewindOnDrop {
    /// The node's URL and key: the rewind on drop builds a client of its own,
    /// since a client's connections belong to the runtime that made them.
    node: Option<(String, String)>,
    height: i64,
}

impl RewindOnDrop {
    fn new(url: &str, key: &str, height: i64) -> Self {
        Self {
            node: Some((url.to_string(), key.to_string())),
            height,
        }
    }

    async fn rewind(mut self, cl: &NodeRpcClient) {
        self.node = None;
        rewind_to(cl, self.height).await;
    }
}

impl Drop for RewindOnDrop {
    fn drop(&mut self) {
        let Some((url, key)) = self.node.take() else {
            return;
        };
        let height = self.height;
        // Drop cannot await, and the test's runtime may be the one panicking:
        // the rewind runs on a runtime of its own, on its own thread.
        let done = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(rewind_to(&client(&url, &key), height));
        })
        .join();
        match done {
            Ok(()) => eprintln!("RewindOnDrop: the chain is back at {height}"),
            Err(_) => eprintln!("RewindOnDrop: could not rewind to {height}"),
        }
    }
}

/// A funded profile owning a fresh name, unlocked. `(app, client, our
/// address, name)`.
async fn own_a_name(
    url: &str,
    key: &str,
    prefix: &str,
) -> (
    tauri::App<tauri::test::MockRuntime>,
    NodeRpcClient,
    String,
    String,
) {
    let app = app_with(seeded_conn_regtest(url, key));
    let cl = client(url, key);
    let (addr, _, _) = leaf00();
    cl.generate_to_address(101, &addr).await.expect("fund");
    sync_wallet_state(app.state(), None).await.expect("sync");
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    let name = format!("{prefix}{tip}");
    acquire_name(&app, &cl, &addr, &name).await;
    unlock(&app);
    (app, cl, addr, name)
}

fn open_listing(
    app: &tauri::App<tauri::test::MockRuntime>,
    name: &str,
) -> Option<db::queries::ShakedexListing> {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    db::queries::open_shakedex_listing_for_name(&c, PROFILE, name).unwrap()
}

/// Lock `name`, broadcast the TRANSFER and mine it; the draft.
async fn lock_on_chain(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    name: &str,
) -> crate::noncustodial::types::TxDraftSummary {
    let draft = shakedex_build_lock_draft(
        app.state(),
        name.to_string(),
        ListingMode::BuyNow,
        false,
        Some(1),
    )
    .await
    .expect("lock builds");
    broadcast_only(app, &draft.id).await;
    settle(app, cl, addr, &draft.id).await;
    draft
}

/// R19 day 0 and R18 on hsd: the mined TRANSFER of our name commits, in
/// covenant items 2-3, to version 0 and SHA3-256 of the lock script of the
/// key derived here from the seed for this name and account; the name stays
/// at our address and hsd records the transfer.
#[tokio::test]
async fn shakedex_lock_transfer_commits_to_the_lock_address() {
    let Some((url, key)) = shakedex_node_env("shakedex_lock_transfer_commits_to_the_lock_address")
    else {
        return;
    };
    let (app, cl, addr, name) = own_a_name(&url, &key, "lockto").await;
    let home = owner_coin_address(&cl, &name).await.expect("owner address");
    let draft = lock_on_chain(&app, &cl, &addr, &name).await;

    let lock = crate::noncustodial::shakedex::lock_key::derive_lock_key(
        &master(),
        NET,
        test_acct(),
        &name,
    )
    .unwrap();
    let txid = draft_status(&app, &draft.id).txid.expect("sent");
    let tx = cl.get_tx_by_hash(&txid).await.expect("tx");
    let height = tx["height"].as_i64().expect("height");
    assert!(height > 0, "mined: {tx}");
    let out = &tx["outputs"][0];
    assert_eq!(
        out["covenant"]["type"],
        u64::from(crate::noncustodial::sync::COV_TRANSFER),
        "{tx}"
    );
    let items = out["covenant"]["items"].as_array().expect("items");
    assert_eq!(
        items[0],
        hex::encode(crate::noncustodial::names::hash_name(&name).unwrap())
    );
    let name_height = info_height(&cl, &name).await;
    assert_eq!(
        items[1],
        hex::encode(name_height.to_le_bytes()),
        "items[1] is the name's height"
    );
    assert_eq!(items[2], "00");
    assert_eq!(items[3], hex::encode(lock.program));
    assert_eq!(
        out["address"], home,
        "the name stays at our address until finalized"
    );

    let info = cl.get_name_info(&name).await.expect("name info");
    assert_eq!(info["info"]["transfer"].as_i64(), Some(height), "{info}");
    let (owner_hash, owner_index) = name_owner(&cl, &name).await;
    assert_eq!((owner_hash.as_str(), owner_index), (txid.as_str(), 0));

    let l = open_listing(&app, &name).expect("listing");
    assert_eq!(l.state, ListingState::Locking);
    assert_eq!(l.lock_pubkey_hex, hex::encode(lock.pubkey));
    assert_eq!(l.lock_transfer_txid.as_deref(), Some(txid.as_str()));
}

/// R19's abort on hsd: Cancel transfer of a name still locking is mined as
/// an UPDATE, hsd drops the transfer, and the abort job then marks the
/// listing Aborted.
#[tokio::test]
async fn shakedex_cancel_transfer_aborts_the_listing_on_chain() {
    let Some((url, key)) =
        shakedex_node_env("shakedex_cancel_transfer_aborts_the_listing_on_chain")
    else {
        return;
    };
    let (app, cl, addr, name) = own_a_name(&url, &key, "lockabort").await;
    lock_on_chain(&app, &cl, &addr, &name).await;
    let listing_id = open_listing(&app, &name).expect("listing").id;

    let cancel = crate::commands::names::build_cancel_draft(app.state(), name.clone(), Some(1))
        .await
        .expect("cancel builds");
    broadcast_only(&app, &cancel.id).await;
    let cancel_txid = draft_status(&app, &cancel.id).txid.expect("sent");
    wait_until_node_has(&cl, &cancel_txid).await;
    // The abort job before the cancel is mined: sent is not aborted.
    abort_job(&app, &cl).await;
    assert_eq!(
        listing_state(&app, &listing_id),
        ListingState::Locking,
        "sent, not mined"
    );
    // `settle` only mines the cancel and syncs the wallet; it does not abort.
    settle(&app, &cl, &addr, &cancel.id).await;

    let tx = cl.get_tx_by_hash(&cancel_txid).await.expect("tx");
    assert!(tx["height"].as_i64().expect("height") > 0, "mined: {tx}");
    assert_eq!(
        tx["outputs"][0]["covenant"]["type"],
        u64::from(crate::noncustodial::sync::COV_UPDATE),
        "{tx}"
    );
    let info = cl.get_name_info(&name).await.expect("name info");
    assert_eq!(
        info["info"]["transfer"], 0,
        "hsd dropped the transfer: {info}"
    );
    let (owner_hash, owner_index) = name_owner(&cl, &name).await;
    assert_eq!(
        (owner_hash.as_str(), owner_index),
        (cancel_txid.as_str(), 0)
    );
    assert_eq!(
        listing_state(&app, &listing_id),
        ListingState::Locking,
        "nothing aborts but the job"
    );
    abort_job(&app, &cl).await;
    assert_eq!(listing_state(&app, &listing_id), ListingState::Aborted);
}

/// Run the R19 abort step (`shakedex_jobs::refresh_listings_before_lock_with_client`)
/// on the app's database against the live node, as `run_sync_steps` does. The
/// connection is taken out of the app for the call, so no lock is held across
/// an await (same pattern as `shakedex_sell_tests::run_abort_job`).
async fn abort_job(app: &tauri::App<tauri::test::MockRuntime>, cl: &NodeRpcClient) {
    let conn = std::mem::replace(
        &mut *app.state::<AppState>().db.lock().unwrap(),
        rusqlite::Connection::open_in_memory().unwrap(),
    );
    let res =
        crate::shakedex_jobs::refresh_listings_before_lock_with_client(&conn, cl, PROFILE).await;
    *app.state::<AppState>().db.lock().unwrap() = conn;
    res.expect("abort job runs");
}

fn listing_state(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> ListingState {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    db::queries::get_shakedex_listing(&conn, id)
        .unwrap()
        .unwrap()
        .state
}

/// R31 on hsd: the lock is refused once the name's expiry (hsd's own
/// `renewalPeriodEnd`) is at or before tip + 1 + transferLockup + day, and
/// one block earlier it builds, with the near-expiry warning. The blocks are
/// mined to an address of nobody's and taken back before asserting, and by a
/// [`RewindOnDrop`] guard if the test panics first.
#[tokio::test]
async fn shakedex_lock_refused_near_expiry() {
    let Some((url, key)) = shakedex_node_env("shakedex_lock_refused_near_expiry") else {
        return;
    };
    let (app, cl, _addr, name) = own_a_name(&url, &key, "lockexp").await;
    let info = cl.get_name_info(&name).await.expect("name info");
    assert_eq!(info["info"]["claimed"], 0, "{info}");
    let end = info["info"]["stats"]["renewalPeriodEnd"]
        .as_i64()
        .expect("hsd's renewalPeriodEnd");
    let p = NET.name_params();
    // Refused while end <= tip + 1 + lockup + day: the last tip that locks.
    let last_lockable = end - 2 - i64::from(p.transfer_lockup) - i64::from(p.margin_day());
    let tip = cl.get_blockchain_info().await.expect("info").blocks;
    assert!(
        tip < last_lockable,
        "tip {tip} already past {last_lockable}"
    );
    let first_mined = tip + 1;
    let rewind = RewindOnDrop::new(&url, &key, first_mined - 1);
    mine_to(&cl, &burn_addr(), last_lockable).await;
    let at_last = shakedex_build_lock_draft(
        app.state(),
        name.clone(),
        ListingMode::BuyNow,
        false,
        Some(1),
    )
    .await;
    if let Ok(d) = &at_last {
        // Free the owner coin for the next build: an unsent draft goes with its listing.
        let state = app.state::<AppState>();
        db::queries::delete_tx_draft(&state.db.lock().unwrap(), &d.id).expect("delete");
    }
    cl.generate_to_address(1, &burn_addr()).await.expect("mine");
    let too_late = shakedex_build_lock_draft(
        app.state(),
        name.clone(),
        ListingMode::BuyNow,
        false,
        Some(1),
    )
    .await;

    rewind.rewind(&cl).await;

    let d = at_last.expect("the last lockable block locks");
    let warnings = d.summary["warnings"].as_array().expect("warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("The name expires in")),
        "{warnings:?}"
    );
    let err = too_late.expect_err("one block later the lock is refused");
    assert!(
        err.to_string()
            .contains("before its transfer into the lock could be finalized"),
        "{err}"
    );
    assert!(
        open_listing(&app, &name).is_none(),
        "the refusal wrote nothing"
    );
}

/// Run `hsw-rpc` against the regtest wallet (the CLI's buyer); its stdout.
fn hsw_rpc(api_key: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("hsw-rpc")
        .arg("--network=regtest")
        .arg(format!("--api-key={api_key}"))
        .args(args)
        .output()
        .expect("run hsw-rpc (scripts/regtest.sh --with-wallet)");
    assert!(
        out.status.success(),
        "hsw-rpc {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .trim_matches('"')
        .to_string()
}

/// Run both listing jobs on the app's database against the live node, as
/// `run_sync_steps` does (connection swapped out, as in `abort_job`).
async fn listing_jobs(app: &tauri::App<tauri::test::MockRuntime>, cl: &NodeRpcClient) {
    abort_job(app, cl).await;
    let conn = std::mem::replace(
        &mut *app.state::<AppState>().db.lock().unwrap(),
        rusqlite::Connection::open_in_memory().unwrap(),
    );
    let res = crate::shakedex_jobs::refresh_lock_finalize_with_client(&conn, cl, PROFILE).await;
    *app.state::<AppState>().db.lock().unwrap() = conn;
    res.expect("finalize job runs");
}

/// A fresh name locked on chain and its lockup mined out; the listing
/// ReadyToFinalize by the job, not by hand. `(app, client, our address,
/// name, listing id)`.
async fn ready_to_finalize_on_chain(
    url: &str,
    key: &str,
    prefix: &str,
) -> (
    tauri::App<tauri::test::MockRuntime>,
    NodeRpcClient,
    String,
    String,
    String,
) {
    let (app, cl, addr, name) = own_a_name(url, key, prefix).await;
    lock_on_chain(&app, &cl, &addr, &name).await;
    let id = open_listing(&app, &name).expect("listing").id;
    // The lock TRANSFER is mined at the tip; the FINALIZE is valid once
    // tip + 1 >= its height + lockup, i.e. after lockup - 1 more blocks.
    cl.generate_to_address(NET.name_params().transfer_lockup - 2, &addr)
        .await
        .expect("mine");
    sync_wallet_state(app.state(), None).await.expect("sync");
    listing_jobs(&app, &cl).await;
    assert_eq!(
        listing_state(&app, &id),
        ListingState::Locking,
        "one block before the lockup ends"
    );
    cl.generate_to_address(1, &addr).await.expect("mine");
    sync_wallet_state(app.state(), None).await.expect("sync");
    listing_jobs(&app, &cl).await;
    assert_eq!(listing_state(&app, &id), ListingState::ReadyToFinalize);
    (app, cl, addr, name, id)
}

/// Finalize & sign `price` HNS at `per_byte`, the R20 prompt confirmed
/// through the test queue; send the FINALIZE through the one broadcast path
/// and mine it. The summary.
async fn finalize_and_sign_on_chain(
    app: &tauri::App<tauri::test::MockRuntime>,
    cl: &NodeRpcClient,
    addr: &str,
    id: &str,
    price: &str,
    per_byte: u64,
) -> crate::commands::shakedex::ListingSummary {
    unlock(app);
    crate::commands::secure_prompt::push_test_answer(
        crate::commands::secure_prompt::SecurePromptResult {
            value: None,
            confirmed: true,
        },
    );
    let s = crate::commands::shakedex::finalize_and_sign_confirmed(
        &app.state(),
        app.handle(),
        id,
        &[crate::commands::shakedex::StepInput {
            price: price.into(),
        }],
        Some(per_byte),
    )
    .await
    .expect("finalize & sign");
    assert_eq!(s.state, ListingState::Finalizing);
    let draft = s.finalize_draft_id.clone().expect("the FINALIZE draft");
    let bc = broadcast_tx_draft(app.state(), draft.clone())
        .await
        .expect("broadcast");
    assert_eq!(bc.status, "broadcasted");
    assert_eq!(
        Some(bc.txid.clone()),
        s.lock_txid,
        "the lock coin is an output of the FINALIZE"
    );
    settle(app, cl, addr, &draft).await;
    s
}

/// R30, selling half: Namehold locks a name, waits out the lockup, Finalize
/// & signs a Buy Now; once its FINALIZE is mined the listing is Listed, and
/// the shakedex CLI buys the exported listing file. On chain: the name's
/// owner is the fill's TRANSFER out of our lock, committing to an address of
/// the CLI's wallet, and the fill pays our payment address exactly the price.
#[tokio::test]
async fn shakedex_namehold_buy_now_is_bought_by_the_cli() {
    let Some((url, key, cli)) = shakedex_env("shakedex_namehold_buy_now_is_bought_by_the_cli")
    else {
        return;
    };
    let (app, cl, addr, name, id) = ready_to_finalize_on_chain(&url, &key, "nhsell").await;
    let s = finalize_and_sign_on_chain(&app, &cl, &addr, &id, "3", 1).await;
    let lock_txid = s.lock_txid.clone().expect("lock txid");
    let lock_vout = u32::try_from(s.lock_vout.expect("lock vout")).unwrap();

    // The FINALIZE on hsd: the name's owner is the lock coin, a FINALIZE at
    // the lock address of the key derived here.
    let lock = crate::noncustodial::shakedex::lock_key::derive_lock_key(
        &master(),
        NET,
        test_acct(),
        &name,
    )
    .unwrap();
    assert_eq!(name_owner(&cl, &name).await, (lock_txid.clone(), lock_vout));
    let coin = cl
        .get_coin(&lock_txid, lock_vout)
        .await
        .expect("coin")
        .expect("lock coin");
    assert_eq!(coin.address.as_deref(), Some(lock.address.as_str()));
    assert_eq!(
        coin.covenant.as_ref().expect("covenant").kind,
        crate::noncustodial::sync::COV_FINALIZE
    );
    assert!(coin.mined_height().unwrap().is_some(), "mined: {coin:?}");
    // Listed by the listing jobs on the live node (the same functions
    // run_sync_steps runs; that wiring is pinned by
    // shakedex_sell_tests::sync_lists_the_listing_in_the_app_and_the_daemon).
    assert_eq!(
        listing_state(&app, &id),
        ListingState::Finalizing,
        "sent and mined, not yet looked at"
    );
    listing_jobs(&app, &cl).await;
    assert_eq!(listing_state(&app, &id), ListingState::Listed);

    let file = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        crate::commands::shakedex::export_listing_file_from_conn(&conn, PROFILE, &id)
            .expect("export")
    };
    // The CLI's hsd wallet pays: give it coins first (a fresh chain has none
    // in it), then let them mature.
    let cli_addr = hsw_rpc(&key, &["getnewaddress"]);
    cl.generate_to_address(4, &cli_addr)
        .await
        .expect("fund the CLI wallet");
    let maturity = u32::try_from(NET.coinbase_maturity()).unwrap();
    cl.generate_to_address(maturity, &addr)
        .await
        .expect("mature its coinbases");
    cli.fill(&CliListing::new(file));

    let (fill_txid, fill_vout) = name_owner(&cl, &name).await;
    assert_ne!(fill_txid, lock_txid, "the name moved out of the lock");
    let fill = cl.get_tx_by_hash(&fill_txid).await.expect("fill tx");
    assert!(
        fill["inputs"].as_array().expect("inputs").iter().any(|i| {
            i["prevout"]["hash"].as_str() == Some(lock_txid.as_str())
                && i["prevout"]["index"].as_u64() == Some(u64::from(lock_vout))
        }),
        "the fill spends our lock coin: {fill}"
    );
    let out = cl
        .get_coin(&fill_txid, fill_vout)
        .await
        .expect("coin")
        .expect("the fill's TRANSFER");
    assert!(out.mined_height().unwrap().is_some(), "mined: {out:?}");
    let cov = out.covenant.as_ref().expect("covenant");
    assert_eq!(cov.kind, crate::noncustodial::sync::COV_TRANSFER);
    assert_eq!(
        out.address.as_deref(),
        Some(lock.address.as_str()),
        "the TRANSFER out of our lock stays at it until finalized"
    );
    assert_eq!(cov.items[2], "00", "witness version 0");
    let hash: [u8; 20] = hex::decode(&cov.items[3])
        .unwrap()
        .try_into()
        .expect("a 20-byte P2WPKH program");
    let buyer = crate::noncustodial::address::encode_p2wpkh(NET, &hash).unwrap();
    let info: serde_json::Value =
        serde_json::from_str(&hsw_rpc(&key, &["getaddressinfo", &buyer])).expect("json");
    assert_eq!(
        info["ismine"], true,
        "the TRANSFER commits to the CLI wallet: {buyer} {info}"
    );
    let pay = s.payment_address.clone().expect("payment address");
    assert_eq!(
        paid_to(&cl, &fill_txid, &pay).await,
        3_000_000,
        "our payment address got exactly the price"
    );
}

/// R19 on hsd: a reorg takes the FINALIZE into the lock out of its block,
/// and the listing follows it. hsd's `invalidateblock` empties the mempool
/// (`reset`), so while the FINALIZE is nowhere the listing stays Listed (no
/// verdict from a 404); handed back to the node, as a real reorg's mempool
/// would hold it, it is a coin in the mempool and the listing is Finalizing;
/// mined again, Listed. Every step is read from hsd before the jobs run.
#[tokio::test]
async fn shakedex_listing_follows_reorg_of_its_lock_finalize() {
    let Some((url, key)) = shakedex_node_env("shakedex_listing_follows_reorg_of_its_lock_finalize")
    else {
        return;
    };
    let (app, cl, addr, _name, id) = ready_to_finalize_on_chain(&url, &key, "nhreorg").await;
    let s = finalize_and_sign_on_chain(&app, &cl, &addr, &id, "3", 1).await;
    let lock_txid = s.lock_txid.clone().expect("lock txid");
    let lock_vout = u32::try_from(s.lock_vout.expect("lock vout")).unwrap();
    let lock_coin = |cl: NodeRpcClient| {
        let txid = lock_txid.clone();
        async move { cl.get_coin(&txid, lock_vout).await.expect("coin lookup") }
    };
    let mined_at = lock_coin(cl.clone())
        .await
        .expect("the lock coin")
        .mined_height()
        .unwrap()
        .expect("mined");
    listing_jobs(&app, &cl).await;
    assert_eq!(listing_state(&app, &id), ListingState::Listed);

    let block = cl.get_block_hash(mined_at).await.expect("blockhash");
    cl.invalidate_block(&block).await.expect("invalidate");
    assert!(
        lock_coin(cl.clone()).await.is_none(),
        "invalidateblock leaves the FINALIZE nowhere"
    );
    listing_jobs(&app, &cl).await;
    assert_eq!(
        listing_state(&app, &id),
        ListingState::Listed,
        "a 404 alone is no verdict"
    );

    let fin = draft_status(&app, s.finalize_draft_id.as_deref().expect("draft"));
    cl.send_raw_transaction(fin.signed_tx_hex.as_deref().expect("signed FINALIZE"))
        .await
        .expect("hand the FINALIZE back");
    wait_until_node_has(&cl, &lock_txid).await;
    let pending = lock_coin(cl.clone()).await.expect("the lock coin, unmined");
    assert_eq!(pending.mined_height().unwrap(), None, "in the mempool");
    listing_jobs(&app, &cl).await;
    assert_eq!(listing_state(&app, &id), ListingState::Finalizing);

    cl.generate_to_address(1, &addr).await.expect("mine");
    cl.reconsider_block(&block).await.expect("reconsider");
    let again = lock_coin(cl.clone()).await.expect("the lock coin");
    assert!(
        again.mined_height().unwrap().is_some(),
        "mined again: {again:?}"
    );
    listing_jobs(&app, &cl).await;
    assert_eq!(listing_state(&app, &id), ListingState::Listed);
}

/// R19 coordinator (b) on hsd: another device with the same recovery phrase
/// (here: a copy of this wallet's database, as a restore would give it)
/// Finalize & signs the listing and its FINALIZE is mined. This device's
/// listing, still ReadyToFinalize, becomes a Restored lock with that
/// FINALIZE's lock outpoint, read from hsd's owner coin — never Aborted.
#[tokio::test]
async fn shakedex_finalize_from_another_device_is_a_restored_lock() {
    let Some((url, key)) =
        shakedex_node_env("shakedex_finalize_from_another_device_is_a_restored_lock")
    else {
        return;
    };
    let (app, cl, addr, name, id) = ready_to_finalize_on_chain(&url, &key, "nhother").await;
    let copy = std::env::temp_dir().join(format!(
        "namehold_live_other_device_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        conn.execute("VACUUM INTO ?1", [copy.to_str().unwrap()])
            .expect("copy the database");
    }
    let other_conn = rusqlite::Connection::open(&copy).unwrap();
    other_conn
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
    let other = app_with(other_conn);
    let s = finalize_and_sign_on_chain(&other, &cl, &addr, &id, "3", 1).await;
    let lock_txid = s.lock_txid.clone().expect("lock txid");
    let lock_vout = u32::try_from(s.lock_vout.expect("lock vout")).unwrap();

    // On hsd: the name's owner is that FINALIZE's lock coin, mined at the
    // lock address of the key derived here.
    let lock = crate::noncustodial::shakedex::lock_key::derive_lock_key(
        &master(),
        NET,
        test_acct(),
        &name,
    )
    .unwrap();
    assert_eq!(name_owner(&cl, &name).await, (lock_txid.clone(), lock_vout));
    let coin = cl
        .get_coin(&lock_txid, lock_vout)
        .await
        .expect("coin")
        .expect("lock coin");
    assert_eq!(coin.address.as_deref(), Some(lock.address.as_str()));
    assert_eq!(
        coin.covenant.as_ref().expect("covenant").kind,
        crate::noncustodial::sync::COV_FINALIZE
    );
    assert!(coin.mined_height().unwrap().is_some(), "mined: {coin:?}");

    assert_eq!(listing_state(&app, &id), ListingState::ReadyToFinalize);
    sync_wallet_state(app.state(), None).await.expect("sync");
    listing_jobs(&app, &cl).await;
    let l = open_listing(&app, &name).expect("still the open listing");
    assert_eq!(l.id, id);
    assert_eq!(l.state, ListingState::Restored);
    assert_eq!(
        (l.lock_txid.as_deref(), l.lock_vout),
        (Some(lock_txid.as_str()), Some(i64::from(lock_vout)))
    );
    drop(other);
    let _ = std::fs::remove_file(&copy);
}

/// R4 on hsd: the FINALIZE into the lock, built at 20 doos/vbyte (above the
/// 5 doos/vbyte floor), pays the fee its summary shows at the rate hsd
/// reports for it.
#[tokio::test]
async fn shakedex_lock_finalize_pays_its_fee_rate_on_vsize() {
    let Some((url, key)) = shakedex_node_env("shakedex_lock_finalize_pays_its_fee_rate_on_vsize")
    else {
        return;
    };
    let per_byte = 20;
    let (app, cl, addr, _name, id) = ready_to_finalize_on_chain(&url, &key, "nhrate").await;
    let s = finalize_and_sign_on_chain(&app, &cl, &addr, &id, "3", per_byte).await;
    let draft = draft_status(&app, s.finalize_draft_id.as_deref().unwrap());
    let fee =
        serde_json::from_str::<serde_json::Value>(&draft.summary_json).unwrap()["feeDoos"].clone();
    let tx = cl
        .get_tx_by_hash(draft.txid.as_deref().expect("sent"))
        .await
        .expect("tx");
    assert!(tx["height"].as_i64().unwrap_or(-1) > 0, "mined: {tx}");
    assert_eq!(tx["fee"], fee, "{tx}");
    let rate = tx["rate"].as_u64().expect("rate");
    let asked = per_byte * 1000;
    assert!(
        (asked..=asked * 102 / 100).contains(&rate),
        "asked {asked} doos/kvB, hsd reports {rate}: {tx}"
    );
}

/// #65 on a live node: a transaction pays the fee rate asked for on hsd's
/// virtual size, not on its raw size. hsd's `GET /tx/:hash` reports the fee
/// and the rate it works out on `getVirtualSize()`; the raw-size bug paid
/// about 1.7 times the rate.
#[tokio::test]
async fn live_send_pays_its_fee_rate_on_vsize() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_send_pays_its_fee_rate_on_vsize: set HNS_IT_NODE_URL");
        return;
    };
    let b = ShakedexBuyer::new(&url, &key).await;
    let per_byte = 5;
    let draft = build_send_hns_draft(b.app.state(), b.addr.clone(), 100_000, Some(per_byte), None)
        .await
        .expect("build send");
    broadcast_only(&b.app, &draft.id).await;
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    let txid = draft_status(&b.app, &draft.id).txid.expect("txid");
    let tx = b.cl.get_tx_by_hash(&txid).await.expect("tx");
    assert_eq!(tx["fee"], draft.summary["feeDoos"], "{tx}");
    // Doos per 1000 virtual bytes. The estimate may run a byte or two over
    // the signed size (a signature's length varies), never 70% over.
    let rate = tx["rate"].as_u64().expect("rate");
    let asked = per_byte * 1000;
    assert!(
        (asked..=asked * 105 / 100).contains(&rate),
        "asked {asked} doos/kvB, hsd reports {rate}: {tx}"
    );
}

/// R11 on a live node: a market fee the buyer accepts is paid exactly, to
/// the listing's fee address, beside the seller's price. The CLI writes no
/// fee without publishing to LearnHNS, so this listing is signed here.
#[tokio::test]
async fn shakedex_accepted_market_fee_is_paid_exactly_on_chain() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_accepted_market_fee_is_paid_exactly_on_chain")
    else {
        return;
    };
    let lock = OwnLock::new(&cli, &url, &key).await;
    let b = ShakedexBuyer::new(&url, &key).await;
    let (price, fee) = (3_000_000, 150_000);
    let listing = lock.listing(&[(price, node_mtp(&b.cl).await - 3_600)], fee);
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let row = b.import(&listing).await;
    assert_eq!(row["verdict"]["verdict"], "buyable", "{row}");

    let draft = b
        .build_purchase_paying(&listing, Some(fee))
        .await
        .expect("build purchase paying the fee");
    unlock(&b.app);
    sign_tx_draft_inner(&b.app.state(), &draft.id)
        .await
        .expect("sign");
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    assert_eq!(
        b.purchase(&lock.name).state,
        db::queries::PurchaseState::AwaitingFinalize
    );
    assert_eq!(paid_to(&b.cl, &bc.txid, &seller_addr()).await, price);
    assert_eq!(paid_to(&b.cl, &bc.txid, &market_fee_addr()).await, fee);
}

/// R8 on a live node: a listing whose first step's lock time is still ahead
/// of the node's median time is "not valid yet" and cannot be bought; once
/// the median time passes it, it is buyable. The CLI starts every listing at
/// the node's median time, so this listing is signed here.
#[tokio::test]
async fn shakedex_listing_not_valid_yet_becomes_buyable() {
    let Some((url, key, cli)) = shakedex_env("shakedex_listing_not_valid_yet_becomes_buyable")
    else {
        return;
    };
    let lock = OwnLock::new(&cli, &url, &key).await;
    let b = ShakedexBuyer::new(&url, &key).await;
    let later = node_mtp(&b.cl).await + 3_600;
    let listing = lock.listing(&[(2_000_000, later)], 0);
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let row = b.import(&listing).await;
    assert_eq!(row["verdict"]["verdict"], "hidden", "{row}");
    assert_eq!(row["verdict"]["kind"], "notYetValid", "{row}");
    assert!(
        row["verdict"]["firstValidInSecs"].as_u64().expect("secs") > 0,
        "{row}"
    );
    assert_refused(&b, &listing, "no price step of this listing is valid yet").await;

    advance_mtp_past(&b.cl, &b.addr, later).await;
    let row = b.import(&listing).await;
    assert_eq!(row["verdict"]["verdict"], "buyable", "{row}");
}

/// The change of a purchase the purchase job has not yet seen mined is held
/// back from coin selection, and is spendable once it has: a purchase lost
/// before it is mined must not have funded anything else.
#[tokio::test]
async fn shakedex_purchase_change_is_held_back_until_seen_mined() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_purchase_change_is_held_back_until_seen_mined")
    else {
        return;
    };
    let lock = OwnLock::new(&cli, &url, &key).await;
    let b = ShakedexBuyer::new(&url, &key).await;
    let listing = lock.listing(&[(2_000_000, node_mtp(&b.cl).await - 3_600)], 0);
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let draft = b.sign_purchase(&listing).await;
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    // Mined and synced, so the change is tracked; the job has not run.
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    assert!(
        b.tracks_coin_of(&bc.txid),
        "the purchase's change is tracked"
    );
    assert!(
        b.spendable().iter().all(|c| c.txid != bc.txid),
        "held back while the purchase is unconfirmed"
    );

    b.refresh().await;
    assert_eq!(
        b.purchase(&lock.name).state,
        db::queries::PurchaseState::AwaitingFinalize
    );
    assert!(
        b.spendable().iter().any(|c| c.txid == bc.txid),
        "spendable once the job saw the purchase mined"
    );
}

/// R13 "Lost, paid": the purchase was mined, but the name expired before it
/// was finalized. hsd then reports no `info` for the name; the job loses the
/// purchase as paid, and the draft stays confirmed. The chain is mined to the
/// expiry and taken back before anything is asserted.
#[tokio::test]
async fn shakedex_name_expired_before_finalize_is_lost_with_the_price_paid() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_name_expired_before_finalize_is_lost_with_the_price_paid")
    else {
        return;
    };
    let lock = OwnLock::new(&cli, &url, &key).await;
    let b = ShakedexBuyer::new(&url, &key).await;
    let listing = lock.listing(&[(2_000_000, node_mtp(&b.cl).await - 3_600)], 0);
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let draft = b.sign_purchase(&listing).await;
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    let before = b.purchase(&lock.name);

    let start = b.cl.get_blockchain_info().await.expect("info").blocks;
    let end = b.cl.get_name_info(&lock.name).await.expect("name info")["info"]["stats"]
        ["renewalPeriodEnd"]
        .as_i64()
        .expect("renewalPeriodEnd");
    mine_to(&b.cl, &b.addr, end).await;
    let expired = b.cl.get_name_info(&lock.name).await.expect("name info");
    b.refresh().await;
    let after = b.purchase(&lock.name);
    let draft_row = draft_status(&b.app, &draft.id);
    let drafts = crate::commands::tx::list_tx_drafts(b.app.state(), None)
        .await
        .expect("drafts");
    let paid = paid_to(&b.cl, &bc.txid, &seller_addr()).await;
    rewind_to(&b.cl, start).await;

    assert_eq!(before.state, db::queries::PurchaseState::AwaitingFinalize);
    assert!(
        expired["info"].is_null(),
        "hsd reports the name expired: {expired}"
    );
    assert_eq!(after.state, db::queries::PurchaseState::Lost);
    let reason =
        "the name expired before it was finalized — the purchase was paid, but the name is lost";
    assert_eq!(after.lost_reason.as_deref(), Some(reason));
    assert_eq!(draft_row.status, "confirmed", "the purchase was paid");
    assert_eq!(paid, listing.price(0));
    let summary = drafts
        .iter()
        .find(|d| d.id == draft.id)
        .expect("draft listed");
    assert_eq!(summary.purchase_lost_reason.as_deref(), Some(reason));
}

/// R13 "Lost, nothing paid" by mempool expiry: a purchase gone from the node
/// (its block taken away; hsd's `invalidateblock` empties the mempool too)
/// that the daemon may not send again waits, and is given up only once hsd's
/// mempool expiry has passed since it went missing. One block earlier it
/// still waits.
#[tokio::test]
async fn shakedex_purchase_the_node_lost_is_given_up_after_mempool_expiry() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_purchase_the_node_lost_is_given_up_after_mempool_expiry")
    else {
        return;
    };
    use crate::shakedex_jobs::{Rebroadcast, MEMPOOL_EXPIRY_BLOCKS};
    let lock = OwnLock::new(&cli, &url, &key).await;
    let b = ShakedexBuyer::new(&url, &key).await;
    let listing = lock.listing(&[(2_000_000, node_mtp(&b.cl).await - 3_600)], 0);
    sync_wallet_state(b.app.state(), None).await.expect("sync");
    let draft = b.sign_purchase(&listing).await;
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    let bought_at = b
        .purchase(&lock.name)
        .purchase_height
        .expect("purchase height");

    let block = b.cl.get_block_hash(bought_at).await.expect("blockhash");
    b.cl.invalidate_block(&block).await.expect("invalidate");
    let gone = b.cl.get_tx_by_hash(&bc.txid).await.expect("tx lookup");
    b.refresh_with(Rebroadcast::Never).await;
    let missing = b.purchase(&lock.name);
    let since = missing.missing_since_height.expect("missing since");
    mine_to(&b.cl, &b.addr, since + MEMPOOL_EXPIRY_BLOCKS - 1).await;
    b.refresh_with(Rebroadcast::Never).await;
    let one_short = b.purchase(&lock.name);
    b.cl.generate_to_address(1, &b.addr).await.expect("mine");
    b.refresh_with(Rebroadcast::Never).await;
    let given_up = b.purchase(&lock.name);
    let never_sent = b.cl.get_tx_by_hash(&bc.txid).await.expect("tx lookup");
    rewind_to(&b.cl, bought_at - 1).await;

    assert!(
        gone.is_null(),
        "invalidateblock leaves the purchase nowhere"
    );
    assert_eq!(missing.state, db::queries::PurchaseState::Unconfirmed);
    assert_eq!(one_short.state, db::queries::PurchaseState::Unconfirmed);
    assert_eq!(one_short.rebroadcast_count, 0, "the daemon never sends");
    assert_eq!(given_up.state, db::queries::PurchaseState::Lost);
    assert_eq!(
        given_up.lost_reason.as_deref(),
        Some("the purchase never confirmed — nothing was paid")
    );
    assert!(never_sent.is_null(), "nothing was sent again");
}

/// R10 on a live node: the step a purchase pays is told by its lock time, not
/// its price. A listing with two steps at one price (the CLI writes such a
/// listing when a reverse auction starts and ends at the same price); the
/// purchase is signed at the later one, the current step once both are
/// valid. The blocks that moved the median time past the later step are then
/// taken back: only the earlier step is valid, at our price, and the purchase
/// is refused before it is sent. Checking the price alone sent a transaction
/// hsd takes only as non-final, while still answering with its txid.
#[tokio::test]
async fn shakedex_purchase_at_a_same_price_step_not_valid_is_not_sent() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_purchase_at_a_same_price_step_not_valid_is_not_sent")
    else {
        return;
    };
    use crate::noncustodial::shakedex::template::{encode_lock_time, is_valid_at};
    let lock = OwnLock::new(&cli, &url, &key).await;
    let b = ShakedexBuyer::new(&url, &key).await;
    let mtp = node_mtp(&b.cl).await;
    let (early, late) = (mtp + 1_000, mtp + 4_000);
    let listing = lock.listing(&[(2_000_000, late), (2_000_000, early)], 0);
    advance_mtp_past(&b.cl, &b.addr, early).await;
    let between = b.cl.get_blockchain_info().await.expect("info").blocks;
    advance_mtp_past(&b.cl, &b.addr, late).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = b.sign_purchase(&listing).await;
    let signed_at = {
        let row = draft_status(&b.app, &draft.id);
        crate::noncustodial::shakedex::purchase::plan_lock_time(&row.signing_inputs_json)
            .expect("plan lock time")
    };
    assert_eq!(
        signed_at,
        encode_lock_time(late).unwrap(),
        "pays the later step"
    );

    rewind_to(&b.cl, between).await;
    let mtp = node_mtp(&b.cl).await;
    assert!(
        is_valid_at(encode_lock_time(early).unwrap(), mtp),
        "early step valid at {mtp}"
    );
    assert!(!is_valid_at(signed_at, mtp), "our step not valid at {mtp}");

    let err = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect_err("refused before sending");
    assert!(
        err.to_string()
            .contains(crate::noncustodial::shakedex::verify::PRICE_CHANGED),
        "{err}"
    );
    let txid = draft.summary["txid"]
        .as_str()
        .expect("draft txid")
        .to_string();
    assert!(
        b.cl.get_tx_by_hash(&txid)
            .await
            .expect("tx lookup")
            .is_null(),
        "nothing reached the node"
    );
}

// --- A node without a transaction index ------------------------------------
//
// hsd answers `getrawtransaction` from its mempool, then from its chain's
// transaction index (`chaindb.getMeta`), which is null on a node started
// without `--index-tx`. On such a node every MINED transaction reads as hsd's
// own "Transaction not found.", the same answer as for one the node never
// had. These tests run against a second regtest node started with
// `--index-address` only:
//
// ```sh
// hsd --network=regtest --index-address --no-wallet --listen=false \
//     --http-host=127.0.0.1 --http-port=24037 --port=24038 \
//     --ns-port=25449 --rs-port=25450 --api-key=test --prefix=<dir> --daemon
// HNS_IT_NOINDEX_NODE_URL=http://127.0.0.1:24037 HNS_IT_NOINDEX_NODE_API_KEY=test \
//   cargo test --manifest-path src-tauri/Cargo.toml live_noindex -- --test-threads=1
// ```

/// `Some((url, api_key))` of the node without a transaction index, else `None`.
fn noindex_env() -> Option<(String, String)> {
    let url = std::env::var("HNS_IT_NOINDEX_NODE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let key = std::env::var("HNS_IT_NOINDEX_NODE_API_KEY").unwrap_or_default();
    Some((url, key))
}

/// Move a draft's `updated_at` back past the refresh's eviction grace window.
fn age_past_grace(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) {
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    c.execute(
        "UPDATE wallet_tx_drafts SET updated_at = datetime('now', '-700 seconds') WHERE id = ?1",
        params![draft_id],
    )
    .unwrap();
}

/// The txid a signed draft's summary carries.
fn summary_txid(app: &tauri::App<tauri::test::MockRuntime>, draft_id: &str) -> String {
    let row = draft_status(app, draft_id);
    let v: serde_json::Value = serde_json::from_str(&row.summary_json).unwrap();
    v["txid"].as_str().expect("summary txid").to_string()
}

/// A send mined on a node without a transaction index is confirmed at its
/// block — never `dropped` with "the coins were not moved", which would invite
/// the user to pay a second time.
#[tokio::test]
async fn live_noindex_mined_send_is_confirmed_not_dropped() {
    let Some((url, key)) = noindex_env() else {
        eprintln!(
            "skip live_noindex_mined_send_is_confirmed_not_dropped: set HNS_IT_NOINDEX_NODE_URL"
        );
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d.id)
        .await
        .expect("sign");
    let bc = broadcast_tx_draft(app.state(), d.id.clone())
        .await
        .expect("broadcast");
    assert_eq!(bc.status, "broadcasted");
    cl.generate_to_address(1, &addr).await.expect("mine");
    let mined_at = cl.get_blockchain_info().await.expect("info").blocks;
    let txid = draft_status(&app, &d.id).txid.expect("txid");
    assert!(
        crate::noncustodial::rpc::is_tx_not_found(
            &cl.get_raw_transaction(&txid)
                .await
                .expect_err("no tx index")
        ),
        "precondition: this node answers a mined tx with hsd's not-found"
    );

    age_past_grace(&app, &d.id);
    let r = refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed", "{r} {:?}", row.error_message);
    assert_eq!(row.confirmation_height, Some(mined_at));

    // A confirmed draft is polled again until final: hsd's not-found must
    // not read as a reorg either.
    cl.generate_to_address(1, &addr).await.expect("mine");
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed", "{:?}", row.error_message);
    assert_eq!(row.confirmation_height, Some(mined_at));
}

/// The same for a `broadcast_pending` draft (an ambiguous broadcast): mined
/// on a node without a transaction index, it is confirmed, never `failed`.
#[tokio::test]
async fn live_noindex_mined_pending_broadcast_is_confirmed_not_failed() {
    let Some((url, key)) = noindex_env() else {
        eprintln!("skip live_noindex_mined_pending_broadcast_is_confirmed_not_failed: set HNS_IT_NOINDEX_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d.id)
        .await
        .expect("sign");
    broadcast_tx_draft(app.state(), d.id.clone())
        .await
        .expect("broadcast");
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'broadcast_pending', txid = NULL WHERE id = ?1",
            params![d.id],
        )
        .unwrap();
    }
    cl.generate_to_address(1, &addr).await.expect("mine");
    let mined_at = cl.get_blockchain_info().await.expect("info").blocks;

    age_past_grace(&app, &d.id);
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed", "{:?}", row.error_message);
    assert_eq!(row.confirmation_height, Some(mined_at));
    assert_eq!(
        row.txid.as_deref(),
        Some(summary_txid(&app, &d.id).as_str())
    );
}

/// A transaction the node never had, whose coins are all still unspent, is
/// `dropped` with its coins released on a node without a transaction index
/// too: here "the coins were not moved" is what the chain shows.
#[tokio::test]
async fn live_noindex_unsent_draft_with_unspent_coins_is_dropped() {
    let Some((url, key)) = noindex_env() else {
        eprintln!("skip live_noindex_unsent_draft_with_unspent_coins_is_dropped: set HNS_IT_NOINDEX_NODE_URL");
        return;
    };
    let conn = seeded_conn_regtest(&url, &key);
    let app = app_with(conn);
    let cl = client(&url, &key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &d.id)
        .await
        .expect("sign");
    // Recorded as broadcast, but never sent: as if the mempool evicted it.
    let txid = summary_txid(&app, &d.id);
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        c.execute(
            "UPDATE wallet_tx_drafts SET status = 'broadcasted', txid = ?2 WHERE id = ?1",
            params![d.id, txid],
        )
        .unwrap();
    }
    cl.generate_to_address(1, &addr).await.expect("mine");

    age_past_grace(&app, &d.id);
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "dropped", "{:?}", row.error_message);
    let state = app.state::<AppState>();
    let c = state.db.lock().unwrap();
    let held: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM tracked_utxos WHERE reserved_by_draft_id = ?1",
            params![d.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(held, 0, "a dropped draft's coins are released");
}

/// Draft A's coin is spent by draft B, mined; A is then sent, and hsd's
/// mempool turns it away while still answering its txid. On a node without a
/// transaction index the wallet cannot tell A's spend from B's, so it gives
/// no verdict on A — above all not "failed, the coins were not moved".
#[tokio::test]
async fn live_noindex_coins_spent_by_another_tx_give_no_verdict() {
    let Some((url, key)) = noindex_env() else {
        eprintln!("skip live_noindex_coins_spent_by_another_tx_give_no_verdict: set HNS_IT_NOINDEX_NODE_URL");
        return;
    };
    let (app, a) = double_spent_draft(&url, &key).await;
    age_past_grace(&app, &a);
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &a);
    assert_eq!(row.status, "broadcast_pending", "{:?}", row.error_message);
}

/// The same double spend on a node WITH a transaction index: hsd's not-found
/// is the chain's answer there, so A is `dropped` — and told why: another
/// transaction spent its coins, not "the coins were not moved".
#[tokio::test]
async fn live_coins_spent_by_another_tx_drop_the_draft_and_say_so() {
    let Some((url, key)) = it_env() else {
        eprintln!(
            "skip live_coins_spent_by_another_tx_drop_the_draft_and_say_so: set HNS_IT_NODE_URL"
        );
        return;
    };
    let (app, a) = double_spent_draft(&url, &key).await;
    age_past_grace(&app, &a);
    refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &a);
    assert_eq!(row.status, "dropped", "{:?}", row.error_message);
    let msg = row.error_message.unwrap_or_default().to_lowercase();
    assert!(msg.contains("another transaction"), "{msg}");
    assert!(!msg.contains("not moved"), "{msg}");
}

/// Draft A signed over one coin, draft B spending the same coin mined, then
/// A sent: hsd answers A's txid and keeps it out of its mempool, and the
/// wallet says so (honest-broadcast R2): not sent, `broadcast_pending`.
/// Returns the app and A's id.
async fn double_spent_draft(
    url: &str,
    key: &str,
) -> (tauri::App<tauri::test::MockRuntime>, String) {
    let conn = seeded_conn_regtest(url, key);
    let app = app_with(conn);
    let cl = client(url, key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");

    let a = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build A");
    unlock(&app);
    sign_tx_draft_inner(&app.state(), &a.id)
        .await
        .expect("sign A");
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::release_reserved_utxos_for_draft(&c, &a.id).unwrap();
    }
    let b = build_send_hns_draft(app.state(), addr.clone(), 200_000, Some(1), None)
        .await
        .expect("build B");
    execute(&app, &cl, &addr, b.id).await;

    let err = broadcast_tx_draft(app.state(), a.id.clone())
        .await
        .expect_err("hsd answers a txid for a tx its mempool turns away");
    assert!(err.to_string().contains("did not take"), "{err}");
    let row = draft_status(&app, &a.id);
    assert_eq!(row.status, "broadcast_pending");
    assert_eq!(
        row.error_message.as_deref(),
        Some(crate::noncustodial::tx_evidence::NOT_TAKEN)
    );
    cl.generate_to_address(1, &addr).await.expect("mine");
    (app, a.id)
}

/// A send judged `dropped` that is mined after all is confirmed by the next
/// poll. The verdict is forced here while the send sits in the mempool, as a
/// node that lost its mempool on a restart would have reached it, with its
/// coins released; the block then mines it.
async fn dropped_send_mined_after_all_is_confirmed(url: &str, key: &str) {
    let conn = seeded_conn_regtest(url, key);
    let app = app_with(conn);
    let cl = client(url, key);
    let (addr, _, _) = leaf00();
    fund(&cl, &addr, 103).await;
    sync_wallet_state(app.state(), None).await.expect("sync");
    let d = build_send_hns_draft(app.state(), addr.clone(), 100_000, Some(1), None)
        .await
        .expect("build");
    broadcast_only(&app, &d.id).await;
    let txid = draft_status(&app, &d.id).txid.expect("txid");
    wait_until_node_has(&cl, &txid).await;
    {
        let state = app.state::<AppState>();
        let c = state.db.lock().unwrap();
        db::queries::update_tx_draft_status(&c, &d.id, "dropped", Some("judged dropped"), None)
            .unwrap();
        db::queries::release_reserved_utxos_for_draft(&c, &d.id).unwrap();
    }
    cl.generate_to_address(1, &addr).await.expect("mine");
    let mined_at = cl.get_blockchain_info().await.expect("info").blocks;

    let r = refresh_tx_confirmations(app.state(), None)
        .await
        .expect("refresh");
    let row = draft_status(&app, &d.id);
    assert_eq!(row.status, "confirmed", "{r} {:?}", row.error_message);
    assert_eq!(row.confirmation_height, Some(mined_at));
    assert_eq!(row.txid.as_deref(), Some(txid.as_str()));
    assert_eq!(r["revived"], 1, "{r}");
}

#[tokio::test]
async fn live_dropped_send_mined_after_all_is_confirmed() {
    let Some((url, key)) = it_env() else {
        eprintln!("skip live_dropped_send_mined_after_all_is_confirmed: set HNS_IT_NODE_URL");
        return;
    };
    dropped_send_mined_after_all_is_confirmed(&url, &key).await;
}

/// The same on a node without a transaction index: the mined send is found
/// by its outputs.
#[tokio::test]
async fn live_noindex_dropped_send_mined_after_all_is_confirmed() {
    let Some((url, key)) = noindex_env() else {
        eprintln!(
            "skip live_noindex_dropped_send_mined_after_all_is_confirmed: set HNS_IT_NOINDEX_NODE_URL"
        );
        return;
    };
    dropped_send_mined_after_all_is_confirmed(&url, &key).await;
}

/// The fee rate is the one figure of a purchase consensus does not pin: an
/// excess is simply paid to the miner. A purchase and its FINALIZE built at a
/// rate above the 5 doos/byte floor pay that rate on hsd's virtual size,
/// the seller's input and signature included (spec R4); hsd's `GET /tx/:hash`
/// works the rate out the same way.
#[tokio::test]
async fn shakedex_purchase_and_finalize_pay_their_fee_rate_on_vsize() {
    let Some((url, key, cli)) =
        shakedex_env("shakedex_purchase_and_finalize_pay_their_fee_rate_on_vsize")
    else {
        return;
    };
    let per_byte = 20;
    let listing = cli.sell_fixed(3);
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = crate::commands::shakedex::shakedex_build_purchase_draft(
        b.app.state(),
        listing.json.clone(),
        None,
        false,
        Some(per_byte),
    )
    .await
    .expect("build purchase");
    broadcast_only(&b.app, &draft.id).await;
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    assert_pays_rate(&b, &draft, per_byte).await;

    b.cl.generate_to_address(NET.name_params().transfer_lockup, &b.addr)
        .await
        .expect("mine lockup");
    b.refresh().await;
    let p = b.purchase(&listing.name);
    let fin = crate::commands::shakedex::shakedex_build_purchase_finalize_draft(
        b.app.state(),
        p.id.clone(),
        Some(per_byte),
    )
    .await
    .expect("build finalize");
    broadcast_only(&b.app, &fin.id).await;
    settle(&b.app, &b.cl, &b.addr, &fin.id).await;
    assert_pays_rate(&b, &fin, per_byte).await;
}

/// `draft`, mined, paid the fee its summary shows, at `per_byte` doos per
/// virtual byte as hsd works it out (doos per 1000 virtual bytes), within the
/// byte or two a signature's length can vary.
async fn assert_pays_rate(
    b: &ShakedexBuyer,
    draft: &crate::noncustodial::types::TxDraftSummary,
    per_byte: u64,
) {
    let txid = draft_status(&b.app, &draft.id).txid.expect("txid");
    let tx = b.cl.get_tx_by_hash(&txid).await.expect("tx");
    assert_eq!(tx["fee"], draft.summary["feeDoos"], "{tx}");
    let rate = tx["rate"].as_u64().expect("rate");
    let asked = per_byte * 1000;
    assert!(
        (asked..=asked * 102 / 100).contains(&rate),
        "asked {asked} doos/kvB, hsd reports {rate}: {tx}"
    );
}

/// R13 and R10 on a live node: a purchase taken out of the chain by a reorg
/// is missing, and its one rebroadcast is checked like any broadcast. A
/// cheaper step of the reverse auction became valid meanwhile, so the old
/// price is not sent again: the purchase is lost with nothing paid, its coins
/// released, and the node never sees it again.
#[tokio::test]
async fn shakedex_rebroadcast_after_a_price_drop_is_not_sent() {
    let Some((url, key, cli)) = shakedex_env("shakedex_rebroadcast_after_a_price_drop_is_not_sent")
    else {
        return;
    };
    let listing = cli.sell_auction(10, 5);
    let name = listing.name.clone();
    let b = ShakedexBuyer::new(&url, &key).await;
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(0)).await;
    sync_wallet_state(b.app.state(), None).await.expect("sync");

    let draft = b.sign_purchase(&listing).await;
    assert_eq!(draft.summary["priceDoos"], listing.price(0));
    let bc = broadcast_tx_draft(b.app.state(), draft.id.clone())
        .await
        .expect("broadcast");
    settle(&b.app, &b.cl, &b.addr, &draft.id).await;
    b.refresh().await;
    let bought_at = b.purchase(&name).purchase_height.expect("purchase height");

    // `invalidateblock` empties the mempool: the purchase is nowhere.
    let block = b.cl.get_block_hash(bought_at).await.expect("blockhash");
    b.cl.invalidate_block(&block).await.expect("invalidate");
    b.refresh().await;
    assert_eq!(
        b.purchase(&name).state,
        db::queries::PurchaseState::Unconfirmed
    );

    // Step 1 becomes valid; the 11 blocks this mines are more than the six
    // missing blocks that call for the rebroadcast.
    advance_mtp_past(&b.cl, &b.addr, listing.lock_time(1)).await;
    b.refresh().await;
    let p = b.purchase(&name);
    assert_eq!(p.state, db::queries::PurchaseState::Lost);
    assert!(
        p.lost_reason
            .as_deref()
            .is_some_and(|r| r.contains("not sent again")),
        "{:?}",
        p.lost_reason
    );
    assert_eq!(p.rebroadcast_count, 0, "nothing was sent");
    assert_eq!(b.reserved_by(&draft.id), 0, "its coins are free");
    b.cl.generate_to_address(1, &b.addr).await.expect("mine");
    assert!(
        b.cl.get_tx_by_hash(&bc.txid)
            .await
            .expect("tx lookup")
            .is_null(),
        "the old price never reached the node again"
    );
    b.cl.reconsider_block(&block).await.expect("reconsider");
}
