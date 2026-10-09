//! Selling through Shakedex, day 0 (T2): the listings table, the lock
//! TRANSFER draft and its gates (R16, R18, R19, R21, R29, R31), and the abort
//! through the existing Cancel transfer.

use rusqlite::{params, Connection};

use crate::db::queries::{ListingMode, ListingState, ShakedexListing};
use crate::db::{self, queries};

const STORE_PROFILE: &str = "sell1";

fn store_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    queries::insert_wallet_profile(
        &conn,
        STORE_PROFILE,
        "Sell",
        "mnemonic_hot",
        "regtest",
        "xpubFAKE",
        0,
        false,
    )
    .unwrap();
    conn
}

fn listing(id: &str, name: &str, state: ListingState) -> ShakedexListing {
    ShakedexListing {
        id: id.into(),
        wallet_profile_id: STORE_PROFILE.into(),
        name: name.into(),
        mode: ListingMode::BuyNow,
        state,
        lock_pubkey_hex: "02".repeat(33),
        lock_transfer_draft_id: None,
        lock_transfer_txid: None,
        lock_txid: None,
        lock_vout: None,
        payment_address: Some("rs1qpay".into()),
        cancel_address: Some("rs1qcancel".into()),
        cancel_child_index: Some(2),
        steps_json: "[]".into(),
        listing_file_json: None,
        publish: false,
        market_status: None,
        market_retry_at: None,
        expires_at: None,
        abort_draft_id: None,
        abort_txid: None,
        sold_txid: None,
        cancel_txid: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn every_listing_state_round_trips_through_the_table() {
    let conn = store_conn();
    for (i, state) in ListingState::ALL.into_iter().enumerate() {
        // One open listing per name: a name per row.
        let l = listing(&format!("l{i}"), &format!("name{i}"), state);
        queries::insert_shakedex_listing(&conn, &l).unwrap();
        let got = queries::get_shakedex_listing(&conn, &l.id)
            .unwrap()
            .unwrap();
        assert_eq!(got.state, state);
        assert_eq!(got.mode, ListingMode::BuyNow);
        assert_eq!(got.cancel_child_index, Some(2));
    }
    let mut l = listing("ra", "auction", ListingState::Locking);
    l.mode = ListingMode::ReverseAuction;
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    assert_eq!(
        queries::get_shakedex_listing(&conn, "ra")
            .unwrap()
            .unwrap()
            .mode,
        ListingMode::ReverseAuction
    );
}

#[test]
fn an_unknown_listing_state_is_refused_when_read() {
    assert!("owned".parse::<ListingState>().is_err());
    assert_eq!(
        "cancel_awaiting_finalize".parse::<ListingState>().unwrap(),
        ListingState::CancelAwaitingFinalize
    );
    let conn = store_conn();
    let err = conn.execute(
        "INSERT INTO shakedex_listings (id, wallet_profile_id, name, mode, state, lock_pubkey_hex)
         VALUES ('x', ?1, 'n', 'buy_now', 'owned', '02')",
        params![STORE_PROFILE],
    );
    assert!(err.is_err(), "the CHECK refuses a spelling the enum lacks");
}

/// The CHECK lists exactly the spellings `as_str` writes: each one goes in
/// through plain SQL, and a spelling no variant has stays out.
#[test]
fn the_state_and_mode_checks_list_exactly_the_enum_spellings() {
    let conn = store_conn();
    let insert = |id: &str, mode: &str, state: &str| {
        conn.execute(
            "INSERT INTO shakedex_listings (id, wallet_profile_id, name, mode, state, lock_pubkey_hex)
             VALUES (?1, ?2, ?1, ?3, ?4, '02')",
            params![id, STORE_PROFILE, mode, state],
        )
    };
    for state in ListingState::ALL {
        insert(state.as_str(), ListingMode::BuyNow.as_str(), state.as_str())
            .unwrap_or_else(|e| panic!("{state:?} spelling '{}' refused: {e}", state.as_str()));
    }
    for mode in [ListingMode::BuyNow, ListingMode::ReverseAuction] {
        insert(&format!("m-{}", mode.as_str()), mode.as_str(), "locking")
            .unwrap_or_else(|e| panic!("{mode:?} spelling '{}' refused: {e}", mode.as_str()));
        assert_eq!(mode.as_str().parse::<ListingMode>().unwrap(), mode);
    }
    assert!(insert("bad-state", "buy_now", "Locking").is_err());
    assert!(insert("bad-state2", "buy_now", "locked").is_err());
    assert!(insert("bad-mode", "buyNow", "locking").is_err());
    assert!("buyNow".parse::<ListingMode>().is_err());
    // Every variant has a distinct spelling and parses back to itself.
    for state in ListingState::ALL {
        assert_eq!(state.as_str().parse::<ListingState>().unwrap(), state);
    }
}

/// The table's "one open listing per name" index, `is_terminal` and
/// `open_shakedex_listing_for_name` agree on which states are open.
#[test]
fn terminal_states_agree_with_the_open_listing_index() {
    for (i, first) in ListingState::ALL.into_iter().enumerate() {
        let conn = store_conn();
        queries::insert_shakedex_listing(&conn, &listing("a", "dup", first)).unwrap();
        let open = queries::open_shakedex_listing_for_name(&conn, STORE_PROFILE, "dup").unwrap();
        assert_eq!(open.is_some(), !first.is_terminal(), "{first:?}");
        let second =
            queries::insert_shakedex_listing(&conn, &listing("b", "dup", ListingState::Locking));
        assert_eq!(second.is_ok(), first.is_terminal(), "{i}: {first:?}");
    }
}

#[test]
fn a_lock_outpoint_is_tracked_once() {
    let conn = store_conn();
    let mut a = listing("a", "relist", ListingState::Cancelled);
    a.lock_txid = Some("aa".repeat(32));
    a.lock_vout = Some(0);
    queries::insert_shakedex_listing(&conn, &a).unwrap();
    let mut b = listing("b", "relist", ListingState::Listed);
    b.lock_txid = a.lock_txid.clone();
    b.lock_vout = Some(0);
    assert!(queries::insert_shakedex_listing(&conn, &b).is_err());
    b.lock_vout = Some(1);
    queries::insert_shakedex_listing(&conn, &b).unwrap();
}

#[test]
fn deleting_an_unsent_lock_draft_deletes_its_listing() {
    let conn = store_conn();
    queries::insert_tx_draft(
        &conn,
        "lockdraft",
        STORE_PROFILE,
        "shakedex_lock",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    let mut l = listing("l1", "gone", ListingState::Locking);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "lockdraft").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_none());
}

#[test]
fn deleting_a_draft_keeps_a_listing_past_locking() {
    let conn = store_conn();
    queries::insert_tx_draft(
        &conn,
        "lockdraft",
        STORE_PROFILE,
        "shakedex_lock",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    let mut l = listing("l1", "kept", ListingState::ReadyToFinalize);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "lockdraft").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_some());
}

