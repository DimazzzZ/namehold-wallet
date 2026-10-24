//! R34: the wallet finds its own Shakedex locks after a restore. The search
//! (`find_locks_with`) and the status line's read (`lock_search_status_with`)
//! run against a `MockNodeRpc`; the commands' gates run against the R32
//! restore fixture.

use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tauri::Manager;

use crate::commands::shakedex::{restore_lock_inner, RestoreInput};
use crate::commands::shakedex_discover::{
    find_locks_with, lock_search_status_with, shakedex_find_locks, shakedex_lock_search_status,
    FindLocksReport, LOCK_SEARCH_NEEDS_INDEX,
};
use crate::db::queries::{self, ListingState};
use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::hd;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{BlockchainInfo, NodeCoin};
use crate::noncustodial::shakedex::lock_key::{derive_lock_key, LockKey};
use crate::noncustodial::shakedex::sell;
use crate::noncustodial::sync::COV_TRANSFER;
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};
use crate::tests::shakedex_cmd_tests::{err_text, seed, seeded, PROFILE};
use crate::tests::shakedex_sell_tests::{
    addr00, count, master, our_lock_coin, restore_fixture_of, restored_info, App, NAME, QUIET_TIP,
};
use crate::AppState;

const NET: Network = Network::Regtest;

/// A fresh profile from the same phrase: its receive address 0/0 only,
/// marked used as the sync before the search leaves an address with history,
/// no listing, the search not done.
fn store() -> Mutex<Connection> {
    let db = Mutex::new(seeded("regtest", "mnemonic_hot", "http://127.0.0.1:1"));
    mark_used(&db, &addr00(NET).0);
    db
}

fn mark_used(db: &Mutex<Connection>, address: &str) {
    crate::noncustodial::sync::mark_address_used(&db.lock().unwrap(), PROFILE, address, None)
        .unwrap();
}

/// Our address `branch/index`, added to the profile's derived addresses,
/// unused: in the window's lookahead when above the branch's used ones.
fn add_address(db: &Mutex<Connection>, branch: u32, index: u32) -> String {
    let (_sk, pk, addr) = hd::derive_address(NET, &seed(), 0, branch, index).unwrap();
    db.lock()
        .unwrap()
        .execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6)",
            params![
                PROFILE,
                branch,
                index,
                addr,
                hex::encode(address::script_pubkey_from_pubkey(&pk).unwrap()),
                hex::encode(pk)
            ],
        )
        .unwrap();
    addr
}

/// Our change address 1/0, added to the profile's derived addresses and
/// marked used, as a sync that saw its history leaves it.
fn add_change_address(db: &Mutex<Connection>) -> String {
    let addr = add_address(db, 1, 0);
    mark_used(db, &addr);
    addr
}

fn derive(name: &str) -> Result<LockKey, AppError> {
    derive_lock_key(&master(), NET, 0, name)
}

fn name_hash() -> String {
    hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap())
}

/// A transaction with a TRANSFER of NAME at `at` committing to the version-0
/// `program`, as `GET /tx/address` reports it.
fn transfer_tx(at: &str, program: &[u8]) -> Value {
    json!({ "hash": "88".repeat(32), "height": QUIET_TIP - 10, "outputs": [{
        "value": 1_000_000, "address": at,
        "covenant": { "type": COV_TRANSFER, "action": "TRANSFER",
                      "items": [name_hash(), "32000000", "00", hex::encode(program)] }
    }]})
}

/// NAME's lock TRANSFER at our receive address 0/0, committing to our lock.
fn our_lock_transfer() -> Value {
    transfer_tx(&addr00(NET).0, &derive(NAME).unwrap().program)
}

fn synced() -> BlockchainInfo {
    BlockchainInfo {
        blocks: QUIET_TIP,
        headers: Some(QUIET_TIP),
        verification_progress: Some(1.0),
        ..Default::default()
    }
}

/// hsd's `GET /` reply with its two indexes as given.
fn node_status(index_tx: bool, index_address: bool) -> Value {
    json!({ "version": "8.0.0", "network": "regtest",
            "chain": { "height": QUIET_TIP,
                       "indexers": { "indexTX": index_tx, "indexAddress": index_address } } })
}

