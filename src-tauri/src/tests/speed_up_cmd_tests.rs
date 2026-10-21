//! Speed up commands against a migrated database and a mocked node: the
//! refusals the database and the node decide (R1-R4), the stored child and
//! its reservation (R7), the target's source (R9), the purchase hold's one
//! exception (R12), the Ledger guard (R13), the re-check before broadcast
//! (R14), the death wording (R15) and the confirmation rows (R11).

use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::commands::speed_up::{
    build_with, preview_with, recheck_before_broadcast, speed_up_check_with, PURCHASE_MOVED_ON,
};
use crate::commands::tx::{
    confirm_details_for_draft, refuse_unsupported_ledger_plan, spent_elsewhere_note,
    SPENT_ELSEWHERE,
};
use crate::db::{self, queries};
use crate::error::AppError;
use crate::noncustodial::actions::DraftPlan;
use crate::noncustodial::address;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{BlockchainInfo, ChainSource};
use crate::noncustodial::send::load_spendable_coins;
use crate::noncustodial::speed_up::{self as su, SpeedUpSummary, TargetSource, SPEED_UP_ACTION};
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};

const MNEMONIC: &str = "april coyote civil finger crane uncle situate moon choice wrong \
                        goose client purse deer funny hobby shrug give anxiety truly rack \
                        stand salad coach";
const PROFILE: &str = "sp1";
const PARENT: &str = "parent-draft";
const PARENT_TXID: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const OTHER_TXID: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const NOW: i64 = 1_761_000_000;
const CHANGE_VALUE: i64 = 1_000_000;

fn seed() -> [u8; 64] {
    hd::seed_from_mnemonic(MNEMONIC, "").unwrap()
}

fn account_xpub() -> String {
    let path = hd::bip44_path(Network::Regtest, 0, 0, 0);
    let master = ExtendedPrivKey::from_seed(&seed()).unwrap();
    let account = master.derive_path(&path[..3]).unwrap();
    ExtendedPubKey::from_priv(&account).to_base58check(Network::Regtest)
}

/// Address, script and pubkey hex of leaf `branch/index`.
fn leaf(branch: u32, index: u32) -> (String, String, String) {
    let (_sk, pk, addr) = hd::derive_address(Network::Regtest, &seed(), 0, branch, index).unwrap();
    let spk = hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap());
    (addr, spk, hex::encode(pk))
}

/// The summary `build_send_hns_draft` stores, after signing
/// (`commands::tx::sign_via_hot_session`).
fn send_summary(change: i64) -> String {
    serde_json::json!({
        "action": "send_hns",
        "sendTotalDoos": 100_000,
        "feeDoos": 140,
        "changeDoos": change,
        "inputTotalDoos": 100_140 + change,
        "numInputs": 1,
        "recipientAddress": leaf(0, 1).0,
        "txid": PARENT_TXID,
        "warnings": []
    })
    .to_string()
}

/// A regtest hot profile with leaves 0/0, 0/1 and change 1/0, and a plain
/// send `PARENT` broadcast as `PARENT_TXID` whose change, output 1 at 1/0,
/// is tracked unspent.
fn seeded() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Speed",
        "mnemonic_hot",
        "regtest",
        &account_xpub(),
        0,
        false,
    )
    .unwrap();
    for (branch, index) in [(0, 0), (0, 1), (1, 0)] {
        let (addr, spk, pk) = leaf(branch, index);
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6)",
            params![PROFILE, branch, index, addr, spk, pk],
        )
        .unwrap();
    }
    let params_json = serde_json::json!({
        "to_address": leaf(0, 1).0, "amount_doos": 100_000, "change_address": leaf(1, 0).0,
        "rate_per_byte": 1, "account": 0, "network": "regtest", "max": false
    })
    .to_string();
    queries::insert_tx_draft(
        &conn,
        PARENT,
        PROFILE,
        "send_hns",
        "",
        &params_json,
        &send_summary(CHANGE_VALUE),
    )
    .unwrap();
    queries::update_tx_draft_status(
        &conn,
        PARENT,
        queries::DraftStatus::Broadcasted,
        None,
        Some(PARENT_TXID),
    )
    .unwrap();
    track(&conn, PARENT_TXID, 1, 1, 0, CHANGE_VALUE);
    conn
}

fn track(conn: &Connection, txid: &str, vout: u32, branch: u32, index: u32, value: i64) {
    let (addr, spk, _) = leaf(branch, index);
    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, covenant_type, spend_class, spent_by_txid, height)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 'liquid_hns', NULL, -1)",
        params![txid, vout, PROFILE, addr, spk, value],
    )
    .unwrap();
}