/// A `dropped` or `failed` draft was broadcast (eviction grace, a timed-out
/// broadcast): its TRANSFER may still be mined, so deleting the draft is no
/// evidence the name never left, and the listing stays.
#[test]
fn deleting_a_broadcast_then_dropped_or_failed_draft_keeps_the_locking_listing() {
    for status in ["dropped", "failed"] {
        let conn = store_conn();
        queries::insert_tx_draft(
            &conn,
            "lockdraft",
            STORE_PROFILE,
            "shakedex_lock",
            "",
            "{}",
            "{}",
        )
        .unwrap();
        conn.execute(
            "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = 'lockdraft'",
            params![status],
        )
        .unwrap();
        let mut l = listing("l1", "maybe", ListingState::Locking);
        l.lock_transfer_draft_id = Some("lockdraft".into());
        queries::insert_shakedex_listing(&conn, &l).unwrap();
        queries::delete_tx_draft(&conn, "lockdraft").unwrap();
        assert!(
            queries::get_shakedex_listing(&conn, "l1")
                .unwrap()
                .is_some(),
            "{status}"
        );
    }
}

#[test]
fn deleting_a_signed_lock_draft_deletes_its_listing() {
    let conn = store_conn();
    queries::insert_tx_draft(
        &conn,
        "lockdraft",
        STORE_PROFILE,
        "shakedex_lock",
        "",
        "{}",
        "{}",
    )
    .unwrap();
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = 'signed' WHERE id = 'lockdraft'",
        [],
    )
    .unwrap();
    let mut l = listing("l1", "unsent", ListingState::Locking);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "lockdraft").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_none());
}

#[test]
fn deleting_another_draft_keeps_a_locking_listing() {
    let conn = store_conn();
    for id in ["lockdraft", "other"] {
        queries::insert_tx_draft(&conn, id, STORE_PROFILE, "shakedex_lock", "", "{}", "{}")
            .unwrap();
    }
    let mut l = listing("l1", "mine", ListingState::Locking);
    l.lock_transfer_draft_id = Some("lockdraft".into());
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    queries::delete_tx_draft(&conn, "other").unwrap();
    assert!(queries::get_shakedex_listing(&conn, "l1")
        .unwrap()
        .is_some());
}

// --- the lock command -------------------------------------------------------

use mockito::{Mock, ServerGuard};
use serde_json::{json, Value};
use tauri::Manager;

use crate::commands::shakedex::{
    build_lock_draft_inner, shakedex_build_lock_draft, ExpiryNotice, LockDraftInput,
};
use crate::noncustodial::actions::DraftPlan;
use crate::noncustodial::address;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::{self, ExtendedPrivKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::shakedex::lock_key::derive_lock_key;
use crate::noncustodial::shakedex::sell::{self, LOCK_ACTION, LOCK_COSTS};
use crate::noncustodial::sync::{COV_REGISTER, COV_REVEAL, COV_TRANSFER};
use crate::noncustodial::types::TxSummary;
use crate::tests::shakedex_cmd_tests::{
    app_with, err_text, mock_blockchain_info, mock_name_info, rpc_ok, seed, seeded, set, with_db,
    PROFILE,
};
use crate::AppState;

type App = tauri::App<tauri::test::MockRuntime>;

const NAME: &str = "dexsale";
const OWNER_TXID: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const NAME_HEIGHT: u32 = 50;
const NAME_VALUE: u64 = 1_000_000;
/// Regtest: renewal 1000 + window 5000.
const RENEWAL: u64 = 1_000;
const REGTEST_END: i64 = 6_000;
/// Far from expiry on regtest (end - tip = 4000 >= 1800).
const QUIET_TIP: i64 = 2_000;

fn net_of(network: &str) -> Network {
    derivation::network_from_profile(network).unwrap()
}

fn master() -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed()).unwrap()
}

/// Our receive address 0/0, where `seeded` puts the funding coin.
fn addr00(net: Network) -> (String, String) {
    let (_sk, pk, addr) = hd::derive_address(net, &seed(), 0, 0, 0).unwrap();
    (
        addr,
        hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap()),
    )
}

fn covenant_json(cov_type: u8, items: &[String]) -> String {
    json!({ "type": cov_type, "action": "", "items": items }).to_string()
}