fn lock_coin() -> NodeCoin {
    serde_json::from_value(our_lock_coin()).unwrap()
}

/// A synced node with both indexes whose history holds `txs` at every
/// address, naming NAME for its hash, and NAME's owner coin our lock's FINALIZE (the R32 fixture).
fn node_with(txs: Vec<Value>) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_blockchain_info(synced())
        .with_node_status(node_status(true, true))
        .with_txs_by_address(txs)
        .with_name_by_hash(Some(NAME.into()))
        .with_name_info(restored_info())
        .with_get_coin(|_, _| Ok(Some(lock_coin())))
}

async fn find(node: &MockNodeRpc, db: &Mutex<Connection>, force: bool) -> FindLocksReport {
    find_locks_with(node, db, PROFILE, NET, force, &derive)
        .await
        .expect("the search")
}

fn done(db: &Mutex<Connection>) -> bool {
    queries::lock_search_done(&db.lock().unwrap(), PROFILE).unwrap()
}

fn listings(db: &Mutex<Connection>) -> i64 {
    db.lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM shakedex_listings", [], |r| r.get(0))
        .unwrap()
}

fn name_reads(node: &MockNodeRpc) -> usize {
    node.count_matching(|c| matches!(c, RpcCall::NameInfo(_)))
}

/// R34: a lock TRANSFER at our address committing to the lock of the key
/// derived for its name, whose owner coin is still the FINALIZE at that
/// lock, is adopted as R32's restore would: a Restored lock at the owner
/// outpoint, and the search is done.
#[tokio::test]
async fn finds_and_adopts_a_lock_still_in_it() {
    let db = store();
    let node = node_with(vec![our_lock_transfer()]);
    let r = find(&node, &db, false).await;
    assert!(r.ran);
    assert_eq!(r.adopted, vec![NAME.to_string()]);
    assert!(r.done && r.complete);
    assert!(done(&db));
    let l = queries::open_shakedex_listing_for_name(&db.lock().unwrap(), PROFILE, NAME)
        .unwrap()
        .expect("the adopted lock");
    assert_eq!(l.state, ListingState::Restored);
    assert_eq!(l.lock_pubkey_hex, hex::encode(derive(NAME).unwrap().pubkey));
    assert_eq!(
        node.count_matching(|c| *c == RpcCall::NameByHash(name_hash())),
        1
    );
}

/// R32's order, "derived after the node reads": the key an adopted lock is
/// restored with is derived once the name's owner and its coin are read; the
/// first derivation, before `getnameinfo`, only reads the lock program the
/// candidate is checked against, and is dropped. A candidate whose committed
/// address is not our lock is rejected on that first derivation alone.
#[tokio::test]
async fn the_key_for_adoption_is_derived_after_the_node_reads() {
    let at_derive = std::sync::Mutex::new(Vec::<Vec<RpcCall>>::new());
    let db = store();
    let node = node_with(vec![our_lock_transfer()]);
    let counting = |n: &str| {
        at_derive.lock().unwrap().push(node.calls());
        derive(n)
    };
    let r = find_locks_with(&node, &db, PROFILE, NET, false, &counting)
        .await
        .expect("the search");
    assert_eq!(r.adopted, vec![NAME.to_string()]);
    let seen = at_derive.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "once for the check, once for adoption");
    let reads = |calls: &[RpcCall]| {
        (
            calls
                .iter()
                .filter(|c| matches!(c, RpcCall::NameInfo(_)))
                .count(),
            calls
                .iter()
                .filter(|c| matches!(c, RpcCall::GetCoin(..)))
                .count(),
        )
    };
    assert_eq!(
        reads(&seen[0]),
        (0, 0),
        "the check comes before the owner reads"
    );
    assert_eq!(reads(&seen[1]), (1, 1), "adoption's key comes after them");

    let at_derive = std::sync::Mutex::new(0usize);
    let db = store();
    let node = node_with(vec![transfer_tx(&addr00(NET).0, &[0x42; 32])]);
    let counting = |n: &str| {
        *at_derive.lock().unwrap() += 1;
        derive(n)
    };
    let r = find_locks_with(&node, &db, PROFILE, NET, false, &counting)
        .await
        .expect("the search");
    assert!(r.adopted.is_empty());
    assert_eq!(*at_derive.lock().unwrap(), 1, "a rejected candidate: once");
}