/// hsd 8.0.0's `getmempoolentry` reply (`lib/node/rpc.js` `entryToJSON`):
/// a 1-in-2-out send of 140 vbytes at 1 doo/vB that entered an hour ago.
fn entry() -> serde_json::Value {
    entry_at(NOW - 3600, 140)
}

fn entry_at(time: i64, fee: u64) -> serde_json::Value {
    serde_json::json!({
        "size": 140,
        "fee": fee as f64 / 1_000_000.0,
        "modifiedfee": 0,
        "time": time,
        "height": 120,
        "startingpriority": 0,
        "currentpriority": 0,
        "descendantcount": 0,
        "descendantsize": 140,
        "descendantfees": fee,
        "ancestorcount": 0,
        "ancestorsize": 0,
        "ancestorfees": 0,
        "depends": []
    })
}

fn regtest_tip() -> BlockchainInfo {
    BlockchainInfo {
        blocks: 120,
        chain: Some("regtest".into()),
        ..Default::default()
    }
}

/// A local regtest node holding the parent.
fn node() -> MockNodeRpc {
    MockNodeRpc::new()
        .with_blockchain_info(regtest_tip())
        .with_mempool_entry(entry())
}

async fn reason(m: &Mutex<Connection>, rpc: &MockNodeRpc) -> Option<String> {
    speed_up_check_with(m, rpc, PARENT, NOW).await.reason
}

async fn check_reason(conn: Connection, rpc: &MockNodeRpc) -> (Mutex<Connection>, Option<String>) {
    let m = Mutex::new(conn);
    let r = speed_up_check_with(&m, rpc, PARENT, NOW).await.reason;
    (m, r)
}

fn refusal(e: AppError) -> String {
    match e {
        AppError::InvalidInput(m) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// --- R1 ------------------------------------------------------------------------

#[tokio::test]
async fn a_sent_unconfirmed_parent_in_the_mempool_is_offered() {
    let rpc = node();
    let m = Mutex::new(seeded());
    let check = speed_up_check_with(&m, &rpc, PARENT, NOW).await;
    assert!(check.available, "{check:?}");
    assert_eq!(check.expires_at, Some(NOW - 3600 + su::MEMPOOL_EXPIRY_SECS));
    assert!(rpc
        .calls()
        .contains(&RpcCall::MempoolEntry(PARENT_TXID.into())));
}

#[tokio::test]
async fn only_a_broadcasted_unconfirmed_draft_is_offered() {
    for status in [
        "draft",
        "signed",
        "broadcast_pending",
        "confirmed",
        "dropped",
        "failed",
    ] {
        let conn = seeded();
        conn.execute(
            "UPDATE wallet_tx_drafts SET status = ?2 WHERE id = ?1",
            params![PARENT, status],
        )
        .unwrap();
        let (_, r) = check_reason(conn, &node()).await;
        assert_eq!(r.as_deref(), Some(su::NOT_SENT_UNCONFIRMED), "{status}");
    }
    // Found mined by the confirmation poll: a height, whatever the status.
    let conn = seeded();
    conn.execute(
        "UPDATE wallet_tx_drafts SET confirmation_height = 121 WHERE id = ?1",
        params![PARENT],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::NOT_SENT_UNCONFIRMED));
}

#[tokio::test]
async fn a_parent_the_node_does_not_hold_is_refused() {
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(regtest_tip())
        .with_mempool_entry_not_in_mempool();
    let (_, r) = check_reason(seeded(), &rpc).await;
    assert_eq!(r.as_deref(), Some(su::NOT_IN_MEMPOOL));
}

#[tokio::test]
async fn a_mempool_reply_that_is_not_hsds_is_no_verdict() {
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(regtest_tip())
        .with_mempool_entry(serde_json::json!({ "size": 140 }));
    let (_, r) = check_reason(seeded(), &rpc).await;
    assert!(r.unwrap().contains("not hsd's answer"));
}

#[tokio::test]
async fn spv_and_explorer_modes_are_refused_before_asking_the_node() {
    for source in [ChainSource::SpvNode, ChainSource::Explorer] {
        let rpc = node().with_source(source);
        let (_, r) = check_reason(seeded(), &rpc).await;
        assert_eq!(r.as_deref(), Some(su::NEEDS_NODE), "{source:?}");
        assert!(!rpc
            .calls()
            .iter()
            .any(|c| matches!(c, RpcCall::MempoolEntry(_))));
    }
}

#[tokio::test]
async fn a_remote_node_without_the_send_opt_in_is_refused() {
    let rpc = node().with_source(ChainSource::RemoteNode);
    let (_, r) = check_reason(seeded(), &rpc).await;
    assert!(r.unwrap().contains("sending via remote node is disabled"));
    let conn = seeded();
    queries::set_setting(&conn, "allow_remote_broadcast", "true").unwrap();
    let (_, r) = check_reason(conn, &rpc).await;
    assert_eq!(r, None);
}

#[tokio::test]
async fn a_node_on_another_chain_is_refused() {
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(BlockchainInfo {
            blocks: 120,
            chain: Some("main".into()),
            ..Default::default()
        })
        .with_mempool_entry(entry());
    let (_, r) = check_reason(seeded(), &rpc).await;
    assert!(r.unwrap().contains("node is on network 'main'"));
}

#[tokio::test]
async fn a_watch_only_profile_is_refused() {
    let conn = seeded();
    conn.execute(
        "UPDATE wallet_profiles SET watch_only = 1 WHERE id = ?1",
        params![PROFILE],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::WATCH_ONLY));
}