/// The name's owner coin, `cov_type` (REGISTER unless a test says
/// otherwise), at our address 0/0, and the tracked row pointing at it.
fn seed_owner_coin(conn: &Connection, net: Network, txid: &str, cov_type: u8) {
    let (addr, spk) = addr00(net);
    let nh = hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap());
    let items = [nh.clone(), hex::encode(NAME_HEIGHT.to_le_bytes())];
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex, value_doos,
             covenant_type, covenant_json, spend_class, spent_by_txid)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, 'name_control', NULL)",
        params![
            txid,
            PROFILE,
            addr,
            spk,
            NAME_VALUE as i64,
            i64::from(cov_type),
            covenant_json(cov_type, &items)
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, owner_address, height)
         VALUES (?1, ?2, ?3, 'CLOSED', ?4, 0, ?5, ?6)",
        params![PROFILE, NAME, nh, txid, addr, i64::from(NAME_HEIGHT)],
    )
    .unwrap();
}

fn name_info(renewal: u64, claimed: u64, owner_txid: &str) -> Value {
    json!({
        "info": {
            "name": NAME, "state": "CLOSED", "height": NAME_HEIGHT, "renewal": renewal,
            "renewals": 0, "claimed": claimed, "weak": false, "transfer": 0,
            "owner": { "hash": owner_txid, "index": 0 }, "value": NAME_VALUE
        },
        "start": null
    })
}

fn unlock(app: &App, net: Network) {
    *app.state::<AppState>().signer.lock().unwrap() = Some(SignerSession::unlock(
        PROFILE.into(),
        net,
        master(),
        600_000,
    ));
}

/// A profile of `kind` on `network` owning NAME as a coin of `cov_type`,
/// unlocked, and a node at `tip` answering `info` for the name.
async fn fixture_with(
    network: &str,
    kind: &str,
    tip: i64,
    cov_type: u8,
    info: Value,
) -> (ServerGuard, Vec<Mock>, App) {
    let mut node = mockito::Server::new_async().await;
    let mocks = vec![
        mock_blockchain_info(&mut node, tip, Some(1_700_000_000)).await,
        mock_name_info(&mut node, info).await,
    ];
    let conn = seeded(network, kind, &node.url());
    seed_owner_coin(&conn, net_of(network), OWNER_TXID, cov_type);
    let app = app_with(conn);
    unlock(&app, net_of(network));
    (node, mocks, app)
}

/// A profile of `kind` on `network` owning NAME, unlocked, and a node at `tip`.
async fn lock_fixture(network: &str, kind: &str, tip: i64) -> (ServerGuard, Vec<Mock>, App) {
    fixture_with(
        network,
        kind,
        tip,
        COV_REGISTER,
        name_info(RENEWAL, 0, OWNER_TXID),
    )
    .await
}

async fn build(
    app: &App,
) -> Result<crate::noncustodial::types::TxDraftSummary, crate::error::AppError> {
    shakedex_build_lock_draft(app.state(), NAME.into(), ListingMode::BuyNow, false, None).await
}

fn count(app: &App, table: &str) -> i64 {
    with_db(app, |c| {
        c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
}

fn open_listing(app: &App) -> Option<ShakedexListing> {
    with_db(app, |c| {
        queries::open_shakedex_listing_for_name(c, PROFILE, NAME).unwrap()
    })
}

/// Nothing written: no draft, no listing, no reserved address.
fn assert_nothing_written(app: &App, addresses_before: i64) {
    assert_eq!(count(app, "wallet_tx_drafts"), 0);
    assert_eq!(count(app, "shakedex_listings"), 0);
    assert_eq!(count(app, "derived_addresses"), addresses_before);
}

/// R19 day 0 and R18: the draft spends our owner coin into a TRANSFER that
/// stays at our address and commits to SHA3-256 of the lock script of the
/// key derived from the seed for this name and account; the listing is
/// Locking with that public key and this draft.
#[tokio::test]
async fn lock_draft_commits_to_the_derived_lock_address() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.expect("lock builds");
    assert_eq!(draft.action, LOCK_ACTION);

    let key = derive_lock_key(&master(), Network::Regtest, 0, NAME).unwrap();
    let row = with_db(&app, |c| {
        queries::get_tx_draft(c, &draft.id).unwrap().unwrap()
    });
    let plan: DraftPlan = serde_json::from_str(&row.signing_inputs_json).unwrap();
    assert_eq!(
        (plan.inputs[0].txid.as_str(), plan.inputs[0].vout),
        (OWNER_TXID, 0)
    );
    let out = &plan.outputs[0];
    assert_eq!(out.covenant_type, COV_TRANSFER);
    assert_eq!(
        out.address,
        addr00(Network::Regtest).0,
        "stays home until finalized"
    );
    assert_eq!(out.value, NAME_VALUE);
    let nh = hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap());
    assert_eq!(
        out.covenant_items_hex,
        vec![
            nh,
            hex::encode(NAME_HEIGHT.to_le_bytes()),
            "00".into(),
            hex::encode(key.program)
        ]
    );
    assert_eq!(draft.summary["recipientAddress"], key.address);
    assert_eq!(draft.summary["name"], NAME);

    let l = open_listing(&app).expect("a listing");
    assert_eq!(l.state, ListingState::Locking);
    assert_eq!(l.mode, ListingMode::BuyNow);
    assert_eq!(l.lock_pubkey_hex, hex::encode(key.pubkey));
    assert_eq!(l.lock_transfer_draft_id.as_deref(), Some(draft.id.as_str()));
    assert_eq!(
        l.lock_transfer_txid,
        row.summary_json
            .parse::<Value>()
            .ok()
            .and_then(|s| s["txid"].as_str().map(str::to_owned))
    );
    assert!(l.lock_transfer_txid.is_some());
    assert!(!l.publish);
    assert_eq!(
        (l.lock_txid, l.lock_vout),
        (None, None),
        "set by Finalize & sign (T3)"
    );
}