/// A key for adoption that is not the one checked (the signer's seed changed
/// between the two derivations) adopts nothing: the candidate is not judged,
/// and the search is not done.
#[tokio::test]
async fn a_key_changed_between_the_check_and_adoption_leaves_the_candidate_unjudged() {
    let n = std::sync::Mutex::new(0usize);
    let db = store();
    let node = node_with(vec![our_lock_transfer()]);
    let changing = |name: &str| {
        let mut n = n.lock().unwrap();
        *n += 1;
        if *n == 1 {
            derive(name)
        } else {
            derive_lock_key(&master(), NET, 1, name)
        }
    };
    let r = find_locks_with(&node, &db, PROFILE, NET, false, &changing)
        .await
        .expect("the search");
    assert!(r.adopted.is_empty());
    assert!(!r.complete && !done(&db));
    assert_eq!(listings(&db), 0);
}

/// Review focus 1: a plain Transfer of a name to someone else is a candidate
/// whose committed address is not our lock: judged and skipped before the
/// name's owner is read, never adopted (even if that owner were our lock),
/// and it does not keep the search from being done.
#[tokio::test]
async fn a_plain_transfer_to_someone_else_is_judged_not_adopted() {
    let db = store();
    let node = node_with(vec![transfer_tx(&addr00(NET).0, &[0x42; 20])]);
    let r = find(&node, &db, false).await;
    assert!(r.adopted.is_empty());
    assert!(r.done && done(&db));
    assert_eq!(name_reads(&node), 0, "the owner is not read");
    assert_eq!(listings(&db), 0);
}

/// Review focus 5: a name already restored by hand on this device is refused
/// by R32's guard (already tracked): judged, no second row, not counted.
#[tokio::test]
async fn a_name_already_restored_is_judged_not_adopted_twice() {
    let db = store();
    {
        let conn = db.lock().unwrap();
        let owner = sell::restore_owner(&restored_info(), NAME).unwrap();
        restore_lock_inner(
            &conn,
            &RestoreInput {
                profile_id: PROFILE,
                network: NET,
                name: NAME,
                key: &derive(NAME).unwrap(),
                owner: &owner,
                coin: &lock_coin(),
            },
        )
        .unwrap();
    }
    let node = node_with(vec![our_lock_transfer()]);
    let r = find(&node, &db, false).await;
    assert!(r.adopted.is_empty());
    assert!(r.done && done(&db));
    assert_eq!(listings(&db), 1);
}

/// Review focus 4: hsd's `null` for the name hash (no name state) is a
/// judged candidate: skipped, the search done.
#[tokio::test]
async fn a_name_the_node_does_not_know_is_judged() {
    let db = store();
    let node = node_with(vec![our_lock_transfer()]).with_name_by_hash(None);
    let r = find(&node, &db, false).await;
    assert!(r.adopted.is_empty());
    assert!(r.done && done(&db));
    assert_eq!(name_reads(&node), 0);
}

/// Review focus 3: one address's history read fails: the locks found at the
/// other addresses are adopted, but the search is not done.
#[tokio::test]
async fn a_failed_history_read_keeps_the_search_not_done() {
    let db = store();
    let change = add_change_address(&db);
    let node = node_with(vec![]).with_txs_by_address_fn(move |a| {
        if a == change {
            Err(AppError::Rpc("connection reset".into()))
        } else {
            Ok(vec![our_lock_transfer()])
        }
    });
    let r = find(&node, &db, false).await;
    assert!(r.ran);
    assert_eq!(r.adopted, vec![NAME.to_string()]);
    assert!(!r.done);
    assert!(!r.complete, "this run did not read every address");
    assert!(!done(&db));
    assert_eq!(listings(&db), 1);
}