// --- R2 ------------------------------------------------------------------------

#[tokio::test]
async fn a_parent_without_change_is_refused() {
    let conn = seeded();
    conn.execute(
        "UPDATE wallet_tx_drafts SET summary_json = ?2 WHERE id = ?1",
        params![PARENT, send_summary(0)],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::NO_CHANGE));
}

#[tokio::test]
async fn a_covenant_draft_whose_change_index_is_its_name_output_is_refused() {
    let conn = seeded();
    let plan = serde_json::json!({
        "version": 0, "locktime": 0, "account": 0, "network": "regtest", "inputs": [],
        "outputs": [
            {"value": 0, "address": leaf(0, 0).0, "covenant_type": 2, "covenant_items_hex": []},
            {"value": CHANGE_VALUE, "address": leaf(1, 0).0, "covenant_type": 3, "covenant_items_hex": []}
        ],
        "change_output_index": 1
    });
    conn.execute(
        "UPDATE wallet_tx_drafts SET action = 'bid', signing_inputs_json = ?2 WHERE id = ?1",
        params![PARENT, plan.to_string()],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::NO_CHANGE));
}

#[tokio::test]
async fn a_tracked_change_with_a_covenant_is_refused() {
    let conn = seeded();
    conn.execute(
        "UPDATE tracked_utxos SET covenant_type = 9 WHERE txid = ?1",
        params![PARENT_TXID],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::NO_CHANGE));
}

#[tokio::test]
async fn change_not_synced_yet_is_refused() {
    let conn = seeded();
    conn.execute("DELETE FROM tracked_utxos", []).unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::CHANGE_NOT_SYNCED));
}

#[tokio::test]
async fn change_spent_by_another_transaction_of_ours_is_refused() {
    let conn = seeded();
    conn.execute(
        "UPDATE tracked_utxos SET spent_by_txid = ?2 WHERE txid = ?1",
        params![PARENT_TXID, OTHER_TXID],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::CHANGE_SPENT));
}

#[tokio::test]
async fn change_held_by_another_draft_is_refused() {
    let conn = seeded();
    queries::insert_tx_draft(&conn, "other", PROFILE, "send_hns", "", "{}", "{}").unwrap();
    conn.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = 'other' WHERE txid = ?1",
        params![PARENT_TXID],
    )
    .unwrap();
    let (_, r) = check_reason(conn, &node()).await;
    assert_eq!(r.as_deref(), Some(su::CHANGE_HELD));
}

// --- R3 ------------------------------------------------------------------------

#[tokio::test]
async fn one_child_per_parent_and_a_child_is_not_sped_up() {
    let rpc = node();
    let m = Mutex::new(seeded());
    let child = build_with(&m, &rpc, PARENT, Some(10), NOW).await.unwrap();
    assert_eq!(
        reason(&m, &rpc).await.as_deref(),
        Some(su::UNSENT_CHILD),
        "a second child is refused"
    );
    let err = build_with(&m, &rpc, PARENT, Some(10), NOW)
        .await
        .unwrap_err();
    assert_eq!(refusal(err), su::UNSENT_CHILD);
    let r = speed_up_check_with(&m, &rpc, &child.id, NOW).await.reason;
    assert_eq!(r.as_deref(), Some(su::IS_CHILD));

    // A dropped child no longer holds the parent.
    {
        let conn = m.lock().unwrap();
        queries::update_tx_draft_status(
            &conn,
            &child.id,
            queries::DraftStatus::Dropped,
            None,
            None,
        )
        .unwrap();
        queries::release_reserved_utxos_for_draft(&conn, &child.id).unwrap();
    }
    assert_eq!(reason(&m, &rpc).await, None);
}