/// R18: a key that fails its self-check never reaches a draft, a listing or
/// a reserved address.
#[tokio::test]
async fn refuses_lock_on_self_check_failure() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let ctx = crate::commands::draft_ctx::load_ctx(&app.state()).unwrap();
    let mut key = derive_lock_key(&master(), Network::Regtest, 0, NAME).unwrap();
    let other = derive_lock_key(&master(), Network::Regtest, 0, "othername").unwrap();
    key.address = other.address.clone();
    let addresses_before = count(&app, "derived_addresses");
    let err = with_db(&app, |c| {
        let owner = queries::get_name_coin(c, PROFILE, NAME).unwrap().unwrap();
        build_lock_draft_inner(
            c,
            &LockDraftInput {
                ctx: &ctx,
                key: &key,
                name: NAME,
                mode: ListingMode::BuyNow,
                publish: false,
                owner: &owner,
                name_height: NAME_HEIGHT,
                notice: ExpiryNotice::Ok,
                rate: 10,
            },
        )
        .unwrap_err()
    });
    assert!(err_text(err).contains("self-check"));
    assert_nothing_written(&app, addresses_before);
}

/// R6/R29: no lock on a node that cannot send.
#[tokio::test]
async fn lock_refused_without_write_capability() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    with_db(&app, |c| set(c, "chain_source", "explorer"));
    let err = build(&app).await.unwrap_err();
    assert!(err_text(err).contains(crate::commands::shakedex::NEEDS_SENDING_NODE));
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R16/R29: Ledger, watch-only and extended-private-key profiles cannot
/// lock, with the sentence the UI shows.
#[tokio::test]
async fn lock_refused_for_ledger_and_watch_only() {
    let sentence = crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY;
    for kind in ["ledger_hardware", "xpriv_hot", "watch_only_xpub"] {
        let (_n, _m, app) = lock_fixture("regtest", kind, QUIET_TIP).await;
        if kind == "watch_only_xpub" {
            with_db(&app, |c| {
                c.execute(
                    "UPDATE wallet_profiles SET watch_only = 1 WHERE id = ?1",
                    params![PROFILE],
                )
                .unwrap();
            });
        }
        let err = err_text(build(&app).await.unwrap_err());
        assert!(err.contains(sentence), "{kind}: {err}");
        assert_eq!(count(&app, "shakedex_listings"), 0, "{kind}");
    }
}

/// R15/R29: a new listing on mainnet needs the experimental flag.
#[tokio::test]
async fn mainnet_lock_needs_experimental_flag() {
    // Mainnet: renewal 1000 + 105 120; tip 2000 is far from expiry.
    let (_node, _m, app) = lock_fixture("mainnet", "mnemonic_hot", 2_000).await;
    let err = build(&app).await.unwrap_err();
    assert!(err_text(err).contains(crate::noncustodial::shakedex::MAINNET_SELLING_EXPERIMENTAL));
    assert_eq!(count(&app, "shakedex_listings"), 0);
    with_db(&app, |c| set(c, "shakedex_experimental", "true"));
    let draft = build(&app).await.expect("flag set: the lock builds");
    assert_eq!(draft.action, LOCK_ACTION);
}

#[tokio::test]
async fn regtest_lock_ignores_flag() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    with_db(&app, |c| set(c, "shakedex_experimental", "false"));
    let draft = build(&app).await.expect("regtest needs no flag");
    assert_eq!(draft.action, LOCK_ACTION);
}