/// An address of the window's lookahead with history means the window would
/// still grow (a sync that read it as unused, or stopped at its round cap,
/// left it so): addresses beyond it are not derived yet, so the search ran
/// but is neither complete nor done, and the next search runs again. With
/// that lookahead's history empty, it is done.
#[tokio::test]
async fn a_lookahead_address_with_history_keeps_the_search_not_done() {
    let db = store();
    let next = add_address(&db, 0, 1);
    let at_next = next.clone();
    let payment = json!({ "hash": "77".repeat(32), "height": QUIET_TIP - 3, "outputs": [{
        "value": 1_000, "address": next,
        "covenant": { "type": 0, "action": "NONE", "items": [] }
    }]});
    let node = node_with(vec![]).with_txs_by_address_fn(move |a| {
        Ok(if a == at_next {
            vec![payment.clone()]
        } else {
            vec![our_lock_transfer()]
        })
    });
    let r = find(&node, &db, false).await;
    assert!(r.ran);
    assert_eq!(r.adopted, vec![NAME.to_string()]);
    assert!(!r.complete && !r.done, "{r:?}");
    assert!(!done(&db));

    let at_next = next.clone();
    let node = node_with(vec![]).with_txs_by_address_fn(move |a| {
        Ok(if a == at_next {
            vec![]
        } else {
            vec![our_lock_transfer()]
        })
    });
    let r = find(&node, &db, false).await;
    assert!(r.ran && r.adopted.is_empty());
    assert!(r.complete && r.done && done(&db), "{r:?}");
}

/// A candidate whose name read fails is not judged: the search is not done,
/// and the next one tries it again.
#[tokio::test]
async fn a_failed_name_read_keeps_the_search_not_done() {
    let db = store();
    let node = node_with(vec![our_lock_transfer()]).with_name_by_hash_err("timeout");
    let r = find(&node, &db, false).await;
    assert!(r.ran && r.adopted.is_empty());
    assert!(!r.done && !done(&db));
    assert!(!r.complete, "a candidate was not judged");
    let node = node_with(vec![our_lock_transfer()]).with_name_info_err("timeout");
    let r = find(&node, &db, false).await;
    assert!(r.ran && r.adopted.is_empty());
    assert!(!r.done && !done(&db));
    let r = find(&node_with(vec![our_lock_transfer()]), &db, false).await;
    assert_eq!(r.adopted, vec![NAME.to_string()]);
    assert!(r.done);
}

/// R32 refusing the name for a reason of its own (here the name still on
/// its way into the lock: its owner coin our TRANSFER committing to the
/// lock) judges the candidate: skipped, nothing adopted, the search complete
/// and done.
#[tokio::test]
async fn a_lock_still_on_its_way_in_is_judged() {
    let db = store();
    let mut locking = our_lock_coin();
    let height = locking["covenant"]["items"][1].clone();
    locking["address"] = json!(addr00(NET).0);
    locking["covenant"] = json!({ "type": COV_TRANSFER, "action": "TRANSFER", "items": [
        name_hash(), height, "00", hex::encode(derive(NAME).unwrap().program)
    ]});
    let coin: NodeCoin = serde_json::from_value(locking).unwrap();
    let node =
        node_with(vec![our_lock_transfer()]).with_get_coin(move |_, _| Ok(Some(coin.clone())));
    let r = find(&node, &db, false).await;
    assert!(r.ran && r.adopted.is_empty());
    assert!(r.complete && r.done && done(&db), "{r:?}");
    assert_eq!(listings(&db), 0);
}

/// hsd's 404 for the name's owner coin (`get_coin` is `None`) judges the
/// candidate: skipped, the search complete and done.
#[tokio::test]
async fn an_owner_coin_the_node_does_not_have_is_judged() {
    let db = store();
    let node = node_with(vec![our_lock_transfer()]).with_get_coin(|_, _| Ok(None));
    let r = find(&node, &db, false).await;
    assert!(r.ran && r.adopted.is_empty());
    assert!(r.complete && r.done && done(&db), "{r:?}");
    assert_eq!(listings(&db), 0);
}

/// Once done, the search does not run again (not a single node call) unless
/// forced, as "Find my locks" does.
#[tokio::test]
async fn a_done_search_does_not_run_again_unless_forced() {
    let db = store();
    queries::mark_lock_search_done(&db.lock().unwrap(), PROFILE).unwrap();
    let node = MockNodeRpc::new().with_blockchain_info_err("must not be called");
    let r = find(&node, &db, false).await;
    assert!(!r.ran && r.adopted.is_empty() && r.done);
    assert_eq!(node.call_count(), 0);
    let node = node_with(vec![our_lock_transfer()]);
    let r = find(&node, &db, true).await;
    assert!(r.ran);
    assert_eq!(r.adopted, vec![NAME.to_string()]);
    assert!(r.done);
}