#[tokio::test]
async fn a_broadcast_child_spending_the_change_reads_as_already_sped_up() {
    let rpc = node();
    let m = Mutex::new(seeded());
    let child = build_with(&m, &rpc, PARENT, Some(10), NOW).await.unwrap();
    {
        let conn = m.lock().unwrap();
        let txid = child.summary["txid"].as_str().unwrap().to_string();
        queries::update_tx_draft_status(
            &conn,
            &child.id,
            queries::DraftStatus::Broadcasted,
            None,
            Some(&txid),
        )
        .unwrap();
        conn.execute(
            "UPDATE tracked_utxos SET spent_by_txid = ?2, reserved_by_draft_id = NULL WHERE txid = ?1",
            params![PARENT_TXID, txid],
        )
        .unwrap();
    }
    assert_eq!(reason(&m, &rpc).await.as_deref(), Some(su::ALREADY_SPED_UP));
}

// --- R4, R12 ---------------------------------------------------------------------

fn make_purchase(conn: &Connection, state: &str) {
    conn.execute(
        "UPDATE wallet_tx_drafts SET action = 'shakedex_purchase', signing_inputs_json = ?2 WHERE id = ?1",
        params![
            PARENT,
            serde_json::json!({
                "version": 0, "locktime": 0, "account": 0, "network": "regtest", "inputs": [],
                "outputs": [
                    {"value": 0, "address": leaf(0, 1).0, "covenant_type": 9, "covenant_items_hex": []},
                    {"value": CHANGE_VALUE, "address": leaf(1, 0).0, "covenant_type": 0, "covenant_items_hex": []},
                    {"value": 5, "address": leaf(0, 0).0, "covenant_type": 0, "covenant_items_hex": []}
                ],
                "change_output_index": 1
            })
            .to_string()
        ],
    )
    .unwrap();
    queries::insert_shakedex_purchase(
        conn,
        &queries::ShakedexPurchase {
            id: "purchase1".into(),
            wallet_profile_id: PROFILE.into(),
            name: "nameholdtest".into(),
            listing_json: "{}".into(),
            lock_txid: OTHER_TXID.into(),
            lock_vout: 0,
            price_doos: 5,
            purchase_draft_id: PARENT.into(),
            purchase_txid: PARENT_TXID.into(),
            destination_address: leaf(0, 1).0,
            state: state.parse().unwrap(),
            purchase_height: None,
            blocks_remaining: None,
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: None,
            finalize_draft_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .unwrap();
}

#[tokio::test]
async fn a_purchase_is_sped_up_only_while_unconfirmed() {
    for state in ["pending_send", "awaiting_finalize", "owned", "lost"] {
        let conn = seeded();
        make_purchase(&conn, state);
        let (_, r) = check_reason(conn, &node()).await;
        assert_eq!(r.as_deref(), Some(su::PURCHASE_NOT_UNCONFIRMED), "{state}");
    }
}

#[tokio::test]
async fn the_held_change_of_an_unconfirmed_purchase_is_spent_by_its_child_only() {
    let conn = seeded();
    make_purchase(&conn, "unconfirmed");
    // Ordinary coin selection holds it back (spec 2026-10-05 R13).
    let coins = load_spendable_coins(&conn, PROFILE, None, Network::Regtest).unwrap();
    assert!(coins.iter().all(|c| c.txid != PARENT_TXID), "{coins:?}");
    // Its child names it itself.
    let m = Mutex::new(conn);
    let child = build_with(&m, &node(), PARENT, Some(10), NOW)
        .await
        .unwrap();
    let summary: SpeedUpSummary = serde_json::from_value(child.summary.clone()).unwrap();
    assert!(summary.parent_is_purchase);
    // And the child's output falls with the purchase: held back as well.
    let conn = m.lock().unwrap();
    track(&conn, &summary.txid, 0, 1, 0, summary.change_doos);
    let coins = load_spendable_coins(&conn, PROFILE, None, Network::Regtest).unwrap();
    assert!(coins.iter().all(|c| c.txid != summary.txid), "{coins:?}");
}

// --- R5, R6 at the command ----------------------------------------------------------

#[tokio::test]
async fn the_parents_entry_refusals_reach_the_row() {
    let late = MockNodeRpc::new()
        .with_blockchain_info(regtest_tip())
        .with_mempool_entry(entry_at(NOW - 66 * 3600 - 1, 140));
    let (_, r) = check_reason(seeded(), &late).await;
    assert_eq!(r.as_deref(), Some(su::EXPIRES_SOON));
    let free = MockNodeRpc::new()
        .with_blockchain_info(regtest_tip())
        .with_mempool_entry(entry_at(NOW, 139));
    let (_, r) = check_reason(seeded(), &free).await;
    assert_eq!(r.as_deref(), Some(su::BELOW_RELAY));
}

// --- R7, R9 -----------------------------------------------------------------------

#[tokio::test]
async fn build_stores_the_child_and_reserves_the_change() {
    let rpc = node();
    let m = Mutex::new(seeded());
    let child = build_with(&m, &rpc, PARENT, Some(10), NOW).await.unwrap();
    assert_eq!(child.action, SPEED_UP_ACTION);
    assert_eq!(child.status, queries::DraftStatus::Draft);
    let conn = m.lock().unwrap();
    let row = queries::get_tx_draft(&conn, &child.id).unwrap().unwrap();
    let plan: DraftPlan = serde_json::from_str(&row.signing_inputs_json).unwrap();
    assert_eq!(plan.own_inputs(), vec![(PARENT_TXID.to_string(), 1)]);
    assert_eq!(plan.outputs.len(), 1);
    assert_eq!(plan.outputs[0].address, leaf(1, 0).0);
    assert_eq!(plan.change_output_index, Some(0));
    assert!(!row.unsigned_tx_hex.is_empty());
    let summary: SpeedUpSummary = serde_json::from_str(&row.summary_json).unwrap();
    assert_eq!(summary.parent_txid, PARENT_TXID);
    assert_eq!(summary.parent_draft_id, PARENT);
    assert_eq!(summary.fee_doos, 10 * (140 + 108) - 140);
    assert_eq!(summary.package_rate_after, 10_000);
    let held: Option<String> = conn
        .query_row(
            "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = ?1",
            params![PARENT_TXID],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(held.as_deref(), Some(child.id.as_str()));
}

#[tokio::test]
async fn preview_stores_nothing() {
    let m = Mutex::new(seeded());
    let s = preview_with(&m, &node(), PARENT, Some(10), NOW)
        .await
        .unwrap();
    assert_eq!(s.fee_doos, 10 * 248 - 140);
    let conn = m.lock().unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM wallet_tx_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn the_target_is_the_typed_rate_the_setting_the_estimate_or_the_floor() {
    let m = Mutex::new(seeded());
    let s = preview_with(&m, &node(), PARENT, Some(7), NOW)
        .await
        .unwrap();
    assert_eq!(
        (s.target_rate_per_vbyte, s.target_source),
        (7, TargetSource::Typed)
    );
    let s = preview_with(&m, &node().with_estimate_smart_fee(12), PARENT, None, NOW)
        .await
        .unwrap();
    assert_eq!(
        (s.target_rate_per_vbyte, s.target_source),
        (12, TargetSource::Estimate)
    );
    // No estimate from the node: the fee-rate setting, seeded at the floor
    // (migration 009), which every parent the mempool took already pays, so
    // the form asks for a rate and says why.
    let rpc = node();
    let err = preview_with(&m, &rpc, PARENT, None, NOW).await.unwrap_err();
    assert_eq!(refusal(err), su::NO_ESTIMATE);
    assert!(rpc.calls().contains(&RpcCall::EstimateSmartFee(6)));
    queries::set_setting(&m.lock().unwrap(), "fee_rate_doos_per_kvb", "3000").unwrap();
    let s = preview_with(&m, &node(), PARENT, None, NOW).await.unwrap();
    assert_eq!(
        (s.target_rate_per_vbyte, s.target_source),
        (3, TargetSource::Setting)
    );
    // The estimate comes first.
    let s = preview_with(&m, &node().with_estimate_smart_fee(12), PARENT, None, NOW)
        .await
        .unwrap();
    assert_eq!(s.target_source, TargetSource::Estimate);
    // No setting either: the floor.
    m.lock()
        .unwrap()
        .execute(
            "DELETE FROM settings WHERE key = 'fee_rate_doos_per_kvb'",
            [],
        )
        .unwrap();
    let err = preview_with(&m, &node(), PARENT, None, NOW)
        .await
        .unwrap_err();
    assert_eq!(refusal(err), su::NO_ESTIMATE);
    let err = preview_with(&m, &node(), PARENT, Some(0), NOW)
        .await
        .unwrap_err();
    assert!(refusal(err).contains("at least 1 doo"));
}

// --- R13 ----------------------------------------------------------------------------

#[tokio::test]
async fn the_ledger_guard_passes_the_childs_plan() {
    let m = Mutex::new(seeded());
    let child = build_with(&m, &node(), PARENT, Some(10), NOW)
        .await
        .unwrap();
    let conn = m.lock().unwrap();
    let row = queries::get_tx_draft(&conn, &child.id).unwrap().unwrap();
    refuse_unsupported_ledger_plan("ledger_hardware", &row.action, &row.signing_inputs_json)
        .unwrap();
}

// --- R11 ----------------------------------------------------------------------------

#[tokio::test]
async fn the_confirmation_says_what_is_paid_and_what_is_not_changed() {
    let m = Mutex::new(seeded());
    let child = build_with(&m, &node(), PARENT, Some(10), NOW)
        .await
        .unwrap();
    let conn = m.lock().unwrap();
    let row = queries::get_tx_draft(&conn, &child.id).unwrap().unwrap();
    let details = confirm_details_for_draft(&row).unwrap();
    let rows: Vec<(String, String)> = details["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["label"].as_str().unwrap().to_string(),
                r["value"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let value = |label: &str| {
        rows.iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("no {label} row in {rows:?}"))
    };
    assert_eq!(value("Action"), "Speed up");
    assert_eq!(value("Speeds up"), PARENT_TXID);
    assert_eq!(value("Fee"), "0.002340 HNS");
    assert_eq!(
        value("Fee rate"),
        "1.00 doos/vB now, 10.00 doos/vB with this fee (target 10 doos/vB, the rate you chose)"
    );
    assert_eq!(value("Paid to"), "The network only: nobody else is paid.");
    assert_eq!(value("Back to your wallet"), "0.997660 HNS");
    assert_eq!(
        value("Original transaction"),
        "It keeps its transaction id."
    );
    // time NOW - 3600 + 72 h = 1761255600 = 2025-10-23 21:40 UTC.
    assert_eq!(
        value("Deadline"),
        "Speeding up does not extend the 72 hours your node keeps the original transaction: if it is not mined by 2025-10-23 21:40 UTC, both are dropped and nothing is paid by either."
    );
    assert_eq!(value("Txid"), child.summary["txid"].as_str().unwrap());
}

#[tokio::test]
async fn a_purchases_confirmation_says_the_price_is_unchanged() {
    let conn = seeded();
    make_purchase(&conn, "unconfirmed");
    let m = Mutex::new(conn);
    let child = build_with(&m, &node(), PARENT, Some(10), NOW)
        .await
        .unwrap();
    let conn = m.lock().unwrap();
    let row = queries::get_tx_draft(&conn, &child.id).unwrap().unwrap();
    let details = confirm_details_for_draft(&row).unwrap().to_string();
    assert!(
        details.contains("The seller's price and the purchase itself are unchanged."),
        "{details}"
    );
}

// --- R14 ----------------------------------------------------------------------------

async fn built_child(m: &Mutex<Connection>) -> String {
    build_with(m, &node(), PARENT, Some(10), NOW)
        .await
        .unwrap()
        .id
}

fn status_of(m: &Mutex<Connection>, id: &str) -> (queries::DraftStatus, Option<String>) {
    let d = queries::get_tx_draft(&m.lock().unwrap(), id)
        .unwrap()
        .unwrap();
    (d.status, d.error_message)
}

fn held_by(m: &Mutex<Connection>) -> Option<String> {
    m.lock()
        .unwrap()
        .query_row(
            "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = ?1",
            params![PARENT_TXID],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn broadcast_rechecks_the_parent_and_drops_an_unsent_child_when_it_is_gone() {
    let m = Mutex::new(seeded());
    let id = built_child(&m).await;
    recheck_before_broadcast(&m, &node(), &id).await.unwrap();

    let gone = MockNodeRpc::new().with_mempool_entry_not_in_mempool();
    let err = recheck_before_broadcast(&m, &gone, &id).await.unwrap_err();
    assert_eq!(refusal(err), su::PARENT_GONE);
    assert_eq!(
        gone.calls(),
        vec![RpcCall::MempoolEntry(PARENT_TXID.into())]
    );
    assert_eq!(
        status_of(&m, &id),
        (
            queries::DraftStatus::Dropped,
            Some(su::PARENT_GONE.to_string())
        )
    );
    assert_eq!(held_by(&m), None, "its coin is released");
}

#[tokio::test]
async fn broadcast_recheck_keeps_a_child_the_node_may_hold() {
    let m = Mutex::new(seeded());
    let id = built_child(&m).await;
    queries::update_tx_draft_status(
        &m.lock().unwrap(),
        &id,
        queries::DraftStatus::BroadcastPending,
        None,
        None,
    )
    .unwrap();
    let gone = MockNodeRpc::new().with_mempool_entry_not_in_mempool();
    assert!(recheck_before_broadcast(&m, &gone, &id).await.is_err());
    assert_eq!(status_of(&m, &id).0, queries::DraftStatus::BroadcastPending);
    assert_eq!(held_by(&m).as_deref(), Some(id.as_str()));
}

#[tokio::test]
async fn broadcast_recheck_without_an_answer_changes_nothing() {
    let m = Mutex::new(seeded());
    let id = built_child(&m).await;
    let silent = MockNodeRpc::new();
    let err = recheck_before_broadcast(&m, &silent, &id)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Rpc(_)), "{err:?}");
    assert_eq!(status_of(&m, &id).0, queries::DraftStatus::Draft);
}

#[tokio::test]
async fn broadcast_recheck_refuses_a_child_of_a_purchase_that_moved_on() {
    let conn = seeded();
    make_purchase(&conn, "unconfirmed");
    let m = Mutex::new(conn);
    let id = built_child(&m).await;
    m.lock()
        .unwrap()
        .execute(
            "UPDATE shakedex_purchases SET state = 'awaiting_finalize'",
            [],
        )
        .unwrap();
    let rpc = node();
    let err = recheck_before_broadcast(&m, &rpc, &id).await.unwrap_err();
    assert_eq!(refusal(err), PURCHASE_MOVED_ON);
    assert_eq!(status_of(&m, &id).0, queries::DraftStatus::Dropped);
}

// --- R15 ----------------------------------------------------------------------------

#[tokio::test]
async fn a_dead_childs_note_names_its_parent() {
    let m = Mutex::new(seeded());
    let id = built_child(&m).await;
    let conn = m.lock().unwrap();
    let child = queries::get_tx_draft(&conn, &id).unwrap().unwrap();
    assert_eq!(
        spent_elsewhere_note(&child),
        su::parent_died_reason(PARENT_TXID)
    );
    let parent = queries::get_tx_draft(&conn, PARENT).unwrap().unwrap();
    assert_eq!(spent_elsewhere_note(&parent), SPENT_ELSEWHERE);
}

// --- R7: the database checks again inside the build's transaction ------------------

#[tokio::test]
async fn a_draft_that_takes_the_change_while_the_node_is_asked_refuses_the_build() {
    let m = std::sync::Arc::new(Mutex::new(seeded()));
    let during = m.clone();
    // While the build waits for the node, another draft takes the change.
    let rpc = MockNodeRpc::new()
        .with_blockchain_info(regtest_tip())
        .with_mempool_entry_fn(move |_| {
            let conn = during.lock().unwrap();
            if queries::get_tx_draft(&conn, "other").unwrap().is_none() {
                queries::insert_tx_draft(&conn, "other", PROFILE, "send_hns", "", "{}", "{}")
                    .unwrap();
                conn.execute(
                    "UPDATE tracked_utxos SET reserved_by_draft_id = 'other' WHERE txid = ?1",
                    params![PARENT_TXID],
                )
                .unwrap();
            }
            Ok(entry())
        });
    let err = build_with(&m, &rpc, PARENT, Some(10), NOW)
        .await
        .unwrap_err();
    assert_eq!(refusal(err), su::CHANGE_HELD);
    let n: i64 = m
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM wallet_tx_drafts WHERE action = ?1",
            params![SPEED_UP_ACTION],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "no child was stored");
}

// --- the commands' own call sites: signing (R8, R10) and broadcast (R14) -----------

fn app_with(conn: Connection) -> tauri::App<tauri::test::MockRuntime> {
    use tauri::test::{mock_builder, mock_context, noop_assets};
    mock_builder()
        .manage(crate::AppState {
            db: Mutex::new(conn),
            signer: Mutex::new(None),
            secure_prompts: Mutex::new(std::collections::HashMap::new()),
            hsd_child: Mutex::new(None),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn unlock(app: &tauri::App<tauri::test::MockRuntime>) {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    *state.signer.lock().unwrap() = Some(crate::noncustodial::session::SignerSession::unlock(
        PROFILE.to_string(),
        Network::Regtest,
        ExtendedPrivKey::from_seed(&seed()).unwrap(),
        600_000,
    ));
}

/// An app whose profile's node is `url`, holding a child built against the
/// mocked node; the child's id.
async fn app_with_child(url: &str) -> (tauri::App<tauri::test::MockRuntime>, String) {
    use tauri::Manager;
    let conn = seeded();
    queries::set_setting(&conn, "node_rpc_url", url).unwrap();
    let app = app_with(conn);
    let id = {
        let state = app.state::<crate::AppState>();
        build_with(&state.db, &node(), PARENT, Some(10), NOW)
            .await
            .unwrap()
            .id
    };
    (app, id)
}

fn app_status(app: &tauri::App<tauri::test::MockRuntime>, id: &str) -> queries::TxDraftRow {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let conn = state.db.lock().unwrap();
    queries::get_tx_draft(&conn, id).unwrap().unwrap()
}

#[tokio::test]
async fn signing_checks_the_signed_child_against_its_summary() {
    use tauri::Manager;
    let (app, id) = app_with_child("http://127.0.0.1:9").await;
    unlock(&app);
    {
        let state = app.state::<crate::AppState>();
        let conn = state.db.lock().unwrap();
        let mut s: SpeedUpSummary = serde_json::from_str(
            &queries::get_tx_draft(&conn, &id)
                .unwrap()
                .unwrap()
                .summary_json,
        )
        .unwrap();
        s.fee_doos += 1;
        conn.execute(
            "UPDATE wallet_tx_drafts SET summary_json = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(&s).unwrap()],
        )
        .unwrap();
    }
    let err = crate::commands::tx::sign_tx_draft_inner(&app.state(), &id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("fee is not the one reviewed"),
        "{err}"
    );
    assert_eq!(app_status(&app, &id).status, queries::DraftStatus::Draft);
}

#[tokio::test]
async fn broadcast_of_a_child_whose_parent_left_the_mempool_sends_nothing() {
    let mut server = mockito::Server::new_async().await;
    let _info = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(r#"{"result":{"chain":"regtest","blocks":120},"error":null,"id":1}"#)
        .create_async()
        .await;
    // hsd 8.0.0 `getMempoolEntry`: RPCError(MISC_ERROR, 'Transaction not in mempool.').
    let _entry = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getmempoolentry".into()))
        .with_body(
            r#"{"result":null,"error":{"message":"Transaction not in mempool.","code":-1},"id":1}"#,
        )
        .create_async()
        .await;
    let send = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("sendrawtransaction".into()))
        .with_body(r#"{"result":"00","error":null,"id":1}"#)
        .expect(0)
        .create_async()
        .await;
    let (app, id) = app_with_child(&server.url()).await;
    unlock(&app);
    use tauri::Manager;
    crate::commands::tx::sign_tx_draft_inner(&app.state(), &id)
        .await
        .unwrap();
    let err = crate::commands::tx::broadcast_tx_draft(app.state(), id.clone())
        .await
        .unwrap_err();
    assert_eq!(refusal(err), su::PARENT_GONE);
    send.assert_async().await;
    let row = app_status(&app, &id);
    assert_eq!(row.status, queries::DraftStatus::Dropped);
    assert_eq!(row.error_message.as_deref(), Some(su::PARENT_GONE));
}

#[tokio::test]
async fn signing_refuses_a_child_whose_plan_pays_elsewhere_than_our_change() {
    use tauri::Manager;
    let (app, id) = app_with_child("http://127.0.0.1:9").await;
    unlock(&app);
    {
        // A plan and summary rewritten together to pay receive 0/1: the
        // txid binding alone would pass it.
        let state = app.state::<crate::AppState>();
        let conn = state.db.lock().unwrap();
        let row = queries::get_tx_draft(&conn, &id).unwrap().unwrap();
        let mut plan: DraftPlan = serde_json::from_str(&row.signing_inputs_json).unwrap();
        plan.outputs[0].address = leaf(0, 1).0;
        let txid = crate::noncustodial::actions::rebuild_unsigned(&plan, Network::Regtest)
            .unwrap()
            .txid();
        let mut s: SpeedUpSummary = serde_json::from_str(&row.summary_json).unwrap();
        s.txid = txid;
        conn.execute(
            "UPDATE wallet_tx_drafts SET signing_inputs_json = ?2, summary_json = ?3 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&plan).unwrap(),
                serde_json::to_string(&s).unwrap()
            ],
        )
        .unwrap();
    }
    let err = crate::commands::tx::sign_tx_draft_inner(&app.state(), &id)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not your change address"), "{err}");
    assert_eq!(app_status(&app, &id).status, queries::DraftStatus::Draft);
}