/// R23/R29: off mainnet a listing is shared as a file only.
#[tokio::test]
async fn regtest_lock_cannot_publish_to_the_market() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let err = shakedex_build_lock_draft(app.state(), NAME.into(), ListingMode::BuyNow, true, None)
        .await
        .unwrap_err();
    assert!(err_text(err).contains(crate::commands::shakedex::MARKET_MAINNET_ONLY));
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R18: Lock derives the lock key, so it needs the unlocked signer: not a
/// locked session, not another profile's, not none.
#[tokio::test]
async fn lock_needs_the_unlocked_signer() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    app.state::<AppState>()
        .signer
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lock();
    assert!(matches!(
        build(&app).await.unwrap_err(),
        crate::error::AppError::WalletLocked
    ));
    *app.state::<AppState>().signer.lock().unwrap() = Some(SignerSession::unlock(
        "another".into(),
        Network::Regtest,
        master(),
        600_000,
    ));
    assert!(err_text(build(&app).await.unwrap_err()).contains("different wallet profile"));
    *app.state::<AppState>().signer.lock().unwrap() = None;
    assert!(matches!(
        build(&app).await.unwrap_err(),
        crate::error::AppError::WalletLocked
    ));
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R31 at Lock, regtest: expiry 6000; refused at tip 5979 (6000 <= 5979 +
/// 1 + 10 + 10), built at tip 5978, with nothing written by the refusal.
#[tokio::test]
async fn lock_refused_when_the_name_would_expire_during_the_lockup() {
    let (mut node, mut mocks, app) =
        lock_fixture("regtest", "mnemonic_hot", REGTEST_END - 21).await;
    let addresses_before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(
        err.contains("before its transfer into the lock could be finalized"),
        "R31 refusal: {err}"
    );
    assert!(err.contains(&REGTEST_END.to_string()), "{err}");
    assert_nothing_written(&app, addresses_before);

    mocks.remove(0).remove_async().await;
    mocks.push(mock_blockchain_info(&mut node, REGTEST_END - 22, Some(1_700_000_000)).await);
    build(&app)
        .await
        .expect("one block earlier the lock builds");
}

/// R31's warning: below 180 R9 days (1800 regtest blocks) from the tip the
/// draft carries it; at 1800 it does not. LOCK_COSTS is always there.
#[tokio::test]
async fn lock_warns_below_six_months() {
    let (mut node, mut mocks, app) =
        lock_fixture("regtest", "mnemonic_hot", REGTEST_END - 1_799).await;
    let draft = build(&app).await.expect("builds with the warning");
    let warnings: Vec<String> = serde_json::from_value(draft.summary["warnings"].clone()).unwrap();
    assert_eq!(
        warnings,
        vec![LOCK_COSTS.to_string(), sell::near_expiry_warning(1_799)]
    );
    // The unsent draft goes, and its listing with it; the owner coin is free.
    with_db(&app, |c| queries::delete_tx_draft(c, &draft.id).unwrap());
    assert_eq!(count(&app, "shakedex_listings"), 0);

    mocks.remove(0).remove_async().await;
    mocks.push(mock_blockchain_info(&mut node, REGTEST_END - 1_800, Some(1_700_000_000)).await);
    let draft = build(&app).await.expect("builds without it");
    let warnings: Vec<String> = serde_json::from_value(draft.summary["warnings"].clone()).unwrap();
    assert_eq!(warnings, vec![LOCK_COSTS.to_string()]);
}

/// R27 and R31: the sentences say what the spec says; the warning carries
/// the block count it is given.
#[test]
fn lock_sentences_say_what_locking_costs() {
    for phrase in ["cannot be changed", "cannot be renewed", "renews the name"] {
        assert!(LOCK_COSTS.contains(phrase), "{phrase}");
    }
    let w = sell::near_expiry_warning(1_234);
    assert!(w.contains("expires in 1234 blocks"), "{w}");
    assert!(w.contains("renews the name"), "{w}");
}

/// The lock draft's summary reads back as the plain `TxSummary` the secure
/// window and the frontend's `TxDraftSummary.summary` expect, plus `name`.
#[tokio::test]
async fn lock_summary_reads_as_a_tx_summary() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.unwrap();
    let row = with_db(&app, |c| {
        queries::get_tx_draft(c, &draft.id).unwrap().unwrap()
    });
    let s: TxSummary = serde_json::from_str(&row.summary_json).unwrap();
    assert_eq!(s.action, LOCK_ACTION);
    assert_eq!(s.warnings, vec![LOCK_COSTS.to_string()]);
    assert_eq!(
        s.send_total_doos, NAME_VALUE as i64,
        "the name's own output"
    );
    assert!(s.fee_doos > 0);
    assert!(s.txid.is_some());
    assert_eq!(open_listing(&app).unwrap().lock_transfer_txid, s.txid);
    assert_eq!(draft.summary["name"], NAME);
}

/// R21: the listing pays a fresh receive address and cancels to another,
/// both marked used when the listing is created, so the next allocation
/// returns a third; the cancel address's receive-branch index is stored.
#[tokio::test]
async fn lock_reserves_payment_and_cancel_addresses() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    build(&app).await.expect("lock builds");
    let l = open_listing(&app).unwrap();
    let pay = l.payment_address.clone().expect("payment address");
    let cancel = l.cancel_address.clone().expect("cancel address");
    assert_ne!(pay, cancel);
    with_db(&app, |c| {
        let rows = queries::list_receive_addresses(c, PROFILE, 0).unwrap();
        for a in [&pay, &cancel] {
            assert!(
                rows.iter().any(|r| &r.address == a && r.used),
                "{a} reserved and used"
            );
        }
        let cancel_row = rows.iter().find(|r| r.address == cancel).unwrap();
        assert_eq!(l.cancel_child_index, Some(i64::from(cancel_row.index)));
        assert_ne!(
            cancel_row.index, 0,
            "not the address the owner coin sits at"
        );
        let next = derivation::reserve_receive_address(c, PROFILE).unwrap();
        assert!(next.address != pay && next.address != cancel);
    });
}

/// hsd lets a TRANSFER coin go only to UPDATE, RENEW, FINALIZE or REVOKE:
/// a name already in a transfer is not locked.
#[tokio::test]
async fn lock_refused_while_a_transfer_is_pending() {
    let (_node, _m, app) = fixture_with(
        "regtest",
        "mnemonic_hot",
        QUIET_TIP,
        COV_TRANSFER,
        name_info(RENEWAL, 0, OWNER_TXID),
    )
    .await;
    let before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(
        err.contains("a transfer of 'dexsale' is pending: cancel or finalize it"),
        "{err}"
    );
    assert_nothing_written(&app, before);
}

/// Only an owner coin hsd lets go to a TRANSFER (REGISTER, UPDATE, RENEW,
/// FINALIZE) is locked; a name the wallet does not hold is not either.
#[tokio::test]
async fn lock_refused_for_a_name_not_ours_or_not_registered() {
    let (_node, _m, app) = fixture_with(
        "regtest",
        "mnemonic_hot",
        QUIET_TIP,
        COV_REVEAL,
        name_info(RENEWAL, 0, OWNER_TXID),
    )
    .await;
    let before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("is not registered to this wallet"), "{err}");
    assert_nothing_written(&app, before);

    with_db(&app, |c| {
        c.execute("DELETE FROM tracked_name_states", []).unwrap();
    });
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("wallet does not hold"), "{err}");
    assert_nothing_written(&app, before);
}