/// A node still syncing is not authoritative: nothing runs past the tip
/// read, and the search is not done.
#[tokio::test]
async fn a_node_still_syncing_runs_nothing() {
    let db = store();
    let node = node_with(vec![our_lock_transfer()]).with_blockchain_info(BlockchainInfo {
        blocks: 100,
        headers: Some(QUIET_TIP),
        verification_progress: Some(0.1),
        ..Default::default()
    });
    let r = find(&node, &db, false).await;
    assert!(!r.ran && r.adopted.is_empty() && !r.done);
    assert_eq!(node.calls(), vec![RpcCall::BlockchainInfo]);
    assert!(!done(&db));
}

/// A node without the address index runs no search: refused with the
/// sentence My listings shows, nothing done.
#[tokio::test]
async fn no_address_index_is_refused_with_its_sentence() {
    let db = store();
    let node = node_with(vec![]).with_txs_by_address_err(
        "address index not enabled on this hsd node: Address indexing not enabled. (status 400)",
    );
    let e = find_locks_with(&node, &db, PROFILE, NET, false, &derive)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, AppError::InvalidInput(m) if m == LOCK_SEARCH_NEEDS_INDEX),
        "{e:?}"
    );
    assert!(!done(&db));
}

/// The status line reads the history from the xpub alone (no seed is
/// passed): candidates are counted once each across addresses; a done search
/// reads nothing.
#[tokio::test]
async fn status_counts_candidates_without_the_seed() {
    let db = store();
    add_change_address(&db);
    let other = transfer_tx(&addr00(NET).0, &[0x42; 20]);
    let node = node_with(vec![our_lock_transfer(), other]);
    let s = lock_search_status_with(&node, &db, PROFILE, NET)
        .await
        .unwrap();
    assert!(!s.done && s.address_index);
    assert_eq!(s.candidates, 2);
    assert_eq!(
        node.count_matching(|c| matches!(c, RpcCall::TxsByAddress(_))),
        2
    );
    queries::mark_lock_search_done(&db.lock().unwrap(), PROFILE).unwrap();
    let node = MockNodeRpc::new();
    let s = lock_search_status_with(&node, &db, PROFILE, NET)
        .await
        .unwrap();
    assert!(s.done && s.candidates == 0);
    assert_eq!(node.call_count(), 0);
}

/// Without the address index the status says so, for the line naming the
/// node flags.
#[tokio::test]
async fn status_says_when_the_index_is_missing() {
    let db = store();
    for (tx, address) in [(false, true), (true, false), (false, false)] {
        let node = node_with(vec![our_lock_transfer()]).with_node_status(node_status(tx, address));
        let s = lock_search_status_with(&node, &db, PROFILE, NET)
            .await
            .unwrap();
        assert!(!s.done && !s.address_index, "{tx} {address}");
        assert_eq!(s.candidates, 0);
        assert_eq!(
            node.count_matching(|c| matches!(c, RpcCall::TxsByAddress(_))),
            0
        );
    }
    let node = MockNodeRpc::new()
        .with_node_status(node_status(true, true))
        .with_txs_by_address_err("address index not enabled on this hsd node: x (status 400)");
    let s = lock_search_status_with(&node, &db, PROFILE, NET)
        .await
        .unwrap();
    assert!(!s.done && !s.address_index);
    assert_eq!(s.candidates, 0);
}

/// A history read that fails for any other reason than the missing index is
/// an error for the status line, never a count of zero candidates.
#[tokio::test]
async fn status_errs_on_a_history_read_failing_for_another_reason() {
    let db = store();
    let node =
        node_with(vec![]).with_txs_by_address_fn(|_| Err(AppError::Rpc("connection reset".into())));
    let e = lock_search_status_with(&node, &db, PROFILE, NET)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, AppError::Rpc(m) if m == "connection reset"),
        "{e:?}"
    );
}