#[tokio::test]
async fn second_lock_of_a_name_is_refused() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    build(&app).await.expect("first lock builds");
    let before = count(&app, "derived_addresses");
    assert!(err_text(build(&app).await.unwrap_err()).contains("already locked for sale"));
    assert_eq!(count(&app, "shakedex_listings"), 1);
    assert_eq!(count(&app, "wallet_tx_drafts"), 1);
    assert_eq!(count(&app, "derived_addresses"), before);
}

/// Fail closed: R31 reads the tip, the renewal height and the claimed count
/// from the node; a reply without one of them refuses the lock ("could not
/// check"), and a name with no state is refused as such.
#[tokio::test]
async fn lock_refused_when_the_node_cannot_say_when_the_name_expires() {
    let mut no_renewal = name_info(RENEWAL, 0, OWNER_TXID);
    no_renewal["info"]
        .as_object_mut()
        .unwrap()
        .remove("renewal");
    let mut no_claimed = name_info(RENEWAL, 0, OWNER_TXID);
    no_claimed["info"]
        .as_object_mut()
        .unwrap()
        .remove("claimed");
    for (what, info) in [("renewal", no_renewal), ("claimed", no_claimed)] {
        let (_n, _m, app) =
            fixture_with("regtest", "mnemonic_hot", QUIET_TIP, COV_REGISTER, info).await;
        let before = count(&app, "derived_addresses");
        let err = err_text(build(&app).await.unwrap_err());
        assert!(err.contains("could not check"), "{what}: {err}");
        assert_nothing_written(&app, before);
    }

    let (_n, _m, app) = fixture_with(
        "regtest",
        "mnemonic_hot",
        QUIET_TIP,
        COV_REGISTER,
        json!({ "info": null, "start": null }),
    )
    .await;
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("no on-chain state or has expired"), "{err}");
    assert_eq!(count(&app, "shakedex_listings"), 0);

    // A getblockchaininfo reply without the tip.
    let mut node = mockito::Server::new_async().await;
    let _b = node
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            json!({ "method": "getblockchaininfo" }),
        ))
        .with_header("content-type", "application/json")
        .with_body(rpc_ok(json!({ "chain": "regtest", "headers": QUIET_TIP })))
        .create_async()
        .await;
    let _n = mock_name_info(&mut node, name_info(RENEWAL, 0, OWNER_TXID)).await;
    let conn = seeded("regtest", "mnemonic_hot", &node.url());
    seed_owner_coin(&conn, Network::Regtest, OWNER_TXID, COV_REGISTER);
    let app = app_with(conn);
    unlock(&app, Network::Regtest);
    let before = count(&app, "derived_addresses");
    let err = err_text(build(&app).await.unwrap_err());
    assert!(err.contains("could not check"), "no tip: {err}");
    assert_nothing_written(&app, before);
}

/// R27 in the secure window: the lock draft's confirmation carries what
/// locking costs as a Warning row.
#[tokio::test]
async fn lock_confirmation_shows_what_locking_costs() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let draft = build(&app).await.unwrap();
    let row = with_db(&app, |c| {
        queries::get_tx_draft(c, &draft.id).unwrap().unwrap()
    });
    let details = crate::commands::tx::confirm_details_for_draft(&row).unwrap();
    assert!(
        details["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["label"] == "Warning" && r["value"] == LOCK_COSTS),
        "{details}"
    );
}

// --- the abort ------------------------------------------------------------

use crate::commands::names::build_cancel_draft;
use crate::noncustodial::rpc::NodeCoin;
use crate::shakedex_jobs::refresh_listing_aborts_with_client;
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};

/// Lock NAME, then make the wallet's records what a sync leaves after the
/// lock TRANSFER is mined: our TRANSFER coin at 0/0 is the owner coin.
/// Returns the lock txid.
async fn locked_on_chain(app: &App) -> String {
    let draft = build(app).await.expect("lock builds");
    let txid = draft.summary["txid"].as_str().unwrap().to_string();
    with_db(app, |c| {
        queries::update_tx_draft_status(c, &draft.id, "broadcasted", None, Some(&txid)).unwrap();
        queries::update_tx_draft_confirmation(c, &draft.id, QUIET_TIP - 100, None).unwrap();
        // The funding coin the lock reserved stands in for its change, so
        // the cancel has a coin to pay its fee from.
        queries::release_reserved_utxos_for_draft(c, &draft.id).unwrap();
        c.execute(
            "UPDATE tracked_utxos SET spent_by_txid = ?1 WHERE txid = ?2",
            params![txid, OWNER_TXID],
        )
        .unwrap();
        c.execute(
            "DELETE FROM tracked_name_states WHERE name = ?1",
            params![NAME],
        )
        .unwrap();
        seed_owner_coin(c, Network::Regtest, &txid, COV_TRANSFER);
    });
    txid
}

/// A coin as hsd's `GET /coin/:hash/:index` sends it: `height` is the block
/// it was mined in, -1 in the mempool, absent when a test leaves it out.
fn node_coin(txid: &str, vout: u32, height: Option<i64>) -> NodeCoin {
    let mut coin = json!({ "hash": txid, "index": vout, "value": NAME_VALUE });
    if let Some(h) = height {
        coin["height"] = h.into();
    }
    serde_json::from_value(coin).unwrap()
}

/// A node whose `GET /coin` knows the cancel's UPDATE (output 0) at
/// `cancel_height` and the lock TRANSFER coin when `transfer_unspent`;
/// `None` for a coin hsd answers 404 for (spent, or never existed).
fn chain(
    cancel_txid: &str,
    cancel_height: Option<Option<i64>>,
    lock_txid: &str,
    transfer_unspent: bool,
) -> MockNodeRpc {
    let (cancel_txid, lock_txid) = (cancel_txid.to_string(), lock_txid.to_string());
    MockNodeRpc::new().with_get_coin(move |txid, vout| {
        Ok(if txid == cancel_txid && vout == 0 {
            cancel_height.map(|h| node_coin(txid, vout, h))
        } else if txid == lock_txid && vout == 0 && transfer_unspent {
            Some(node_coin(txid, vout, Some(QUIET_TIP - 100)))
        } else {
            None
        })
    })
}