/// hsd answers `GET /tx/address` with an empty list, not an error, when an
/// index is off: the indexes are read from `GET /` first, and a node without
/// them is refused with the sentence, the profile left not done.
#[tokio::test]
async fn a_node_without_the_indexes_is_refused_though_its_histories_read_empty() {
    let db = store();
    for (tx, address) in [(false, true), (true, false), (false, false)] {
        let node = node_with(vec![]).with_node_status(node_status(tx, address));
        let e = find_locks_with(&node, &db, PROFILE, NET, false, &derive)
            .await
            .unwrap_err();
        assert!(
            matches!(&e, AppError::InvalidInput(m) if m == LOCK_SEARCH_NEEDS_INDEX),
            "{tx} {address}: {e:?}"
        );
        assert_eq!(
            node.count_matching(|c| matches!(c, RpcCall::TxsByAddress(_))),
            0
        );
        assert!(!done(&db));
    }
}

/// A node status without its indexes (missing, or not booleans) is not
/// "enabled": an error, the search not done, and the status line errs.
#[tokio::test]
async fn a_node_status_without_the_indexers_is_an_error() {
    let db = store();
    for reply in [
        json!({ "chain": { "height": QUIET_TIP } }),
        json!({ "chain": { "indexers": { "indexTX": true, "indexAddress": "yes" } } }),
        json!({ "chain": { "indexers": { "indexAddress": true } } }),
    ] {
        let node = node_with(vec![our_lock_transfer()]).with_node_status(reply.clone());
        let e = find_locks_with(&node, &db, PROFILE, NET, false, &derive)
            .await
            .unwrap_err();
        assert!(!matches!(e, AppError::InvalidInput(_)), "{reply}: {e:?}");
        assert!(!done(&db), "{reply}");
        assert!(lock_search_status_with(&node, &db, PROFILE, NET)
            .await
            .is_err());
        assert_eq!(listings(&db), 0);
    }
}

/// A lock key that cannot be derived (the signer locked, or switched to
/// another profile, mid-search) judges nothing: only R32's checks do. The
/// candidate stays unjudged and the search not done.
#[tokio::test]
async fn a_failed_key_derivation_leaves_the_candidate_unjudged() {
    let db = store();
    let node = node_with(vec![our_lock_transfer()]);
    let refuse = |_: &str| -> Result<LockKey, AppError> {
        Err(AppError::InvalidInput(
            "the unlocked signer is for a different wallet profile".into(),
        ))
    };
    let r = find_locks_with(&node, &db, PROFILE, NET, false, &refuse)
        .await
        .unwrap();
    assert!(r.ran && r.adopted.is_empty());
    assert!(!r.done && !done(&db));
    assert_eq!(listings(&db), 0);
}

// --- the commands' gates ------------------------------------------------------

async fn fixture(kind: &str) -> (mockito::ServerGuard, Vec<mockito::Mock>, App) {
    restore_fixture_of(kind, restored_info(), Some(our_lock_coin())).await
}

/// R34: deriving the lock key needs the recovery phrase: a locked wallet is
/// refused before the node is read.
#[tokio::test]
async fn find_locks_needs_the_unlocked_signer() {
    let (_node, mocks, app) = fixture("mnemonic_hot").await;
    *app.state::<AppState>().signer.lock().unwrap() = None;
    assert!(matches!(
        shakedex_find_locks(app.state(), true).await.unwrap_err(),
        AppError::WalletLocked
    ));
    assert!(!mocks[0].matched_async().await, "the node was not read");
    assert_eq!(count(&app, "shakedex_listings"), 0);
}

/// R16/R34: Ledger, watch-only and extended-private-key profiles have no
/// seed to derive the lock key from: the search is refused with the sentence
/// the UI shows, before the node is read, and the status offers nothing.
#[tokio::test]
async fn find_locks_refused_for_ledger_and_watch_only() {
    let sentence = crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY;
    for kind in ["ledger_hardware", "xpriv_hot", "watch_only_xpub"] {
        let (_node, mocks, app) = fixture(kind).await;
        let e = err_text(shakedex_find_locks(app.state(), true).await.unwrap_err());
        assert!(e.contains(sentence), "{kind}: {e}");
        let s = shakedex_lock_search_status(app.state()).await.unwrap();
        assert!(
            s.done && s.address_index && s.candidates == 0,
            "{kind}: nothing to offer"
        );
        assert!(
            !mocks[0].matched_async().await,
            "{kind}: the node was not read"
        );
        assert_eq!(count(&app, "shakedex_listings"), 0, "{kind}");
    }
}