/// Run the abort job on the app's database, as the sync step does. The
/// connection is taken out of the app for the call, so no lock is held
/// across an await.
async fn run_abort_job(app: &App, rpc: &MockNodeRpc) {
    let conn = std::mem::replace(
        &mut *app.state::<AppState>().db.lock().unwrap(),
        Connection::open_in_memory().unwrap(),
    );
    let res = refresh_listing_aborts_with_client(&conn, rpc, PROFILE).await;
    *app.state::<AppState>().db.lock().unwrap() = conn;
    res.expect("abort job runs");
    assert_eq!(
        rpc.count_matching(|c| matches!(c, RpcCall::SendRawTransaction(_))),
        0,
        "the job sends nothing (it runs in the daemon too)"
    );
}

fn listing_state(app: &App, id: &str) -> ListingState {
    with_db(app, |c| {
        queries::get_shakedex_listing(c, id).unwrap().unwrap().state
    })
}

/// R19: Cancel transfer on a name still Locking is the abort. Building or
/// sending it changes nothing (hsd answers a refused send with its txid);
/// once its UPDATE is a mined coin the listing is Aborted, and a reorg that
/// takes it out (the UPDATE back in the mempool, or the lock TRANSFER
/// unspent again) makes the listing Locking again.
#[tokio::test]
async fn cancel_transfer_aborts_the_listing() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    assert_eq!(
        open_listing(&app).unwrap().lock_transfer_txid.as_deref(),
        Some(lock_txid.as_str())
    );

    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .expect("cancel builds");
    let l = open_listing(&app).unwrap();
    assert_eq!(l.abort_draft_id.as_deref(), Some(cancel.id.as_str()));
    assert_eq!(l.state, ListingState::Locking, "built, not sent");
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    assert_eq!(
        l.abort_txid.as_deref(),
        Some(ctxid.as_str()),
        "the cancel's txid"
    );
    with_db(&app, |c| {
        queries::update_tx_draft_status(c, &cancel.id, "broadcasted", None, Some(&ctxid)).unwrap();
    });

    // Sent, nothing on chain yet: the lock TRANSFER is still unspent.
    run_abort_job(&app, &chain(&ctxid, None, &lock_txid, true)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "not taken"
    );
    run_abort_job(&app, &chain(&ctxid, Some(Some(-1)), &lock_txid, false)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "in the mempool"
    );

    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(listing_state(&app, &l.id), ListingState::Aborted);
    assert!(open_listing(&app).is_none(), "the name is free again");

    run_abort_job(&app, &chain(&ctxid, Some(Some(-1)), &lock_txid, false)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "a reorg took the cancel back to the mempool"
    );

    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Aborted,
        "mined again"
    );
    run_abort_job(&app, &chain(&ctxid, None, &lock_txid, true)).await;
    assert_eq!(
        listing_state(&app, &l.id),
        ListingState::Locking,
        "a reorg took the cancel out and the lock TRANSFER is unspent again"
    );
}

/// A final verdict needs the node's word: a coin without its height, the
/// cancel's UPDATE gone with the lock TRANSFER spent (mined and spent since,
/// or another spend), or no answer at all leave the listing as it is.
#[tokio::test]
async fn abort_needs_the_nodes_word() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    let id = open_listing(&app).unwrap().id;

    for state in [ListingState::Locking, ListingState::Aborted] {
        with_db(&app, |c| {
            c.execute(
                "UPDATE shakedex_listings SET state = ?1 WHERE id = ?2",
                params![state, id],
            )
            .unwrap();
        });
        for (what, rpc) in [
            ("no height", chain(&ctxid, Some(None), &lock_txid, true)),
            ("both spent", chain(&ctxid, None, &lock_txid, false)),
            ("no answer", MockNodeRpc::new()),
        ] {
            run_abort_job(&app, &rpc).await;
            assert_eq!(listing_state(&app, &id), state, "{what}");
        }
    }
}

/// Only a cancel of this listing's own lock TRANSFER aborts it.
#[tokio::test]
async fn cancel_of_another_transfer_leaves_the_listing_alone() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock = build(&app).await.expect("lock builds"); // never sent
                                                        // A TRANSFER of the name that is not the lock's (another device).
    with_db(&app, |c| {
        queries::release_reserved_utxos_for_draft(c, &lock.id).unwrap();
        c.execute(
            "UPDATE tracked_utxos SET spent_by_txid = 'x' WHERE txid = ?1",
            params![OWNER_TXID],
        )
        .unwrap();
        c.execute(
            "DELETE FROM tracked_name_states WHERE name = ?1",
            params![NAME],
        )
        .unwrap();
        seed_owner_coin(c, Network::Regtest, &"77".repeat(32), COV_TRANSFER);
    });
    build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .expect("cancel builds");
    assert_eq!(open_listing(&app).unwrap().abort_draft_id, None);
}

/// Deleting a Cancel transfer that was never sent (`draft`, `signed`) clears
/// its link: it aborts nothing. A `dropped` or `failed` one was broadcast and
/// may still be mined, so its listing keeps the link.
#[tokio::test]
async fn deleting_an_unsent_cancel_unlinks_it() {
    for (status, keeps_link) in [
        ("draft", false),
        ("signed", false),
        ("dropped", true),
        ("failed", true),
    ] {
        let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
        locked_on_chain(&app).await;
        let cancel = build_cancel_draft(app.state(), NAME.into(), None)
            .await
            .unwrap();
        assert_eq!(
            open_listing(&app).unwrap().abort_draft_id.as_deref(),
            Some(cancel.id.as_str())
        );
        with_db(&app, |c| {
            if status != "draft" {
                queries::update_tx_draft_status(c, &cancel.id, status, None, None).unwrap();
            }
            queries::delete_tx_draft(c, &cancel.id).unwrap();
        });
        let after = open_listing(&app).unwrap();
        let expected = keeps_link.then(|| cancel.id.clone());
        assert_eq!(after.abort_draft_id, expected, "{status}");
        let expected_txid =
            keeps_link.then(|| cancel.summary["txid"].as_str().unwrap().to_string());
        assert_eq!(after.abort_txid, expected_txid, "{status}");
    }
}

/// A cancel that was broadcast, then dropped and its draft deleted, may
/// still be mined (another node held it): the listing keeps the cancel's
/// txid, so the job still finds the mined UPDATE and the listing is Aborted.
#[tokio::test]
async fn deleted_dropped_cancel_mined_later_still_aborts() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    let id = open_listing(&app).unwrap().id;
    with_db(&app, |c| {
        queries::update_tx_draft_status(c, &cancel.id, "dropped", None, Some(&ctxid)).unwrap();
        queries::delete_tx_draft(c, &cancel.id).unwrap();
        assert!(queries::get_tx_draft(c, &cancel.id).unwrap().is_none());
    });

    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(listing_state(&app, &id), ListingState::Aborted);
}

/// A reorg that takes the abort out after the name was locked again: the new
/// listing is the one open (`idx_shakedex_listings_open_name` allows one per
/// name), so the old listing stays Aborted and the job carries on. The new
/// lock TRANSFER spends the cancel's UPDATE, so it is mined only if the
/// cancel is.
#[tokio::test]
async fn reorged_abort_leaves_a_newer_listing_open() {
    let (_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
    let lock_txid = locked_on_chain(&app).await;
    let cancel = build_cancel_draft(app.state(), NAME.into(), None)
        .await
        .unwrap();
    let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
    let old = open_listing(&app).unwrap().id;
    run_abort_job(
        &app,
        &chain(&ctxid, Some(Some(QUIET_TIP)), &lock_txid, false),
    )
    .await;
    assert_eq!(listing_state(&app, &old), ListingState::Aborted);

    let mut newer = listing("newer", NAME, ListingState::Locking);
    newer.wallet_profile_id = PROFILE.into();
    with_db(&app, |c| {
        queries::insert_shakedex_listing(c, &newer).unwrap()
    });

    run_abort_job(&app, &chain(&ctxid, Some(Some(-1)), &lock_txid, false)).await;
    assert_eq!(listing_state(&app, &old), ListingState::Aborted);
    assert_eq!(open_listing(&app).unwrap().id, "newer");
    // Refused by the guard, not by the index: no error for the job to log.
    with_db(&app, |c| {
        assert_eq!(queries::unabort_shakedex_listing(c, &old).unwrap(), 0);
    });
}

/// The abort is a sync step: both the app's sync and the daemon's run it
/// against an authoritative node, and neither sends anything (SECURITY.md,
/// "Daemon is read-only").
#[tokio::test]
async fn sync_aborts_the_listing_in_the_app_and_the_daemon() {
    use crate::commands::sync::{run_sync_steps, SyncCaller, SyncStatus};

    for caller in [SyncCaller::Daemon, SyncCaller::App] {
        let (_lock_node, _m, app) = lock_fixture("regtest", "mnemonic_hot", QUIET_TIP).await;
        locked_on_chain(&app).await;
        let cancel = build_cancel_draft(app.state(), NAME.into(), None)
            .await
            .unwrap();
        let ctxid = cancel.summary["txid"].as_str().unwrap().to_string();
        let id = open_listing(&app).unwrap().id;

        let mut node = mockito::Server::new_async().await;
        let path = std::env::temp_dir().join(format!(
            "namehold_sell_abort_{}_{caller:?}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let db_path = path.to_str().unwrap().to_string();
        with_db(&app, |c| {
            queries::set_setting(c, "node_rpc_url", &node.url()).unwrap();
            c.execute("VACUUM INTO ?1", params![db_path]).unwrap();
        });
        let send = node
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("sendrawtransaction".into()))
            .expect(0)
            .create_async()
            .await;
        let _info = node
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(
                json!({ "method": "getblockchaininfo" }),
            ))
            .with_header("content-type", "application/json")
            .with_body(rpc_ok(json!({
                "chain": "regtest", "blocks": QUIET_TIP, "headers": QUIET_TIP,
                "verificationprogress": 1.0, "mediantime": 1_700_000_000u64
            })))
            .create_async()
            .await;
        let _mined = node
            .mock("GET", format!("/coin/{ctxid}/0").as_str())
            .with_header("content-type", "application/json")
            .with_body(
                json!({ "hash": ctxid, "index": 0, "value": NAME_VALUE, "height": QUIET_TIP })
                    .to_string(),
            )
            .create_async()
            .await;

        match caller {
            SyncCaller::Daemon => crate::daemon::sync_profile(&db_path, PROFILE).await,
            SyncCaller::App => {
                let status = std::sync::Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
                run_sync_steps(&status, &db_path, PROFILE, SyncCaller::App).await;
            }
        }

        send.assert_async().await;
        let conn = Connection::open(&path).unwrap();
        let got = queries::get_shakedex_listing(&conn, &id).unwrap().unwrap();
        assert_eq!(got.state, ListingState::Aborted, "{caller:?}");
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
