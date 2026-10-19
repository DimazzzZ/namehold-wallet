//! The market jobs (T6): R23's publishing, R25's keeping listed, R28's
//! reports. The node is `MockNodeRpc`; LearnHNS Market is a mockito server
//! answering in the market's own shapes (`learnhns_tests`' constants, copied
//! from its source; no live write is ever recorded).

use mockito::{Matcher, ServerGuard};
use rusqlite::Connection;
use serde_json::json;

use crate::db::queries::{self, ListingMode, ListingState, MarketStatus, ShakedexListing};
use crate::db::sync_lock::LockLost;
use crate::market::learnhns::LearnHnsClient;
use crate::noncustodial::derivation;
use crate::noncustodial::hd::ExtendedPrivKey;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{BlockchainInfo, NodeCoin};
use crate::noncustodial::shakedex::listing_file::{
    write_listing_file, ListingFile, NewListingFile, PriceStep,
};
use crate::noncustodial::shakedex::lock_key::{derive_lock_key, LockKey};
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::sell::{self, StoredStep};
use crate::noncustodial::shakedex::template::{valid_from_mtp, StepTemplate};
use crate::noncustodial::sync::COV_FINALIZE;
use crate::noncustodial::tx::output_address_from_string;
use crate::shakedex_jobs::{
    advance_reverse_auctions_with_client, keep_listed_with_client, publish_listings_with_client,
};
use crate::tests::learnhns_tests::{
    proof_part, recording, Seen, CANCEL_RECORDED, COIN_NOT_SEEN, NO_LISTING, PENDING_ACCEPTED,
    PROOF_NOT_FOUND, SALE_RECORDED, TX_NOT_SEEN, UPLOAD_ACCEPTED,
};
use crate::tests::mock_node_rpc::{MockNodeRpc, RpcCall};
use crate::tests::shakedex_cmd_tests::{seed, seeded, PROFILE};

const NET: Network = Network::Main;
const NAME: &str = "dexjobs";
const LOCK_VALUE: u64 = 1_000_000;
const PRICE: u64 = 5_000_000;
const MTP: u64 = 1_790_000_000;
/// The job's wall clock: ten minutes past the node's median time.
const NOW: i64 = 1_790_000_600;
const TIP: i64 = 400_000;

fn txid(b: &str) -> String {
    b.repeat(32)
}

struct Fx {
    conn: Connection,
    id: String,
    key: LockKey,
    lock: String,
    payment: String,
}

/// A mainnet profile with one listing of NAME in `state`, `publish` as
/// given. From Finalizing on it has its lock coin `(f1…, 0)` at the lock
/// address of the seed's key for NAME, one Buy Now step at PRICE signed over
/// that coin worth LOCK_VALUE (lock time MTP − 512) paying a reserved
/// address of ours, and the listing file Finalize & sign writes (expiry MTP
/// + 365 days). Its lock TRANSFER `e1…` was sent (`lockd`, broadcasted).
fn fx(state: ListingState, publish: bool) -> Fx {
    fx_at(state, publish, MTP)
}

/// [`fx`] at the node's median time `mtp`: the step's lock time and the
/// listing's expiry follow it.
fn fx_at(state: ListingState, publish: bool, mtp: u64) -> Fx {
    let conn = seeded("mainnet", "mnemonic_hot", "http://127.0.0.1:9");
    let key = derive_lock_key(&ExtendedPrivKey::from_seed(&seed()).unwrap(), NET, 0, NAME).unwrap();
    let lock = script::lock_address(NET, &key.pubkey).unwrap();
    let payment = derivation::reserve_receive_address(&conn, PROFILE)
        .unwrap()
        .address;
    queries::insert_tx_draft(&conn, "lockd", PROFILE, sell::LOCK_ACTION, "00", "{}", "{}").unwrap();
    queries::update_tx_draft_status(&conn, "lockd", "broadcasted", None, Some(&txid("e1")))
        .unwrap();
    let locked = !matches!(state, ListingState::Locking | ListingState::ReadyToFinalize);
    let (steps_json, file) = if locked {
        let step = signed_step(&key, &payment, PRICE, sell::buy_now_lock_time(mtp));
        (
            stored(std::slice::from_ref(&step)),
            Some(file_of(
                &key,
                &payment,
                &[step],
                mtp + sell::LISTING_LIFETIME_SECS,
            )),
        )
    } else {
        ("[]".to_string(), None)
    };
    let l = ShakedexListing {
        id: "l1".into(),
        wallet_profile_id: PROFILE.into(),
        name: NAME.into(),
        mode: ListingMode::BuyNow,
        state,
        lock_pubkey_hex: hex::encode(key.pubkey),
        lock_transfer_draft_id: Some("lockd".into()),
        lock_finalize_draft_id: None,
        lock_transfer_txid: Some(txid("e1")),
        lock_txid: locked.then(|| txid("f1")),
        lock_vout: locked.then_some(0),
        payment_address: Some(payment.clone()),
        cancel_address: None,
        cancel_child_index: None,
        steps_json,
        listing_file_json: file,
        publish,
        market_status: None,
        market_retry_at: None,
        market_attempts: 0,
        market_error: None,
        market_accepted: false,
        market_changed: false,
        market_told: false,
        market_sent_step: None,
        expires_at: locked.then_some((mtp + sell::LISTING_LIFETIME_SECS) as i64),
        abort_draft_id: None,
        abort_txid: None,
        sold_txid: None,
        cancel_txid: None,
        cancel_draft_id: None,
        cancel_vout: None,
        cancel_finalize_draft_id: None,
        cancel_blocks_remaining: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    queries::insert_shakedex_listing(&conn, &l).unwrap();
    Fx {
        conn,
        id: "l1".into(),
        key,
        lock,
        payment,
    }
}

fn signed_step(key: &LockKey, payment: &str, price: u64, lock_time: u64) -> PriceStep {
    let mut lock_txid = [0u8; 32];
    hex::decode_to_slice(txid("f1"), &mut lock_txid).unwrap();
    let t = StepTemplate {
        lock_outpoint: (lock_txid, 0),
        lock_value: LOCK_VALUE,
        lock_pubkey: &key.pubkey,
        payment: output_address_from_string(NET, payment).unwrap(),
        price,
        lock_time_secs: lock_time,
    };
    PriceStep {
        price,
        lock_time,
        signature: sell::sign_step(key, &t).unwrap(),
        fee: 0,
    }
}

fn stored(steps: &[PriceStep]) -> String {
    serde_json::to_string(
        &steps
            .iter()
            .map(|s| StoredStep {
                price: s.price,
                lock_time: s.lock_time,
                signature: hex::encode(s.signature),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn file_of(key: &LockKey, payment: &str, steps: &[PriceStep], expires_at: u64) -> String {
    let mut lock_txid = [0u8; 32];
    hex::decode_to_slice(txid("f1"), &mut lock_txid).unwrap();
    write_listing_file(
        &NewListingFile {
            name: NAME,
            lock_txid,
            lock_vout: 0,
            public_key: key.pubkey,
            payment_addr: payment,
            steps,
            expires_at,
        },
        NET,
    )
    .unwrap()
}

fn listing(f: &Fx) -> ShakedexListing {
    queries::get_shakedex_listing(&f.conn, &f.id)
        .unwrap()
        .unwrap()
}

/// hsd as it answers once the FINALIZE into the lock is mined: the lock coin
/// a FINALIZE of NAME at the lock address, worth LOCK_VALUE, at `height`;
/// the tip and its median time MTP.
fn node(f: &Fx, height: i64) -> MockNodeRpc {
    node_at(f, height, MTP)
}

/// hsd as [`node`], its median time `mtp`.
fn node_at(f: &Fx, height: i64, mtp: u64) -> MockNodeRpc {
    let mut info = tip();
    info.mediantime = Some(mtp);
    let coin: NodeCoin = serde_json::from_value(json!({
        "version": 0, "height": height, "value": LOCK_VALUE, "address": f.lock,
        "covenant": { "type": COV_FINALIZE, "action": "FINALIZE",
                      "items": [hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap()), "32000000"] },
        "coinbase": false, "hash": txid("f1"), "index": 0
    }))
    .unwrap();
    MockNodeRpc::new()
        .with_blockchain_info(info)
        .with_get_coin(move |t, i| Ok((t == txid("f1") && i == 0).then(|| coin.clone())))
}

fn tip() -> BlockchainInfo {
    serde_json::from_value(json!({ "chain": "main", "blocks": TIP, "headers": TIP,
                                   "verificationprogress": 1.0, "mediantime": MTP }))
    .unwrap()
}

async fn market() -> ServerGuard {
    mockito::Server::new_async().await
}

fn client(s: &ServerGuard) -> LearnHnsClient {
    LearnHnsClient::with_base_url(&s.url())
        .unwrap()
        .for_network(NET)
}

async fn publish(f: &Fx, node: &MockNodeRpc, s: &ServerGuard, now: i64) {
    publish_listings_with_client(
        &f.conn,
        node,
        &client(s),
        PROFILE,
        now,
        &LockLost::default(),
    )
    .await
    .unwrap();
}

/// The market's upload acceptance (`UPLOAD_ACCEPTED`, MKT's shape) for
/// NAME: the market answers with the name it listed.
fn upload_accepted() -> &'static str {
    Box::leak(
        UPLOAD_ACCEPTED
            .replace("\"dexreviews\"", &format!("\"{NAME}\""))
            .into_boxed_str(),
    )
}

async fn upload_mock(s: &mut ServerGuard, hits: usize) -> (mockito::Mock, Seen) {
    let (m, seen) = recording(s.mock("POST", "/api/upload-proof"), 201, upload_accepted());
    (m.expect(hits).create_async().await, seen)
}

/// R23 day 0: once the lock TRANSFER is sent, a published listing is
/// announced as pending — its name, the lock TRANSFER's outpoint, the lock
/// address and its mode — and is not announced again. An unsent lock
/// TRANSFER announces nothing.
#[tokio::test]
async fn publishes_pending_on_day_zero() {
    let f = fx(ListingState::Locking, true);
    let mut s = market().await;
    let (m, seen) = recording(
        s.mock("POST", "/api/v2/pending-listings"),
        201,
        PENDING_ACCEPTED,
    );
    let m = m.expect(1).create_async().await;
    let n = MockNodeRpc::new();
    // The day-0 draft not sent yet: nothing.
    queries::update_tx_draft_status(&f.conn, "lockd", "signed", None, None).unwrap();
    publish(&f, &n, &s, NOW).await;
    assert!(seen.lock().unwrap().is_empty());
    queries::update_tx_draft_status(&f.conn, "lockd", "broadcasted", None, Some(&txid("e1")))
        .unwrap();
    publish(&f, &n, &s, NOW).await;
    publish(&f, &n, &s, NOW + 7_200).await;
    m.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(
        sent,
        json!({ "name": NAME, "network": "main", "transferTxHash": txid("e1"),
                "transferOutputIdx": 0, "lockScriptAddr": f.lock, "listingMode": "fixed-price" })
    );
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Pending));
    assert_eq!(n.call_count(), 0, "day 0 needs no node read");
}

/// Ruling 2026-10-10: the market's own refusal of the pending post (its
/// JSON 409) is kept with its words and not posted again; no answer (a 502
/// page) is retried after the backoff.
#[tokio::test]
async fn refused_pending_post_is_not_retried() {
    let f = fx(ListingState::Locking, true);
    let mut s = market().await;
    let down = s
        .mock("POST", "/api/v2/pending-listings")
        .with_status(502)
        .with_body("<html>bad gateway</html>")
        .expect(1)
        .create_async()
        .await;
    let n = MockNodeRpc::new();
    publish(&f, &n, &s, NOW).await;
    publish(&f, &n, &s, NOW + 299).await; // not due
    down.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_attempts),
        (Some(MarketStatus::Retrying), 1)
    );
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 300).as_str())
    );
    drop(down);
    let refuse = s
        .mock("POST", "/api/v2/pending-listings")
        .with_status(409)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"A pending listing with that transferTxHash already exists"}"#)
        .expect(1)
        .create_async()
        .await;
    for at in [NOW + 300, NOW + 86_400, NOW + 7 * 86_400] {
        publish(&f, &n, &s, at).await;
    }
    refuse.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at),
        (Some(MarketStatus::Refused), None)
    );
    assert!(l.market_error.unwrap().contains("already exists"));
}

/// R23: nothing is uploaded while the FINALIZE into the lock is unmined
/// (Finalizing); once it is mined (Listed, one confirmation) the current
/// step is uploaded, as a one-step copy of the stored file, after every
/// stored step verified over the lock coin hsd reports.
#[tokio::test]
async fn publishes_active_after_one_confirmation() {
    let f = fx(ListingState::Finalizing, true);
    let mut s = market().await;
    // Announced on day 0 while Finalizing (Pending); the move to Listed
    // starts the bookkeeping over, and the upload follows.
    let (pending, _) = recording(
        s.mock("POST", "/api/v2/pending-listings"),
        201,
        PENDING_ACCEPTED,
    );
    let _pending = pending.create_async().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    let n = node(&f, TIP);
    publish(&f, &n, &s, NOW).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "Finalizing: nothing uploaded"
    );
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Pending));
    assert_eq!(queries::mark_listing_listed(&f.conn, &f.id).unwrap(), 1);
    publish(&f, &n, &s, NOW).await;
    publish(&f, &n, &s, NOW + 60).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    let up = ListingFile::parse(&sent, NET).unwrap();
    let stored =
        ListingFile::parse(listing(&f).listing_file_json.as_deref().unwrap(), NET).unwrap();
    assert_eq!(up.steps, stored.steps);
    assert_eq!(up.expires_at, stored.expires_at);
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 3_600).as_str())
    );
    assert!(n.count_matching(|c| matches!(c, RpcCall::GetCoin(t, 0) if *t == txid("f1"))) >= 1);
    // SECURITY.md: the job only reads the node — the tip and the lock coin.
    assert_eq!(
        n.count_matching(|c| !matches!(c, RpcCall::BlockchainInfo | RpcCall::GetCoin(..))),
        0
    );
}

/// R23: with the box off, nothing reaches the market — no pending listing,
/// no upload — and the node is not read for it; off mainnet neither
/// (carry 5).
#[tokio::test]
async fn nothing_published_when_the_box_is_off() {
    for state in [ListingState::Locking, ListingState::Listed] {
        let f = fx(state, false);
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
        let n = node(&f, TIP);
        publish(&f, &n, &s, NOW).await;
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
            .await
            .unwrap();
        any.assert_async().await;
        get.assert_async().await;
        assert_eq!(listing(&f).market_status, None);
        assert_eq!(n.call_count(), 0);
    }
}

/// R23: only a listing the user opted in to publish (`publish`) that is
/// Listed is uploaded: a Listed one with the box off, and published ones
/// Sold, Restored, SalePending or Cancelling (its cancel built, not sent),
/// never reach the market, which sees no request at all.
#[tokio::test]
async fn only_published_listed_listings_are_uploaded() {
    let cases: [(ListingState, bool); 5] = [
        (ListingState::Listed, false),
        (ListingState::Sold, true),
        (ListingState::Restored, true),
        (ListingState::SalePending, true),
        (ListingState::Cancelling, true),
    ];
    for (state, publish_box) in cases {
        let f = fx(ListingState::Listed, publish_box);
        queries::insert_tx_draft(
            &f.conn,
            "canceld",
            PROFILE,
            "shakedex_cancel",
            "00",
            "{}",
            "{}",
        )
        .unwrap();
        f.conn
            .execute(
                "UPDATE shakedex_listings SET state = ?1, cancel_draft_id = 'canceld'",
                [state],
            )
            .unwrap();
        if state == ListingState::Cancelling {
            // With the acceptance flag set it is in the jobs' set; the
            // publish job's first upload still takes a Listed one only.
            f.conn
                .execute("UPDATE shakedex_listings SET market_accepted = 1", [])
                .unwrap();
            let kept = queries::list_listings_kept_on_market(&f.conn, PROFILE).unwrap();
            assert_eq!(kept.len(), 1, "the unsent cancel keeps it in the jobs' set");
        }
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
        let n = node(&f, TIP);
        publish(&f, &n, &s, NOW).await;
        any.assert_async().await;
        get.assert_async().await;
        assert_eq!(listing(&f).market_status, None, "{state:?}");
        assert_eq!(n.call_count(), 0, "{state:?}");
    }
}

/// Carry 5: a regtest profile's published listing (only reachable by
/// editing the row: Lock refuses `publish` off mainnet) makes no HTTP, even
/// with a client told mainnet: the job's own network gate.
#[tokio::test]
async fn nothing_published_off_mainnet() {
    for state in [ListingState::Locking, ListingState::Listed] {
        let f = fx(state, true);
        f.conn
            .execute(
                "UPDATE wallet_profiles SET network = 'regtest' WHERE id = ?1",
                [PROFILE],
            )
            .unwrap();
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
        let n = node(&f, TIP);
        publish(&f, &n, &s, NOW).await;
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
            .await
            .unwrap();
        any.assert_async().await;
        get.assert_async().await;
        assert_eq!(listing(&f).market_status, None);
    }
    // R28 (T6): a sale the market was told about is not reported off
    // mainnet either.
    let f = listed_on_market(MarketStatus::Listed);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
            [txid("b1")],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE wallet_profiles SET network = 'regtest' WHERE id = ?1",
            [PROFILE],
        )
        .unwrap();
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    any.assert_async().await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Listed));
}

/// Carried from T4: a step stored unverified (imported while the lock coin
/// was spent, R32) or tampered with is re-verified over the lock coin hsd
/// reports before any upload; one that does not verify stops the upload and
/// the listing says why.
#[tokio::test]
async fn steps_not_signed_by_the_lock_are_not_uploaded() {
    let f = fx(ListingState::Listed, true);
    let other = signed_step(&f.key, &f.payment, PRICE, sell::buy_now_lock_time(MTP));
    let mut bad = other.clone();
    bad.price += 1;
    let file = file_of(
        &f.key,
        &f.payment,
        std::slice::from_ref(&bad),
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    f.conn
        .execute(
            "UPDATE shakedex_listings SET steps_json = ?1, listing_file_json = ?2",
            [stored(&[bad]), file],
        )
        .unwrap();
    let mut s = market().await;
    let (m, _) = upload_mock(&mut s, 0).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::StepsUnverified));
    assert!(l
        .market_error
        .unwrap()
        .contains(sell::STEP_NOT_SIGNED_BY_LOCK));
}

/// Carried from T4: the steps the row stores are the ones verified: a
/// stored step the listing file does not hold (the file is not the
/// listing's own) stops the upload too.
#[tokio::test]
async fn stored_steps_not_in_the_file_are_not_uploaded() {
    let f = fx(ListingState::Listed, true);
    let other = signed_step(&f.key, &f.payment, PRICE + 1, sell::buy_now_lock_time(MTP));
    f.conn
        .execute(
            "UPDATE shakedex_listings SET steps_json = ?1",
            [stored(&[other])],
        )
        .unwrap();
    let mut s = market().await;
    let (m, _) = upload_mock(&mut s, 0).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    assert_eq!(
        listing(&f).market_status,
        Some(MarketStatus::StepsUnverified)
    );
}

/// Fix round 1: a row that does not read — its stored steps, lock key or
/// lock output index — is no node question: it is recorded once as
/// StepsUnverified with why (for the UI, retried with backoff), nothing is
/// uploaded and the node is not read.
#[tokio::test]
async fn unreadable_row_is_recorded_as_steps_unverified() {
    for (column, value, why) in [
        ("steps_json", "not json", "unreadable steps"),
        ("lock_pubkey_hex", "zz", "bad lock key"),
        ("lock_vout", "-1", "bad lock output"),
    ] {
        let f = fx(ListingState::Listed, true);
        f.conn
            .execute(
                &format!("UPDATE shakedex_listings SET {column} = ?1"),
                [value],
            )
            .unwrap();
        let mut s = market().await;
        let (m, _) = upload_mock(&mut s, 0).await;
        let n = node(&f, TIP);
        publish(&f, &n, &s, NOW).await;
        m.assert_async().await;
        let l = listing(&f);
        assert_eq!(
            (l.market_status, l.market_attempts),
            (Some(MarketStatus::StepsUnverified), 1),
            "{column}"
        );
        assert_eq!(
            l.market_retry_at.as_deref(),
            Some(rfc3339(NOW + 300).as_str())
        );
        assert!(l.market_error.unwrap().contains(why), "{column}");
        assert_eq!(n.call_count(), 0, "{column}: the node is not read");
    }
}

/// A1 (code-review round): a row the day-0 post cannot be built from — no
/// lock TRANSFER txid, or a lock key that does not read — is recorded as
/// StepsUnverified with why and backed off, nothing posted; it is tried
/// again after the backoff, not every sync.
#[tokio::test]
async fn unreadable_row_is_not_announced_and_is_recorded() {
    for (state, sql, why) in [
        (
            ListingState::ReadyToFinalize,
            "UPDATE shakedex_listings SET lock_transfer_txid = NULL",
            "no lock TRANSFER txid",
        ),
        (
            ListingState::Locking,
            "UPDATE shakedex_listings SET lock_pubkey_hex = 'zz'",
            "bad lock key",
        ),
    ] {
        let f = fx(state, true);
        f.conn.execute(sql, []).unwrap();
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        publish(&f, &MockNodeRpc::new(), &s, NOW).await;
        publish(&f, &MockNodeRpc::new(), &s, NOW + 299).await;
        let l = listing(&f);
        assert_eq!(
            (
                l.market_status,
                l.market_attempts,
                l.market_retry_at.as_deref()
            ),
            (
                Some(MarketStatus::StepsUnverified),
                1,
                Some(rfc3339(NOW + 300).as_str())
            ),
            "{why}"
        );
        assert!(l.market_error.unwrap().contains(why), "{why}");
        publish(&f, &MockNodeRpc::new(), &s, NOW + 300).await;
        assert_eq!(
            listing(&f).market_attempts,
            2,
            "{why}: tried after the backoff"
        );
        any.assert_async().await;
    }
}

/// A1: a report whose row lacks the mined txid it carries is recorded as
/// StepsUnverified with why and backed off; nothing is sent.
#[tokio::test]
async fn report_of_a_row_without_its_txid_is_recorded() {
    for (sql, why) in [
        (
            "UPDATE shakedex_listings SET state = 'sold', sold_txid = NULL",
            "no sale txid",
        ),
        (
            "UPDATE shakedex_listings SET state = 'cancelled', cancel_txid = NULL",
            "no cancel txid",
        ),
    ] {
        let f = listed_on_market(MarketStatus::Listed);
        f.conn.execute(sql, []).unwrap();
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        publish(&f, &node(&f, TIP), &s, NOW).await;
        publish(&f, &node(&f, TIP), &s, NOW + 299).await;
        any.assert_async().await;
        let l = listing(&f);
        assert_eq!(
            (
                l.market_status,
                l.market_attempts,
                l.market_retry_at.as_deref()
            ),
            (
                Some(MarketStatus::StepsUnverified),
                1,
                Some(rfc3339(NOW + 300).as_str())
            ),
            "{why}"
        );
        assert!(l.market_error.unwrap().contains(why), "{why}");
    }
}

/// A1: a stored file that reads but cannot be rewritten with its refreshed
/// expiry (here: the rewrite would pass the listing-file size limit) is the
/// row's problem, not the node's: recorded as StepsUnverified with why,
/// backed off, nothing uploaded and the stored file left as it is.
#[tokio::test]
async fn file_that_cannot_be_rewritten_is_recorded_as_steps_unverified() {
    use crate::noncustodial::shakedex::listing_file::MAX_LISTING_FILE_BYTES;
    let f = fx(ListingState::Listed, true);
    let mut v: serde_json::Value =
        serde_json::from_str(&listing(&f).listing_file_json.unwrap()).unwrap();
    // An expiry long past (one digit): the refresh writes ten.
    v["expiresAt"] = 1.into();
    v["pad"] = "".into();
    let bare = serde_json::to_string(&v).unwrap().len();
    v["pad"] = "x".repeat(MAX_LISTING_FILE_BYTES - 5 - bare).into();
    let file = serde_json::to_string(&v).unwrap();
    ListingFile::parse(&file, NET).expect("the stored file reads");
    f.conn
        .execute(
            "UPDATE shakedex_listings SET listing_file_json = ?1, expires_at = 1",
            [&file],
        )
        .unwrap();
    let mut s = market().await;
    let (m, _) = upload_mock(&mut s, 0).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (
            l.market_status,
            l.market_attempts,
            l.market_retry_at.as_deref()
        ),
        (
            Some(MarketStatus::StepsUnverified),
            1,
            Some(rfc3339(NOW + 300).as_str())
        )
    );
    assert!(l.market_error.unwrap().contains("larger than"));
    assert_eq!(l.listing_file_json.as_deref(), Some(file.as_str()));
}

/// A1: a Refused Listed listing near its expiry whose row does not read is
/// recorded as StepsUnverified with why and backed off, with no market
/// call.
#[tokio::test]
async fn refused_listing_whose_row_does_not_read_is_recorded() {
    let f = listed_on_market(MarketStatus::Refused);
    f.conn
        .execute("UPDATE shakedex_listings SET market_retry_at = NULL", [])
        .unwrap();
    set_expiry(&f, (NOW + 86_400) as u64);
    f.conn
        .execute("UPDATE shakedex_listings SET steps_json = 'not json'", [])
        .unwrap();
    let mut s = market().await;
    let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    keep(&f, &s, NOW).await;
    keep(&f, &s, NOW + 299).await;
    get.assert_async().await;
    post.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (
            l.market_status,
            l.market_attempts,
            l.market_retry_at.as_deref()
        ),
        (
            Some(MarketStatus::StepsUnverified),
            1,
            Some(rfc3339(NOW + 300).as_str())
        )
    );
    assert!(l.market_error.unwrap().contains("unreadable steps"));
}

/// R23 with R26 (T6): a lowered Buy Now's file holds both steps; the market
/// takes one, so only the current step (the cheapest valid at the node's
/// median time) is uploaded.
#[tokio::test]
async fn lowered_buy_now_uploads_only_its_current_step() {
    let f = fx(ListingState::Listed, true);
    let first = signed_step(
        &f.key,
        &f.payment,
        PRICE,
        sell::buy_now_lock_time(MTP - 3_600),
    );
    let lower = signed_step(&f.key, &f.payment, 3_000_000, sell::buy_now_lock_time(MTP));
    let file = file_of(
        &f.key,
        &f.payment,
        &[first.clone(), lower.clone()],
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    f.conn
        .execute(
            "UPDATE shakedex_listings SET steps_json = ?1, listing_file_json = ?2",
            [stored(&[first, lower.clone()]), file],
        )
        .unwrap();
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(ListingFile::parse(&sent, NET).unwrap().steps, vec![lower]);
}

/// The market's "listed" names the listing it took: an acceptance naming
/// another name is not an answer about ours (backed off, retried).
#[tokio::test]
async fn upload_accepted_for_another_name_is_no_answer() {
    let f = fx(ListingState::Listed, true);
    let mut s = market().await;
    let (m, _) = recording(s.mock("POST", "/api/upload-proof"), 201, UPLOAD_ACCEPTED);
    let m = m.expect(1).create_async().await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_attempts),
        (Some(MarketStatus::Retrying), 1)
    );
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 300).as_str())
    );
}

/// Fix round 1 and the Step 3 re-review: the market's "not seen yet" (here
/// its 404 "Listing coin was not found or is already spent", which can also
/// mean spent) is never a verdict on the listing: its state, steps and file
/// stay, it only backs off like no answer, with the market's words.
#[tokio::test]
async fn not_seen_yet_upload_only_backs_off() {
    let f = fx(ListingState::Listed, true);
    let before = listing(&f);
    let mut s = market().await;
    let m = s
        .mock("POST", "/api/upload-proof")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(COIN_NOT_SEEN)
        .expect(1)
        .create_async()
        .await;
    let n = node(&f, TIP);
    publish(&f, &n, &s, NOW).await;
    publish(&f, &n, &s, NOW + 60).await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(l.state, ListingState::Listed);
    assert_eq!(
        (&l.steps_json, &l.listing_file_json, l.expires_at),
        (
            &before.steps_json,
            &before.listing_file_json,
            before.expires_at
        )
    );
    assert_eq!(
        (l.market_status, l.market_attempts),
        (Some(MarketStatus::Retrying), 1)
    );
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 300).as_str())
    );
    assert!(l
        .market_error
        .unwrap()
        .contains("not found or is already spent"));
}

/// R23: a Listed listing whose `expiresAt` is within the margin of now gets
/// the node's median time plus a year before it is uploaded — but only when
/// that is later than the stored one: a node whose median time lags the
/// wall clock never moves the expiry back.
#[tokio::test]
async fn expiry_refreshed_only_near_its_end_and_only_later() {
    // Near its end: refreshed to MTP + a year, and that copy uploaded.
    let f = fx(ListingState::Listed, true);
    let soon = MTP + 86_400;
    set_expiry(&f, soon);
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    let want = MTP + sell::LISTING_LIFETIME_SECS;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().expires_at,
        Some(want)
    );
    let l = listing(&f);
    assert_eq!(l.expires_at, Some(want as i64));
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    drop(m);

    // Far from its end (a day short of MTP + a year, which would be later):
    // as stored.
    let g = fx(ListingState::Listed, true);
    let far = want - 86_400;
    set_expiry(&g, far);
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&g, &node(&g, TIP), &s, NOW).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().expires_at,
        Some(far)
    );
    assert_eq!(listing(&g).expires_at, Some(far as i64));

    // Within the margin of a wall clock far ahead of the node's median time,
    // but MTP + a year is earlier than the stored expiry: never moved back.
    let h = fx(ListingState::Listed, true);
    let later = want + 86_400;
    set_expiry(&h, later);
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&h, &node(&h, TIP), &s, want as i64).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().expires_at,
        Some(later)
    );
    assert_eq!(listing(&h).expires_at, Some(later as i64));
}

/// `f`'s stored file and `expires_at` moved to `at`.
fn set_expiry(f: &Fx, at: u64) {
    let file = crate::noncustodial::shakedex::listing_file::with_expiry(
        listing(f).listing_file_json.as_deref().unwrap(),
        at,
        NET,
    )
    .unwrap();
    f.conn
        .execute(
            "UPDATE shakedex_listings SET listing_file_json = ?1, expires_at = ?2",
            rusqlite::params![file, at as i64],
        )
        .unwrap();
}

/// R23 day 0: a reverse auction gets its pending post with
/// `listingMode: "reverse-auction"`.
#[tokio::test]
async fn reverse_auction_gets_its_pending_post() {
    let f = fx(ListingState::Locking, true);
    f.conn
        .execute("UPDATE shakedex_listings SET mode = 'reverse_auction'", [])
        .unwrap();
    let mut s = market().await;
    let (m, seen) = recording(
        s.mock("POST", "/api/v2/pending-listings"),
        201,
        PENDING_ACCEPTED,
    );
    let m = m.expect(1).create_async().await;
    publish(&f, &MockNodeRpc::new(), &s, NOW).await;
    m.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(sent["listingMode"], "reverse-auction");
}

/// R24: five steps 1024 s apart (at least 15 minutes), 10 → 2 HNS; at MTP
/// steps 0–2 are valid (step 2 since MTP − 512) and steps 3–4 are not (step
/// 3's lock time is MTP + 512, so its 512-s rounding stays above MTP).
fn schedule() -> Vec<(u64, u64)> {
    schedule_at(MTP)
}

/// [`schedule`] around the node's median time `mtp`: step 2 is current.
fn schedule_at(mtp: u64) -> Vec<(u64, u64)> {
    let l2 = sell::buy_now_lock_time(mtp);
    vec![
        (10_000_000, l2 - 2_048),
        (8_000_000, l2 - 1_024),
        (6_000_000, l2),
        (4_000_000, l2 + 1_024),
        (2_000_000, l2 + 2_048),
    ]
}

/// `f`'s listing as a reverse auction with `steps` `(price, lock time)`,
/// each signed over its lock coin, in the row's steps and its listing file
/// (expiry MTP + 365 days) alike. The signed steps.
fn as_schedule(f: &Fx, steps: &[(u64, u64)]) -> Vec<PriceStep> {
    as_schedule_at(f, steps, MTP)
}

/// [`as_schedule`], the file's expiry `mtp` + 365 days.
fn as_schedule_at(f: &Fx, steps: &[(u64, u64)], mtp: u64) -> Vec<PriceStep> {
    let signed: Vec<PriceStep> = steps
        .iter()
        .map(|&(price, lock_time)| signed_step(&f.key, &f.payment, price, lock_time))
        .collect();
    let file = file_of(
        &f.key,
        &f.payment,
        &signed,
        mtp + sell::LISTING_LIFETIME_SECS,
    );
    f.conn
        .execute(
            "UPDATE shakedex_listings SET mode = 'reverse_auction', steps_json = ?1,
             listing_file_json = ?2",
            [stored(&signed), file],
        )
        .unwrap();
    signed
}

/// R24 (T8): a Listed reverse auction's first upload is its current step
/// alone — the cheapest step valid at the node's median time, not the
/// first, not the floor — and the step sent is recorded.
#[tokio::test]
async fn uploads_current_step_only() {
    let f = fx(ListingState::Listed, true);
    let steps = as_schedule(&f, &schedule());
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[2].clone()],
        "one step: the current one"
    );
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    assert_eq!(
        l.market_sent_step.as_deref(),
        Some(hex::encode(steps[2].signature).as_str())
    );
    // Kept like a Buy Now: the hourly check compares the market's copy with
    // the current step and finds ours.
    let mut s = market().await;
    let get = copy_mock(&mut s, sent.clone(), 1).await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&f, &s, NOW + 3_600).await;
    get.assert_async().await;
    m.assert_async().await;
}

/// R24 (T8): a reverse auction whose first step is not valid yet at the
/// node's median time (a file imported by R32, or a median time a reorg
/// took back) is not uploaded and nothing is written — no verdict, nothing
/// to offer; once the median time passes it, its first step is.
#[tokio::test]
async fn no_upload_before_the_first_step_is_valid() {
    let f = fx(ListingState::Listed, true);
    let first = MTP + 512;
    let steps = as_schedule(&f, &[(10_000_000, first), (8_000_000, first + 1_024)]);
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    any.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (
            l.market_status,
            l.market_retry_at,
            l.market_attempts,
            l.market_sent_step
        ),
        (None, None, 0, None),
        "nothing written"
    );
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&f, &node_at(&f, TIP, valid_from_mtp(first)), &s, NOW + 600).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[0].clone()]
    );
}

fn rfc3339(secs: i64) -> String {
    crate::shakedex_jobs::rfc3339(secs)
}

/// A Listed, published listing the market already took (an upload of it
/// accepted: accepted and told), due for its hourly check.
fn listed_on_market(status: MarketStatus) -> Fx {
    let f = fx(ListingState::Listed, true);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_status = ?1, market_retry_at = ?2,
             market_accepted = 1, market_told = 1",
            [status.as_str(), &rfc3339(NOW)],
        )
        .unwrap();
    f
}

/// Our copy, as the market would serve it (the stored file cut to its step).
fn our_copy(f: &Fx) -> String {
    listing_file_market_copy(listing(f).listing_file_json.as_deref().unwrap())
}

fn listing_file_market_copy(stored: &str) -> String {
    crate::noncustodial::shakedex::listing_file::market_copy(stored, 0, NET).unwrap()
}

async fn keep(f: &Fx, s: &ServerGuard, now: i64) {
    keep_listed_with_client(
        &f.conn,
        &node(f, TIP),
        &client(s),
        PROFILE,
        now,
        &LockLost::default(),
    )
    .await
    .unwrap();
}

/// The market serves `body` (200) as its copy of NAME.
async fn copy_mock(s: &mut ServerGuard, body: String, hits: usize) -> mockito::Mock {
    s.mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_body(body)
        .expect(hits)
        .create_async()
        .await
}

/// R25: the market serves someone else's copy (another price, signature or
/// fee address over our lock): ours is uploaded over it, and the listing
/// says "replaced on the market — re-uploaded".
#[tokio::test]
async fn reuploads_when_replaced() {
    let f = listed_on_market(MarketStatus::Listed);
    // A Buy Now the market took before T8 recorded the step sent: nothing
    // tells its own copy from someone else's but `market_changed`.
    assert_eq!(listing(&f).market_sent_step, None);
    let mut theirs: serde_json::Value = serde_json::from_str(&our_copy(&f)).unwrap();
    theirs["data"][0]["price"] = 1.into();
    let mut s = market().await;
    let _get = copy_mock(&mut s, theirs.to_string(), 1).await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(sent, our_copy(&f));
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::ReplacedReuploaded));
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 3_600).as_str())
    );
}

async fn keep_at(f: &Fx, s: &ServerGuard, now: i64, mtp: u64) {
    keep_listed_with_client(
        &f.conn,
        &node_at(f, TIP, mtp),
        &client(s),
        PROFILE,
        now,
        &LockLost::default(),
    )
    .await
    .unwrap();
}

/// [`uploads_current_step_only`]'s first upload: a reverse auction of
/// [`schedule`] on the market, its step 2 accepted. The signed steps and
/// the copy sent.
async fn schedule_on_market(f: &Fx) -> (Vec<PriceStep>, String) {
    let steps = as_schedule(f, &schedule());
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(f, &node(f, TIP), &s, NOW).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    (steps, sent)
}

/// The median time at which [`schedule`]'s step 3 is the current one.
fn step_3_mtp() -> u64 {
    valid_from_mtp(schedule()[3].1)
}

/// R25 (T8, fix round 1): the step moved with no write (no Lower price, no
/// refresh: `market_changed` clear) and the market's copy is the step we
/// last sent (`market_sent_step`): it is our own earlier step, not someone
/// else's — the current step goes up as Listed. Here the step moves while
/// the market gave no answer.
#[tokio::test]
async fn our_earlier_step_on_the_market_is_ours() {
    let f = fx(ListingState::Listed, true);
    let (steps, sent) = schedule_on_market(&f).await;
    let mut s = market().await;
    let get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(502)
        .with_body("<html>Application failed to respond</html>")
        .expect(1)
        .create_async()
        .await;
    keep(&f, &s, NOW + 3_600).await;
    get.assert_async().await;
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Retrying));
    assert!(!l.market_changed);
    let mut s = market().await;
    let get = copy_mock(&mut s, sent, 1).await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    keep_at(&f, &s, NOW + 3_900, step_3_mtp()).await;
    get.assert_async().await;
    m.assert_async().await;
    let (_, up) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&up, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    assert_eq!(l.market_sent_step, Some(hex::encode(steps[3].signature)));
}

/// R25 (T8, fix round 1): after the step moved, a copy that is not the step
/// we last sent — another signature, our sent step's signature over
/// another offer (a price changed), or a stored step of ours we did not
/// send (step 1, dearer: anyone holding the file may upload it) — is
/// someone else's: ReplacedReuploaded.
#[tokio::test]
async fn foreign_copy_after_the_step_moved_is_replaced() {
    for case in [
        "foreign signature",
        "our signature, another price",
        "our step 1, not sent",
    ] {
        let f = fx(ListingState::Listed, true);
        let (_, sent) = schedule_on_market(&f).await;
        let theirs = if case == "foreign signature" {
            let foreign = signed_step(&f.key, &f.payment, 7_000_000, schedule()[2].1);
            let file = file_of(
                &f.key,
                &f.payment,
                &[foreign],
                MTP + sell::LISTING_LIFETIME_SECS,
            );
            listing_file_market_copy(&file)
        } else if case == "our step 1, not sent" {
            let stored = listing(&f).listing_file_json.unwrap();
            crate::noncustodial::shakedex::listing_file::market_copy(&stored, 1, NET).unwrap()
        } else {
            let mut v: serde_json::Value = serde_json::from_str(&sent).unwrap();
            v["data"][0]["price"] = 1.into();
            v.to_string()
        };
        let mut s = market().await;
        let get = copy_mock(&mut s, theirs, 1).await;
        let (m, _) = upload_mock(&mut s, 1).await;
        keep_at(&f, &s, NOW + 3_600, step_3_mtp()).await;
        get.assert_async().await;
        m.assert_async().await;
        assert_eq!(
            listing(&f).market_status,
            Some(MarketStatus::ReplacedReuploaded),
            "{case}"
        );
    }
}

async fn advance(f: &Fx, node: &MockNodeRpc, s: &ServerGuard, now: i64) {
    advance_reverse_auctions_with_client(
        &f.conn,
        node,
        &client(s),
        PROFILE,
        now,
        &LockLost::default(),
    )
    .await
    .unwrap();
}

/// A Listed reverse auction on [`schedule`] whose current step at MTP
/// (step 2) the market accepted at NOW; the signed steps.
async fn auction_on_market() -> (Fx, Vec<PriceStep>) {
    let f = fx(ListingState::Listed, true);
    let (steps, _) = schedule_on_market(&f).await;
    (f, steps)
}

/// The median time at which step 3 of [`schedule`] is current, and the
/// wall clock then (before the hourly check is due).
fn moved() -> (u64, i64) {
    let mtp = step_3_mtp();
    (mtp, NOW + i64::try_from(mtp - MTP).unwrap())
}

/// `f`'s stored listing file cut to its step `i`, as the market would serve
/// it.
fn listing_file_market_copy_at(f: &Fx, i: usize) -> String {
    let stored = listing(f).listing_file_json.unwrap();
    crate::noncustodial::shakedex::listing_file::market_copy(&stored, i, NET).unwrap()
}

/// R24 (T8): once the node's median time makes a cheaper step current, the
/// next sync uploads it alone, though the hourly check is not due, with no
/// read of the market's copy; once is enough at the same median time.
#[tokio::test]
async fn uploads_again_when_the_step_moves() {
    let (f, steps) = auction_on_market().await;
    // Not moved: nothing.
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    advance(&f, &node(&f, TIP), &s, NOW + 60).await;
    any.assert_async().await;

    let (mtp, later) = moved();
    assert!(later < NOW + 3_600, "the hourly check is not due");
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later).await;
    m.assert_async().await;
    get.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    assert_eq!(
        l.market_sent_step.as_deref(),
        Some(hex::encode(steps[3].signature).as_str())
    );
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(later + 3_600).as_str())
    );

    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later + 60).await;
    any.assert_async().await;
}

/// R24 with R25 (T8, code review): a reverse auction whose market copy was
/// replaced by someone else's and re-uploaded (ReplacedReuploaded) is
/// advanced too: once its step moves, the current step goes up once. The
/// accepted upload keeps the warning (ReplacedReuploaded stays, as for a
/// Buy Now, until keep-listed finds our copy served back).
#[tokio::test]
async fn replaced_auction_advances_and_keeps_its_warning() {
    let (f, steps) = auction_on_market().await;
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_status = ?1",
            [MarketStatus::ReplacedReuploaded.as_str()],
        )
        .unwrap();
    let (mtp, later) = moved();
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::ReplacedReuploaded));
    assert_eq!(l.market_sent_step, Some(hex::encode(steps[3].signature)));
}

/// R3, R24 (T8, code review): the advance job's "current step" is the step
/// the market copy is cut to, ties included. A schedule lowered to 4 HNS (a
/// step appended, equal to step 3's price) offers the lowered step first;
/// once step 3 is valid too, the cheapest valid step is the first of the
/// two (step 3): the advance job uploads exactly that once, the median
/// time read once for both the move and the copy, and leaves it there.
#[tokio::test]
async fn advance_moves_to_the_step_the_copy_is_cut_to_on_equal_prices() {
    let (f, steps) = auction_on_market().await;
    let old = listing(&f);
    let earlier = listing_file_market_copy_at(&f, 2);
    let lower = signed_step(&f.key, &f.payment, 4_000_000, sell::buy_now_lock_time(MTP));
    assert_eq!(lower.price, steps[3].price, "equal-priced");
    let mut lowered = steps.clone();
    lowered.push(lower.clone());
    let file = file_of(
        &f.key,
        &f.payment,
        &lowered,
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    let txid_f1 = txid("f1");
    assert_eq!(
        queries::lower_listing_price(
            &f.conn,
            &f.id,
            &queries::LoweredPrice {
                lock: (&txid_f1, 0),
                old_steps_json: &old.steps_json,
                steps_json: &stored(&lowered),
                listing_file_json: &file,
                expires_at: old.expires_at.unwrap(),
            },
        )
        .unwrap(),
        1
    );
    let mut s = market().await;
    let _get = copy_mock(&mut s, earlier, 1).await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW + 60).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![lower.clone()]
    );
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    assert_eq!(l.market_sent_step, Some(hex::encode(lower.signature)));

    let (mtp, later) = moved();
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    let n = node_at(&f, TIP, mtp);
    advance(&f, &n, &s, later).await;
    m.assert_async().await;
    assert_eq!(n.call_count(), 2, "the median time once, the lock coin");
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    assert_eq!(
        listing(&f).market_sent_step,
        Some(hex::encode(steps[3].signature))
    );

    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later + 60).await;
    any.assert_async().await;
}

/// R25 with R24 (T8): a moved step whose upload got no answer is retried by
/// keep-listed after its backoff; the market's copy is then our earlier
/// step, so ours is uploaded over it as Listed, not "replaced by someone
/// else's copy".
#[tokio::test]
async fn our_earlier_step_on_the_market_is_not_a_replacement() {
    let (f, steps) = auction_on_market().await;
    let earlier = listing_file_market_copy_at(&f, 2);
    let (mtp, later) = moved();
    let mut s = market().await;
    let _down = s
        .mock("POST", "/api/upload-proof")
        .with_status(502)
        .with_body("<html>Bad gateway</html>")
        .create_async()
        .await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later).await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_changed),
        (Some(MarketStatus::Retrying), true)
    );
    // Advance leaves a Retrying listing to keep-listed's backoff, even once
    // the schedule moved on again (step 4 current: not the step last sent).
    for at in [mtp, valid_from_mtp(schedule()[4].1)] {
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        advance(&f, &node_at(&f, TIP, at), &s, later + 60).await;
        any.assert_async().await;
    }

    let retry = later + crate::shakedex_jobs::retry_delay_secs(1);
    let mut s = market().await;
    let _get = copy_mock(&mut s, earlier, 1).await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    keep_listed_with_client(
        &f.conn,
        &node_at(&f, TIP, mtp),
        &client(&s),
        PROFILE,
        retry,
        &LockLost::default(),
    )
    .await
    .unwrap();
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Listed));
}

/// Deviation 6 (T8): the market's refusal of a step is not retried for that
/// step, and is tried again once the schedule moves to the next one (what
/// is sent changed).
#[tokio::test]
async fn refused_auction_is_tried_again_when_its_step_moves() {
    let f = fx(ListingState::Listed, true);
    let steps = as_schedule(&f, &schedule());
    let mut s = market().await;
    let _refuse = s
        .mock("POST", "/api/upload-proof")
        .with_status(409)
        .with_body(r#"{"error":"An active listing for dexjobs already exists."}"#)
        .expect(1)
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Refused));
    assert_eq!(
        l.market_sent_step.as_deref(),
        Some(hex::encode(steps[2].signature).as_str())
    );
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    advance(&f, &node(&f, TIP), &s, NOW + 600).await;
    any.assert_async().await;

    let (mtp, later) = moved();
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later).await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Listed));
}

/// R25 with deviation 5 (T8): a moved step not taken marks the market's
/// differing copy as our own earlier one only when the market took this
/// listing before (`market_accepted`). Here it never did (step 2 refused),
/// so after step 3's upload got no answer, a differing copy — step 2, which
/// the market refused from us — is someone else's: ReplacedReuploaded.
#[tokio::test]
async fn never_taken_auction_does_not_claim_the_markets_copy() {
    let f = fx(ListingState::Listed, true);
    let steps = as_schedule(&f, &schedule());
    let mut s = market().await;
    let _refuse = s
        .mock("POST", "/api/upload-proof")
        .with_status(409)
        .with_body(r#"{"error":"An active listing for dexjobs already exists."}"#)
        .expect(1)
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Refused));
    let theirs = listing_file_market_copy_at(&f, 2);

    let (mtp, later) = moved();
    let mut s = market().await;
    let _down = s
        .mock("POST", "/api/upload-proof")
        .with_status(502)
        .with_body("<html>Bad gateway</html>")
        .expect(1)
        .create_async()
        .await;
    advance(&f, &node_at(&f, TIP, mtp), &s, later).await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_changed, l.market_accepted),
        (Some(MarketStatus::Retrying), false, false)
    );

    let retry = later + crate::shakedex_jobs::retry_delay_secs(1);
    let mut s = market().await;
    let _get = copy_mock(&mut s, theirs, 1).await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    keep_listed_with_client(
        &f.conn,
        &node_at(&f, TIP, mtp),
        &client(&s),
        PROFILE,
        retry,
        &LockLost::default(),
    )
    .await
    .unwrap();
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().steps,
        vec![steps[3].clone()]
    );
    assert_eq!(
        listing(&f).market_status,
        Some(MarketStatus::ReplacedReuploaded)
    );
}

/// R25 (T8, code review): a Lower price claims the market's differing copy
/// as our own older one only when the market took this listing before
/// (`market_accepted`), as a moved step does. A reverse auction whose step
/// the market refused, lowered, finds someone else's copy there:
/// ReplacedReuploaded, the lowered step uploaded over it.
#[tokio::test]
async fn lowered_never_taken_auction_does_not_claim_the_markets_copy() {
    let f = fx(ListingState::Listed, true);
    let steps = as_schedule(&f, &schedule());
    let mut s = market().await;
    let _refuse = s
        .mock("POST", "/api/upload-proof")
        .with_status(409)
        .with_body(r#"{"error":"An active listing for dexjobs already exists."}"#)
        .expect(1)
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Refused));

    let old = listing(&f);
    let lower = signed_step(&f.key, &f.payment, 5_000_000, sell::buy_now_lock_time(MTP));
    let mut lowered = steps.clone();
    lowered.push(lower.clone());
    let file = file_of(
        &f.key,
        &f.payment,
        &lowered,
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    let txid_f1 = txid("f1");
    assert_eq!(
        queries::lower_listing_price(
            &f.conn,
            &f.id,
            &queries::LoweredPrice {
                lock: (&txid_f1, 0),
                old_steps_json: &old.steps_json,
                steps_json: &stored(&lowered),
                listing_file_json: &file,
                expires_at: old.expires_at.unwrap(),
            },
        )
        .unwrap(),
        1
    );
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_accepted, l.market_changed),
        (Some(MarketStatus::Retrying), false, false)
    );

    let foreign = signed_step(&f.key, &f.payment, 7_000_000, schedule()[2].1);
    let theirs = listing_file_market_copy(&file_of(
        &f.key,
        &f.payment,
        &[foreign],
        MTP + sell::LISTING_LIFETIME_SECS,
    ));
    let mut s = market().await;
    let get = copy_mock(&mut s, theirs, 1).await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW + 60).await;
    get.assert_async().await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(ListingFile::parse(&sent, NET).unwrap().steps, vec![lower]);
    assert_eq!(
        listing(&f).market_status,
        Some(MarketStatus::ReplacedReuploaded)
    );
}

/// Deviation 7 (T8): the advance job takes reverse auctions only — a Buy
/// Now (its current step moves only by Lower price, whose write already
/// makes it due) is not looked at, even with no step recorded as sent; and
/// one the market has not taken yet is the publish job's. Deviation 8: a
/// Retrying or StepsUnverified one is keep-listed's (its backoff), though
/// its step moved. Nothing is read.
#[tokio::test]
async fn advance_takes_only_reverse_auctions_the_market_took() {
    for case in ["buy_now", "untold", "retrying", "steps_unverified"] {
        let f = listed_on_market(MarketStatus::Listed);
        if case != "buy_now" {
            let steps = as_schedule(&f, &schedule());
            let status = match case {
                "untold" => None,
                "retrying" => Some(MarketStatus::Retrying),
                _ => Some(MarketStatus::StepsUnverified),
            };
            f.conn
                .execute(
                    "UPDATE shakedex_listings SET market_status = ?1, market_sent_step = ?2",
                    rusqlite::params![status.map(|s| s.as_str()), hex::encode(steps[2].signature)],
                )
                .unwrap();
        }
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        let n = node_at(&f, TIP, moved().0);
        advance(&f, &n, &s, NOW).await;
        any.assert_async().await;
        assert_eq!(n.call_count(), 0, "{case}: the node was not read");
    }
}

/// R24 (T8): a reverse auction on the market whose steps no longer read is
/// not taken for "not moved": the advance job records it as
/// StepsUnverified, backed off, and uploads nothing.
#[tokio::test]
async fn advance_records_an_unreadable_row() {
    let (f, _) = auction_on_market().await;
    f.conn
        .execute("UPDATE shakedex_listings SET steps_json = 'not json'", [])
        .unwrap();
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    advance(&f, &node(&f, TIP), &s, NOW + 60).await;
    any.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_attempts),
        (Some(MarketStatus::StepsUnverified), 1)
    );
    assert!(l.market_error.unwrap().contains("unreadable steps"));
}

/// R24 (T8): no step valid at the node's median time (a reorg took it back
/// before step 0) is no move: nothing uploaded, nothing written, and the
/// node read once (its median time) — not handed to the upload.
#[tokio::test]
async fn no_valid_step_is_no_move() {
    let (f, _) = auction_on_market().await;
    let before = listing(&f);
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    let n = node_at(&f, TIP, schedule()[0].1 - 1_024);
    advance(&f, &n, &s, NOW + 60).await;
    any.assert_async().await;
    assert_eq!(n.call_count(), 1, "the median time only");
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at, l.market_sent_step),
        (
            before.market_status,
            before.market_retry_at,
            before.market_sent_step
        )
    );
}

/// R23: off mainnet the advance job reads nothing and calls no market.
#[tokio::test]
async fn nothing_advanced_off_mainnet() {
    let (f, _) = auction_on_market().await;
    f.conn
        .execute(
            "UPDATE wallet_profiles SET network = 'regtest' WHERE id = ?1",
            [PROFILE],
        )
        .unwrap();
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    let n = node_at(&f, TIP, moved().0);
    advance(&f, &n, &s, NOW).await;
    any.assert_async().await;
    assert_eq!(n.call_count(), 0);
}

/// R25: a copy that offers a buyer exactly ours — keys in Flask's order, an
/// extra field the market adds — is not uploaded again; the next check is
/// an hour on, and nothing is asked before then. R24 (T8): the step it
/// serves as ours is recorded as sent.
#[tokio::test]
async fn matching_copy_is_not_reuploaded() {
    let f = listed_on_market(MarketStatus::Listed);
    let mut served: serde_json::Value = serde_json::from_str(&our_copy(&f)).unwrap();
    served["served"] = true.into();
    let mut s = market().await;
    let get = copy_mock(&mut s, served.to_string(), 1).await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&f, &s, NOW).await;
    keep(&f, &s, NOW + 3_599).await;
    get.assert_async().await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Listed));
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 3_600).as_str())
    );
    let ours = ListingFile::parse(&our_copy(&f), NET).unwrap();
    assert_eq!(
        l.market_sent_step,
        Some(hex::encode(ours.steps[0].signature))
    );
}

/// R25: no answer from the market (a proxy's 502 page) is "Not on market —
/// retrying", never a verdict: nothing is uploaded on it, and the next try
/// waits 5 minutes, then 10, then 20 — exponential backoff. The market's
/// own "not listed" (its JSON 404) gets an upload, which puts it back.
#[tokio::test]
async fn backs_off_when_unreachable() {
    let f = listed_on_market(MarketStatus::Listed);
    let mut s = market().await;
    let get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(502)
        .with_body("<html>Application failed to respond</html>")
        .expect(3)
        .create_async()
        .await;
    let (m, _) = upload_mock(&mut s, 0).await;
    let mut at = NOW;
    for (attempts, wait) in [(1, 300), (2, 600), (3, 1_200)] {
        keep(&f, &s, at).await;
        let l = listing(&f);
        assert_eq!(
            (l.market_status, l.market_attempts),
            (Some(MarketStatus::Retrying), attempts)
        );
        assert_eq!(
            l.market_retry_at.as_deref(),
            Some(rfc3339(at + wait).as_str())
        );
        keep(&f, &s, at + wait - 1).await; // not due: no request
        at += wait;
    }
    get.assert_async().await;
    m.assert_async().await;
    drop(get);
    drop(m);
    let _gone = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .create_async()
        .await;
    let (m, _) = upload_mock(&mut s, 1).await;
    keep(&f, &s, at).await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_attempts, l.market_error),
        (Some(MarketStatus::Listed), 0, None)
    );
    assert_eq!(
        retry_delays(),
        [300, 600, 1_200, 2_400, 4_800, 9_600, 19_200, 21_600, 21_600]
    );
}

fn retry_delays() -> Vec<i64> {
    (1..=9)
        .map(crate::shakedex_jobs::retry_delay_secs)
        .collect()
}

/// R23: an `expiresAt` within 30 days of now is moved to the node's median
/// time plus 365 days in the stored file (every other field as written) and
/// on the market; the listing's `expires_at` follows.
#[tokio::test]
async fn expires_at_refreshed_before_it_lapses() {
    let f = listed_on_market(MarketStatus::Listed);
    let soon = (NOW + 29 * 86_400) as u64;
    set_expiry(&f, soon);
    let old = listing(&f).listing_file_json.unwrap();
    let mut s = market().await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    // The market still serves the old copy; it is replaced by the refreshed one.
    // The file just changed: uploaded at once, the market's copy not asked.
    let get = copy_mock(&mut s, listing_file_market_copy(&old), 0).await;
    keep(&f, &s, NOW).await;
    m.assert_async().await;
    get.assert_async().await;
    let want = MTP + sell::LISTING_LIFETIME_SECS;
    let l = listing(&f);
    assert_eq!(l.expires_at, Some(want as i64));
    let stored_file = l.listing_file_json.as_deref().unwrap();
    assert_eq!(
        ListingFile::parse(stored_file, NET).unwrap().expires_at,
        Some(want)
    );
    assert_eq!(
        stored_file,
        crate::noncustodial::shakedex::listing_file::with_expiry(&old, want, NET).unwrap(),
        "every other field as written"
    );
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(
        ListingFile::parse(&sent, NET).unwrap().expires_at,
        Some(want)
    );
    assert_eq!(l.market_status, Some(MarketStatus::Listed));

    // 31 days left: not refreshed.
    let g = listed_on_market(MarketStatus::Listed);
    let far = (NOW + 31 * 86_400) as u64;
    set_expiry(&g, far);
    let file = listing(&g).listing_file_json.unwrap();
    let mut s = market().await;
    let _get = copy_mock(&mut s, listing_file_market_copy(&file), 1).await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&g, &s, NOW).await;
    m.assert_async().await;
    assert_eq!(listing(&g).expires_at, Some(far as i64));
}

/// R25 (ruling 2026-10-10): the market's own refusal (a 4xx JSON `error`)
/// is a verdict: kept with its words, Refused, and **not** retried — no
/// request reaches the market on later syncs, however long after. Only a
/// write that changes what is sent starts it over: here a Lower price
/// (a cheaper step, `queries::lower_listing_price`), after which the next
/// run uploads the new current step. (No answer, by contrast, is retried
/// with backoff: `backs_off_when_unreachable`.)
#[tokio::test]
async fn refused_upload_is_not_retried_until_it_changes() {
    let f = listed_on_market(MarketStatus::Retrying);
    let mut s = market().await;
    let get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .expect(1)
        .create_async()
        .await;
    let refuse = s
        .mock("POST", "/api/upload-proof")
        .with_status(409)
        .with_body(r#"{"error":"An active listing for dexjobs already exists. Cancel it or wait for it to expire before uploading a different listing proof."}"#)
        .expect(1)
        .create_async()
        .await;
    keep(&f, &s, NOW).await;
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::Refused));
    assert!(l
        .market_error
        .clone()
        .unwrap()
        .contains("An active listing for dexjobs already exists"));
    assert_eq!(l.market_retry_at, None, "no automatic retry");
    // Hours and days later: nothing is asked of the market.
    for at in [NOW + 300, NOW + 6 * 3_600, NOW + 7 * 86_400] {
        publish(&f, &node(&f, TIP), &s, at).await;
        keep(&f, &s, at).await;
    }
    get.assert_async().await;
    refuse.assert_async().await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Refused));
    drop(refuse);
    // A Lower price changes what is sent: the bookkeeping starts over and
    // the cheaper step is uploaded at the next run.
    let old = listing(&f);
    let first = signed_step(&f.key, &f.payment, PRICE, sell::buy_now_lock_time(MTP));
    let lower = signed_step(&f.key, &f.payment, 3_000_000, sell::buy_now_lock_time(MTP));
    let file = file_of(
        &f.key,
        &f.payment,
        &[first.clone(), lower.clone()],
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    let steps = stored(&[first, lower.clone()]);
    let txid_f1 = txid("f1");
    assert_eq!(
        queries::lower_listing_price(
            &f.conn,
            &f.id,
            &queries::LoweredPrice {
                lock: (&txid_f1, 0),
                old_steps_json: &old.steps_json,
                steps_json: &steps,
                listing_file_json: &file,
                expires_at: old.expires_at.unwrap(),
            },
        )
        .unwrap(),
        1
    );
    let l = listing(&f);
    assert_eq!(
        (
            l.market_status,
            l.market_retry_at,
            l.market_attempts,
            l.market_error
        ),
        (Some(MarketStatus::Retrying), None, 0, None),
        "started over, due now"
    );
    get.remove_async().await;
    let get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .expect(1)
        .create_async()
        .await;
    let (m, seen) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW + 7 * 86_400).await;
    keep(&f, &s, NOW + 7 * 86_400).await;
    get.assert_async().await;
    m.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(ListingFile::parse(&sent, NET).unwrap().steps, vec![lower]);
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Listed));
}

/// Ruling 2026-10-10: a Refused Listed listing near its expiry gets the
/// expiry check only — the refresh, read from our node, with no market call
/// — which makes it Retrying, due now, its acceptance kept (what is sent
/// changed); the next keep-listed run checks the market's copy and uploads
/// the new file.
#[tokio::test]
async fn refused_listing_near_expiry_is_refreshed_without_a_market_call() {
    let f = listed_on_market(MarketStatus::Refused);
    f.conn
        .execute("UPDATE shakedex_listings SET market_retry_at = NULL", [])
        .unwrap();
    set_expiry(&f, (NOW + 86_400) as u64);
    let mut s = market().await;
    let any = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&f, &s, NOW).await;
    any.assert_async().await;
    m.assert_async().await;
    let want = MTP + sell::LISTING_LIFETIME_SECS;
    let l = listing(&f);
    assert_eq!(l.expires_at, Some(want as i64));
    assert_eq!(
        (l.market_status, l.market_retry_at, l.market_accepted),
        (Some(MarketStatus::Retrying), None, true),
        "started over, due now"
    );
    any.remove_async().await;
    m.remove_async().await;
    let get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .expect(1)
        .create_async()
        .await;
    let (m, _) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW).await;
    get.assert_async().await;
    m.assert_async().await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Listed));

    // Far from its expiry: the Refused listing is not even read from the node.
    let g = listed_on_market(MarketStatus::Refused);
    let mut s = market().await;
    let any = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    let n = node(&g, TIP);
    keep_listed_with_client(&g.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
        .await
        .unwrap();
    any.assert_async().await;
    post.assert_async().await;
    assert_eq!(n.call_count(), 0);
    assert_eq!(listing(&g).market_status, Some(MarketStatus::Refused));

    // A Refused Cancelling listing (cancel unsent) near its expiry: its
    // expiry is never refreshed (a Listed-only write), so the node is not
    // read either.
    let h = listed_on_market(MarketStatus::Refused);
    set_expiry(&h, (NOW + 86_400) as u64);
    queries::insert_tx_draft(&h.conn, "cd", PROFILE, "x", "00", "{}", "{}").unwrap();
    h.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'cancelling', cancel_draft_id = 'cd'",
            [],
        )
        .unwrap();
    let mut s = market().await;
    let any = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    let n = node(&h, TIP);
    keep_listed_with_client(&h.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
        .await
        .unwrap();
    any.assert_async().await;
    post.assert_async().await;
    assert_eq!(n.call_count(), 0);
    assert_eq!(listing(&h).market_status, Some(MarketStatus::Refused));
}

/// A6 (code-review round): a Refused listing's expiry is judged from its
/// stored file's `expiresAt`, the one source the refresh reads, never from
/// the `expires_at` column: a column unset or far while the file's is far
/// reads nothing from the node; a file near its end is refreshed whatever
/// the column says; and nothing is read before the listing is due.
#[tokio::test]
async fn refused_listing_expiry_is_read_from_its_file_once_due() {
    // The column unset, the file's expiry far: nothing read.
    let f = listed_on_market(MarketStatus::Refused);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_retry_at = NULL, expires_at = NULL",
            [],
        )
        .unwrap();
    let s = market().await;
    let n = node(&f, TIP);
    for at in [NOW, NOW + 3_600] {
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, at, &LockLost::default())
            .await
            .unwrap();
    }
    assert_eq!(n.call_count(), 0, "a far file expiry reads nothing");
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Refused));

    // The file near its end, the column far: refreshed (Retrying, due now).
    let g = listed_on_market(MarketStatus::Refused);
    g.conn
        .execute("UPDATE shakedex_listings SET market_retry_at = NULL", [])
        .unwrap();
    set_expiry(&g, (NOW + 86_400) as u64);
    g.conn
        .execute(
            "UPDATE shakedex_listings SET expires_at = ?1",
            [NOW + 300 * 86_400],
        )
        .unwrap();
    let mut s = market().await;
    let any = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    keep(&g, &s, NOW).await;
    any.assert_async().await;
    post.assert_async().await;
    let l = listing(&g);
    assert_eq!(
        (l.market_status, l.expires_at),
        (
            Some(MarketStatus::Retrying),
            Some((MTP + sell::LISTING_LIFETIME_SECS) as i64)
        )
    );

    // Near its end but not due yet: nothing read.
    let h = listed_on_market(MarketStatus::Refused);
    set_expiry(&h, (NOW + 86_400) as u64);
    h.conn
        .execute(
            "UPDATE shakedex_listings SET market_retry_at = ?1",
            [rfc3339(NOW + 60)],
        )
        .unwrap();
    let s = market().await;
    let n = node(&h, TIP);
    keep_listed_with_client(&h.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
        .await
        .unwrap();
    assert_eq!(n.call_count(), 0, "not due: nothing read");
    assert_eq!(listing(&h).market_status, Some(MarketStatus::Refused));
}

/// R23: off mainnet the keep-listed job returns before any read: no node
/// call, no market request, nothing written — even for a listing that is
/// due and that a mainnet client would check.
#[tokio::test]
async fn nothing_kept_off_mainnet() {
    let f = listed_on_market(MarketStatus::Listed);
    f.conn
        .execute(
            "UPDATE wallet_profiles SET network = 'regtest' WHERE id = ?1",
            [PROFILE],
        )
        .unwrap();
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let n = node(&f, TIP);
    keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
        .await
        .unwrap();
    any.assert_async().await;
    get.assert_async().await;
    assert_eq!(n.call_count(), 0);
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at),
        (Some(MarketStatus::Listed), Some(rfc3339(NOW)))
    );
}

/// R25: a copy that does not read as a listing file — here the price spelled
/// as a float, which hsd's integers never are — is not ours: ours is
/// uploaded over it. Once ours is served again it is left alone, still
/// saying "replaced on the market — re-uploaded".
#[tokio::test]
async fn unreadable_copy_is_replaced() {
    let f = listed_on_market(MarketStatus::Listed);
    let ours = our_copy(&f);
    let floated = ours.replace(&format!(":{PRICE}"), &format!(":{PRICE}.0"));
    assert_ne!(floated, ours, "the fixture spells the price as a float");
    let mut s = market().await;
    let get = copy_mock(&mut s, floated, 1).await;
    let (m, _) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW).await;
    m.assert_async().await;
    get.assert_async().await;
    assert_eq!(
        listing(&f).market_status,
        Some(MarketStatus::ReplacedReuploaded)
    );
    drop(get);
    drop(m);
    let get = copy_mock(&mut s, ours, 1).await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&f, &s, NOW + 3_600).await;
    get.assert_async().await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(l.market_status, Some(MarketStatus::ReplacedReuploaded));
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 7_200).as_str())
    );
}

/// Carried from T4: a listing whose steps did not verify (StepsUnverified)
/// is verified again on our node before the market is asked anything: still
/// failing → backed off, no request; verifying now → checked against the
/// market's copy, Listed.
#[tokio::test]
async fn steps_unverified_is_verified_again_before_the_market() {
    let f = listed_on_market(MarketStatus::StepsUnverified);
    let mut bad = signed_step(&f.key, &f.payment, PRICE, sell::buy_now_lock_time(MTP));
    bad.price += 1;
    let file = file_of(
        &f.key,
        &f.payment,
        std::slice::from_ref(&bad),
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    f.conn
        .execute(
            "UPDATE shakedex_listings SET steps_json = ?1, listing_file_json = ?2, market_attempts = 1",
            [stored(&[bad]), file],
        )
        .unwrap();
    let mut s = market().await;
    let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&f, &s, NOW).await;
    get.assert_async().await;
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_attempts),
        (Some(MarketStatus::StepsUnverified), 2)
    );
    assert_eq!(
        l.market_retry_at.as_deref(),
        Some(rfc3339(NOW + 600).as_str())
    );
    assert!(l
        .market_error
        .unwrap()
        .contains(sell::STEP_NOT_SIGNED_BY_LOCK));

    let g = listed_on_market(MarketStatus::StepsUnverified);
    let mut s = market().await;
    let get = copy_mock(&mut s, our_copy(&g), 1).await;
    let (m, _) = upload_mock(&mut s, 0).await;
    keep(&g, &s, NOW).await;
    get.assert_async().await;
    m.assert_async().await;
    let l = listing(&g);
    assert_eq!(
        (l.market_status, l.market_attempts, l.market_error),
        (Some(MarketStatus::Listed), 0, None)
    );
}

/// R24, R25: of the jobs' set, one the market has not taken yet (the first
/// upload is `publish_listings_with_client`'s) and a reported one are not
/// asked about.
#[tokio::test]
async fn only_listings_the_market_took_are_kept() {
    for case in ["unset", "reported"] {
        let f = listed_on_market(MarketStatus::Listed);
        let sql = match case {
            "unset" => "UPDATE shakedex_listings SET market_status = NULL",
            _ => "UPDATE shakedex_listings SET market_status = 'reported'",
        };
        f.conn.execute(sql, []).unwrap();
        assert_eq!(
            queries::list_listings_kept_on_market(&f.conn, PROFILE)
                .unwrap()
                .len(),
            1,
            "{case}: in the jobs' set"
        );
        let mut s = market().await;
        let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
        let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        let n = node(&f, TIP);
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
            .await
            .unwrap();
        get.assert_async().await;
        post.assert_async().await;
        assert_eq!(n.call_count(), 0, "{case}");
    }
}

/// R24, R25 (coordinator ruling 2026-10-10): a Cancelling listing whose
/// cancel is not sent yet is still buyable on chain, so it is kept on the
/// market like a Listed one: the market's "not listed" or someone else's
/// copy gets ours uploaded. Once the cancel is sent (R28) it is not touched.
#[tokio::test]
async fn cancelling_listing_is_kept_until_its_cancel_is_sent() {
    for (gone, ok) in [
        (true, MarketStatus::Listed),
        (false, MarketStatus::ReplacedReuploaded),
    ] {
        let f = listed_on_market(MarketStatus::Listed);
        queries::insert_tx_draft(&f.conn, "cd", PROFILE, "x", "00", "{}", "{}").unwrap();
        queries::update_tx_draft_status(&f.conn, "cd", "signed", None, None).unwrap();
        f.conn
            .execute(
                "UPDATE shakedex_listings SET state = 'cancelling', cancel_draft_id = 'cd'",
                [],
            )
            .unwrap();
        let mut s = market().await;
        let get = if gone {
            s.mock("GET", format!("/listing/{NAME}/proof.json").as_str())
                .with_status(404)
                .with_body(PROOF_NOT_FOUND)
                .expect(1)
                .create_async()
                .await
        } else {
            let mut theirs: serde_json::Value = serde_json::from_str(&our_copy(&f)).unwrap();
            theirs["data"][0]["price"] = 1.into();
            copy_mock(&mut s, theirs.to_string(), 1).await
        };
        let (m, seen) = upload_mock(&mut s, 1).await;
        keep(&f, &s, NOW).await;
        get.assert_async().await;
        m.assert_async().await;
        let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
        assert_eq!(sent, our_copy(&f));
        let l = listing(&f);
        assert_eq!(l.state, ListingState::Cancelling);
        assert_eq!(l.market_status, Some(ok));
        assert_eq!(
            l.market_retry_at.as_deref(),
            Some(rfc3339(NOW + 3_600).as_str())
        );

        // The cancel sent: nothing is asked of the market, however long after.
        queries::update_tx_draft_status(&f.conn, "cd", "broadcasted", None, Some(&txid("c1")))
            .unwrap();
        let mut s = market().await;
        let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
        let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        let n = node(&f, TIP);
        keep_listed_with_client(
            &f.conn,
            &n,
            &client(&s),
            PROFILE,
            NOW + 86_400,
            &LockLost::default(),
        )
        .await
        .unwrap();
        get.assert_async().await;
        post.assert_async().await;
        assert_eq!(n.call_count(), 0);
    }
}

/// Fix round 1 (ruling 2026-10-10, option b): a reorg that takes the mined
/// cancel off the chain while this device's cancel draft is unsent
/// (`queries::mark_listing_cancel_unmined`) starts the bookkeeping over on a
/// Cancelling listing the market was told about: Retrying, due now, its
/// acceptance (`market_accepted`) kept. It is still buyable on chain, so
/// keep-listed takes it like any kept listing: the market's copy is asked
/// for first, and the market's "not listed" gets ours uploaded. The first
/// upload stays Listed only. With the cancel sent, nothing is asked of the
/// market.
#[tokio::test]
async fn cancelling_listing_reset_by_a_reorg_is_kept() {
    for (status, kept) in [("signed", true), ("broadcasted", false)] {
        let f = listed_on_market(MarketStatus::Reported);
        queries::insert_tx_draft(&f.conn, "cd", PROFILE, "x", "00", "{}", "{}").unwrap();
        queries::update_tx_draft_status(&f.conn, "cd", status, None, Some(&txid("c1"))).unwrap();
        f.conn
            .execute(
                "UPDATE shakedex_listings SET state = 'cancel_awaiting_finalize',
                 cancel_draft_id = 'cd', cancel_txid = ?1, cancel_vout = 0",
                [txid("c1")],
            )
            .unwrap();
        assert_eq!(
            queries::mark_listing_cancel_unmined(&f.conn, &f.id, &txid("c1")).unwrap(),
            1,
            "{status}"
        );
        let l = listing(&f);
        assert_eq!(l.state, ListingState::Cancelling, "{status}");
        let expected = if kept {
            (Some(MarketStatus::Retrying), None)
        } else {
            (Some(MarketStatus::Reported), Some(rfc3339(NOW)))
        };
        assert_eq!((l.market_status, l.market_retry_at), expected, "{status}");
        let mut s = market().await;
        let hits = usize::from(kept);
        let get = s
            .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
            .with_status(404)
            .with_body(PROOF_NOT_FOUND)
            .expect(hits)
            .create_async()
            .await;
        let (m, seen) = upload_mock(&mut s, hits).await;
        // The publish job's first upload is Listed only: it leaves it.
        publish(&f, &node(&f, TIP), &s, NOW).await;
        assert!(seen.lock().unwrap().is_empty(), "{status}");
        keep(&f, &s, NOW).await;
        get.assert_async().await;
        m.assert_async().await;
        let l = listing(&f);
        if kept {
            let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
            assert_eq!(sent, our_copy(&f));
            assert_eq!(
                (l.state, l.market_status),
                (ListingState::Cancelling, Some(MarketStatus::Listed))
            );
        } else {
            assert_eq!(l.market_status, Some(MarketStatus::Reported));
        }
    }
}

/// Ruling 2026-10-10 (fix round 1 of Step 6): a Cancelling listing is kept
/// on the market only once the market accepted an upload of it
/// (`market_accepted`). One it never accepted — never told (the publish
/// job's first upload had not run), its steps unverified (nothing
/// uploaded), or Retrying after no answer to its first upload — whose cancel
/// is signed but unsent is published by neither job.
#[tokio::test]
async fn never_accepted_listing_is_not_published_while_cancelling() {
    for status in [
        None,
        Some(MarketStatus::StepsUnverified),
        Some(MarketStatus::Retrying),
    ] {
        let f = fx(ListingState::Listed, true);
        f.conn
            .execute(
                "UPDATE shakedex_listings SET market_status = ?1",
                [status.map(|s| s.as_str())],
            )
            .unwrap();
        cancelling(&f, "signed");
        assert!(!listing(&f).market_accepted);
        let mut s = market().await;
        let get = s.mock("GET", Matcher::Any).expect(0).create_async().await;
        let post = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        for at in [NOW, NOW + 3_600, NOW + 86_400] {
            publish(&f, &node(&f, TIP), &s, at).await;
            keep(&f, &s, at).await;
        }
        get.assert_async().await;
        post.assert_async().await;
        assert_eq!(listing(&f).market_status, status, "{status:?}");
    }
}

/// Ruling 2026-10-10 (fix round 1 of Step 6): an accepted upload is
/// recorded (`market_accepted`) and outlives a later failure: accepted, then
/// no answer to the hourly check (Retrying), then a cancel signed — the
/// listing is still buyable on chain and on the market, so it is kept: the
/// market's "not listed" gets ours uploaded.
#[tokio::test]
async fn accepted_listing_is_kept_while_cancelling_after_no_answer() {
    let f = fx(ListingState::Listed, true);
    let mut s = market().await;
    let (up, _) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    up.assert_async().await;
    up.remove_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_accepted),
        (Some(MarketStatus::Listed), true)
    );
    let down = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(503)
        .with_body("<html>unavailable</html>")
        .expect(1)
        .create_async()
        .await;
    keep(&f, &s, NOW + 3_600).await;
    down.assert_async().await;
    down.remove_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_accepted),
        (Some(MarketStatus::Retrying), true)
    );
    cancelling(&f, "signed");
    let get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .expect(1)
        .create_async()
        .await;
    let (up, seen) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW + 3_600 + 300).await;
    get.assert_async().await;
    up.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(sent, our_copy(&f));
    let l = listing(&f);
    assert_eq!(
        (l.state, l.market_status, l.market_accepted),
        (ListingState::Cancelling, Some(MarketStatus::Listed), true)
    );
}

/// Put `f` (Listed, on the market) into Cancelling with its cancel draft
/// `cd` in `status`, cancel txid `c1…` (stored on the draft once sent).
fn cancelling(f: &Fx, status: &str) {
    queries::insert_tx_draft(
        &f.conn,
        "cd",
        PROFILE,
        crate::noncustodial::shakedex::cancel::CANCEL_ACTION,
        "00",
        "{}",
        "{}",
    )
    .unwrap();
    let sent = !queries::never_sent(status);
    let c1 = txid("c1");
    queries::update_tx_draft_status(&f.conn, "cd", status, None, sent.then_some(c1.as_str()))
        .unwrap();
    f.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'cancelling', cancel_draft_id = 'cd',
             cancel_txid = ?1",
            [txid("c1")],
        )
        .unwrap();
}

/// `f`'s cancel `c1…` mined: the after-lock job's CancelAwaitingFinalize
/// (`queries::mark_listing_cancel_mined`), then `state` as given.
fn cancel_mined(f: &Fx, state: ListingState) {
    queries::update_tx_draft_status(&f.conn, "cd", "confirmed", None, Some(&txid("c1"))).unwrap();
    assert_eq!(
        queries::mark_listing_cancel_mined(&f.conn, &f.id, (&txid("c1"), 0), (&txid("f1"), 0))
            .unwrap(),
        1
    );
    f.conn
        .execute("UPDATE shakedex_listings SET state = ?1", [state.as_str()])
        .unwrap();
}

/// The market's `refresh-status` of NAME, answering `status` `body`, seen
/// `hits` times; the bodies sent.
async fn report_mock(
    s: &mut ServerGuard,
    status: usize,
    body: &'static str,
    hits: usize,
) -> (mockito::Mock, Seen) {
    let (m, seen) = recording(
        s.mock(
            "POST",
            format!("/api/v2/listings/{NAME}/refresh-status").as_str(),
        ),
        status,
        body,
    );
    (m.expect(hits).create_async().await, seen)
}

/// R28: while the cancel is unsent the listing is still kept on the market
/// (carry 3) and not reported. Once the cancel is sent, R24 and R25 stop:
/// no copy fetched, nothing uploaded, then or later; nothing is reported
/// either while the cancel is in no block (a purchase may still beat it).
/// Once our node has the cancel mined, the market is told once —
/// `refresh-status` with the mined cancel's txid — and never again.
#[tokio::test]
async fn cancelled_listing_reports_and_stops() {
    let f = listed_on_market(MarketStatus::Listed);
    cancelling(&f, "signed");
    let mut s = market().await;
    let get = copy_mock(&mut s, our_copy(&f), 1).await;
    let (report, seen) = report_mock(&mut s, 200, CANCEL_RECORDED, 1).await;
    let (up, _) = upload_mock(&mut s, 0).await;
    let n = node(&f, TIP);
    // Unsent: still kept, checked like any listed one, not reported.
    keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW, &LockLost::default())
        .await
        .unwrap();
    publish(&f, &n, &s, NOW).await;
    get.assert_async().await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "an unsent cancel is not reported"
    );
    // Sent, not mined: the jobs stop, nothing is reported yet.
    queries::update_tx_draft_status(&f.conn, "cd", "broadcasted", None, Some(&txid("c1"))).unwrap();
    for at in [NOW + 3_600, NOW + 7_200] {
        publish(&f, &n, &s, at).await;
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, at, &LockLost::default())
            .await
            .unwrap();
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "a cancel in no block is not reported"
    );
    // Mined: reported once, then nothing more.
    cancel_mined(&f, ListingState::CancelAwaitingFinalize);
    for at in [NOW + 10_800, NOW + 86_400, NOW + 7 * 86_400] {
        publish(&f, &n, &s, at).await;
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, at, &LockLost::default())
            .await
            .unwrap();
    }
    report.assert_async().await;
    get.assert_async().await;
    up.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(
        sent,
        json!({ "outcome": "cancelled", "cancelTxHash": txid("c1") })
    );
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at, l.market_error),
        (Some(MarketStatus::Reported), None, None)
    );
}

/// R28 (T6): a cancel is reported in every state our node has it mined in
/// (CancelAwaitingFinalize, CancelFinalizing, Cancelled), never before
/// (Cancelling with the cancel sent) and never from a purchase in the
/// mempool (SalePending).
#[tokio::test]
async fn only_a_mined_cancel_or_sale_is_reported() {
    for (state, hits) in [
        (ListingState::CancelAwaitingFinalize, 1),
        (ListingState::CancelFinalizing, 1),
        (ListingState::Cancelled, 1),
        (ListingState::Cancelling, 0),
        (ListingState::SalePending, 0),
    ] {
        let f = listed_on_market(MarketStatus::Listed);
        cancelling(&f, "broadcasted");
        match state {
            ListingState::Cancelling => {}
            ListingState::SalePending => {
                f.conn
                    .execute(
                        "UPDATE shakedex_listings SET state = 'sale_pending', sold_txid = ?1",
                        [txid("b1")],
                    )
                    .unwrap();
            }
            _ => cancel_mined(&f, state),
        }
        let mut s = market().await;
        let (m, seen) = report_mock(&mut s, 200, CANCEL_RECORDED, hits).await;
        publish(&f, &node(&f, TIP), &s, NOW).await;
        m.assert_async().await;
        if hits == 1 {
            let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
            assert_eq!(
                sent,
                json!({ "outcome": "cancelled", "cancelTxHash": txid("c1") }),
                "{state:?}"
            );
            assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
        } else {
            assert_eq!(
                listing(&f).market_status,
                Some(MarketStatus::Listed),
                "{state:?}"
            );
        }
    }
}

/// R28 (T6): only a published listing the market was told about is
/// reported; a listing it never heard of (status unset) is not.
#[tokio::test]
async fn untold_or_unpublished_listing_is_not_reported() {
    for case in ["untold", "unpublished"] {
        let f = listed_on_market(MarketStatus::Listed);
        let sql = match case {
            "untold" => {
                "UPDATE shakedex_listings SET market_status = NULL, market_accepted = 0, \
                 market_told = 0"
            }
            _ => "UPDATE shakedex_listings SET publish = 0",
        };
        f.conn.execute(sql, []).unwrap();
        cancelling(&f, "broadcasted");
        cancel_mined(&f, ListingState::Cancelled);
        let mut s = market().await;
        let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
        publish(&f, &node(&f, TIP), &s, NOW).await;
        any.assert_async().await;
    }
}

/// R28: no answer, or the market's "not seen yet" (its node has not seen
/// the cancel: chain lag, never a verdict), is retried with the backoff; a
/// market that lists nothing to withdraw (its "Listing not found") is done
/// all the same: Reported, with a note.
#[tokio::test]
async fn a_cancel_the_market_cannot_find_is_reported_and_no_answer_is_retried() {
    let f = listed_on_market(MarketStatus::Listed);
    cancelling(&f, "broadcasted");
    cancel_mined(&f, ListingState::CancelAwaitingFinalize);
    let path = format!("/api/v2/listings/{NAME}/refresh-status");
    let mut s = market().await;
    let down = s
        .mock("POST", path.as_str())
        .with_status(503)
        .with_body("<html>unavailable</html>")
        .expect(1)
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    // Not due before the backoff ends.
    publish(&f, &node(&f, TIP), &s, NOW + 299).await;
    down.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at.as_deref()),
        (
            Some(MarketStatus::Retrying),
            Some(rfc3339(NOW + 300).as_str())
        )
    );
    down.remove_async().await;
    let (lag, _) = report_mock(&mut s, 404, TX_NOT_SEEN, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW + 300).await;
    lag.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (
            l.market_status,
            l.market_retry_at.as_deref(),
            l.market_attempts
        ),
        (
            Some(MarketStatus::Retrying),
            Some(rfc3339(NOW + 900).as_str()),
            2
        ),
        "not seen yet backs off, never a refusal"
    );
    assert!(l
        .market_error
        .unwrap()
        .contains("Transaction was not found"));
    lag.remove_async().await;
    let (nf, _) = report_mock(&mut s, 404, NO_LISTING, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW + 900).await;
    publish(&f, &node(&f, TIP), &s, NOW + 86_400).await;
    nf.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at, l.market_attempts),
        (Some(MarketStatus::Reported), None, 0)
    );
    assert!(l.market_error.unwrap().contains("Listing not found"));
}

/// R28/R22 (T6): a mined sale is reported with its txid, once.
#[tokio::test]
async fn sold_listing_reports_its_sale() {
    let f = listed_on_market(MarketStatus::Listed);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
            [txid("b1")],
        )
        .unwrap();
    let mut s = market().await;
    let (m, seen) = report_mock(&mut s, 200, SALE_RECORDED, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    publish(&f, &node(&f, TIP), &s, NOW + 86_400).await;
    m.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(sent, json!({ "saleTxHash": txid("b1") }));
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
}

/// Ruling 2026-10-10, code-review round: the market's own refusal of a
/// report (its JSON 400) leaves nothing more to tell: the report is
/// recorded as Reported with the market's words, and not sent again.
#[tokio::test]
async fn refused_report_is_not_retried() {
    let f = listed_on_market(MarketStatus::Listed);
    cancelling(&f, "broadcasted");
    cancel_mined(&f, ListingState::CancelAwaitingFinalize);
    let mut s = market().await;
    let (m, _) = report_mock(
        &mut s,
        400,
        r#"{"error":"Transaction does not spend this listing's Shakedex locking coin","url":"/listing/dexjobs"}"#,
        1,
    )
    .await;
    for at in [NOW, NOW + 21_600, NOW + 7 * 86_400] {
        publish(&f, &node(&f, TIP), &s, at).await;
    }
    m.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at),
        (Some(MarketStatus::Reported), None)
    );
    assert!(l
        .market_error
        .unwrap()
        .contains("does not spend this listing"));
}

/// `f` (Listed, accepted on the market) after a later upload the market
/// refused: keep-listed finds its copy gone, and the market answers the
/// upload with its own 409.
async fn upload_refused(f: &Fx) {
    let mut s = market().await;
    let _get = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .create_async()
        .await;
    let _refuse = s
        .mock("POST", "/api/upload-proof")
        .with_status(409)
        .with_body(r#"{"error":"An active listing for dexjobs already exists."}"#)
        .create_async()
        .await;
    keep(f, &s, NOW).await;
    let l = listing(f);
    assert_eq!(
        (l.market_status, l.market_accepted),
        (Some(MarketStatus::Refused), true)
    );
}

/// S1 (code-review round): a listing the market accepted is still told of
/// its mined cancel after a later upload was refused — the market holds
/// our earlier copy. Exactly one `refresh-status`.
#[tokio::test]
async fn refused_upload_listing_still_reports_its_mined_cancel() {
    let f = listed_on_market(MarketStatus::Listed);
    upload_refused(&f).await;
    cancelling(&f, "broadcasted");
    cancel_mined(&f, ListingState::CancelAwaitingFinalize);
    let mut s = market().await;
    let (m, seen) = report_mock(&mut s, 200, CANCEL_RECORDED, 1).await;
    for at in [NOW, NOW + 3_600, NOW + 7 * 86_400] {
        publish(&f, &node(&f, TIP), &s, at).await;
    }
    m.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(
        sent,
        json!({ "outcome": "cancelled", "cancelTxHash": txid("c1") })
    );
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
}

/// S1, the sale: an accepted listing whose later upload was refused is
/// told of its mined sale, once.
#[tokio::test]
async fn refused_upload_listing_still_reports_its_sale() {
    let f = listed_on_market(MarketStatus::Listed);
    upload_refused(&f).await;
    f.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
            [txid("b1")],
        )
        .unwrap();
    let mut s = market().await;
    let (m, seen) = report_mock(&mut s, 200, SALE_RECORDED, 1).await;
    for at in [NOW, NOW + 3_600, NOW + 7 * 86_400] {
        publish(&f, &node(&f, TIP), &s, at).await;
    }
    m.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(sent, json!({ "saleTxHash": txid("b1") }));
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
}

/// S1: a listing whose steps never verified was never sent: the market
/// holds nothing of it (never accepted, no pending post), so its mined
/// cancel is not reported.
#[tokio::test]
async fn never_sent_steps_unverified_listing_does_not_report_its_mined_cancel() {
    let f = fx(ListingState::Listed, true);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_status = ?1, market_retry_at = ?2",
            [MarketStatus::StepsUnverified.as_str(), &rfc3339(NOW)],
        )
        .unwrap();
    cancelling(&f, "broadcasted");
    cancel_mined(&f, ListingState::Cancelled);
    let mut s = market().await;
    let any = s.mock("POST", Matcher::Any).expect(0).create_async().await;
    for at in [NOW, NOW + 7 * 86_400] {
        publish(&f, &node(&f, TIP), &s, at).await;
    }
    any.assert_async().await;
    assert_eq!(
        listing(&f).market_status,
        Some(MarketStatus::StepsUnverified)
    );
}

/// S1: a listing the market knows only from its day-0 pending post is told
/// of its mined sale; a report with no answer is Retrying, backed off (the
/// sticky `market_told` keeps it in the report set), and is reported once
/// the market answers.
#[tokio::test]
async fn pending_only_listing_reports_its_sale_after_no_answer() {
    let f = fx(ListingState::Finalizing, true);
    let mut s = market().await;
    let post = s
        .mock("POST", "/api/v2/pending-listings")
        .with_status(201)
        .with_body(PENDING_ACCEPTED)
        .expect(1)
        .create_async()
        .await;
    publish(&f, &MockNodeRpc::new(), &s, NOW).await;
    post.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_told),
        (Some(MarketStatus::Pending), true)
    );
    f.conn
        .execute(
            "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
            [txid("b1")],
        )
        .unwrap();
    let down = s
        .mock(
            "POST",
            format!("/api/v2/listings/{NAME}/refresh-status").as_str(),
        )
        .with_status(503)
        .with_body("<html>unavailable</html>")
        .expect(1)
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    publish(&f, &node(&f, TIP), &s, NOW + 299).await;
    down.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (
            l.market_status,
            l.market_retry_at.as_deref(),
            l.market_attempts
        ),
        (
            Some(MarketStatus::Retrying),
            Some(rfc3339(NOW + 300).as_str()),
            1
        )
    );
    assert!(l.market_error.unwrap().contains("no answer"));
    down.remove_async().await;
    let (m, _) = report_mock(&mut s, 200, SALE_RECORDED, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW + 300).await;
    publish(&f, &node(&f, TIP), &s, NOW + 86_400).await;
    m.assert_async().await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
}

/// Fix round 2: what a first upload after the pending post got.
#[derive(Debug, Clone, Copy)]
enum FirstUpload {
    /// None yet (the bookkeeping as the move into Listed left it).
    NotYet,
    NoAnswer,
    Refused,
    StepsUnverified,
}

/// Through the real writes: Finalizing, the pending post accepted (told),
/// the FINALIZE into the lock mined (`mark_listing_listed`, which starts the
/// bookkeeping over), then the first upload as `first`. Returns the listing
/// with its status as that upload left it.
async fn told_by_pending_post_then(first: FirstUpload) -> Fx {
    let f = fx(ListingState::Finalizing, true);
    let mut s = market().await;
    let _post = s
        .mock("POST", "/api/v2/pending-listings")
        .with_status(201)
        .with_body(PENDING_ACCEPTED)
        .create_async()
        .await;
    publish(&f, &MockNodeRpc::new(), &s, NOW).await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Pending));
    assert_eq!(queries::mark_listing_listed(&f.conn, &f.id).unwrap(), 1);
    let l = listing(&f);
    assert_eq!(
        (l.state, l.market_status, l.market_accepted),
        (ListingState::Listed, None, false),
        "the move into Listed starts the bookkeeping over"
    );
    let mut s = market().await;
    let (status, body) = match first {
        FirstUpload::NotYet => return f,
        FirstUpload::NoAnswer => (503, "<html>unavailable</html>"),
        FirstUpload::Refused => (
            409,
            r#"{"error":"An active listing for dexjobs already exists."}"#,
        ),
        FirstUpload::StepsUnverified => {
            f.conn
                .execute("UPDATE shakedex_listings SET steps_json = 'not json'", [])
                .unwrap();
            (201, "")
        }
    };
    let _up = s
        .mock("POST", "/api/upload-proof")
        .with_status(status)
        .with_body(body)
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    let want = match first {
        FirstUpload::NoAnswer => MarketStatus::Retrying,
        FirstUpload::Refused => MarketStatus::Refused,
        _ => MarketStatus::StepsUnverified,
    };
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_accepted),
        (Some(want), false),
        "{first:?}"
    );
    f
}

/// Fix round 2 (ruling): the market took our pending post, so it is told
/// of the mined cancel whatever the first upload after the move into Listed
/// got — none yet, no answer, refused, steps unverified: exactly one
/// `refresh-status`.
#[tokio::test]
async fn told_by_its_pending_post_reports_its_mined_cancel_whatever_the_first_upload_got() {
    for first in [
        FirstUpload::NotYet,
        FirstUpload::NoAnswer,
        FirstUpload::Refused,
        FirstUpload::StepsUnverified,
    ] {
        let f = told_by_pending_post_then(first).await;
        assert!(listing(&f).market_told, "{first:?}: sticky");
        cancelling(&f, "broadcasted");
        cancel_mined(&f, ListingState::CancelAwaitingFinalize);
        let mut s = market().await;
        let (m, seen) = report_mock(&mut s, 200, CANCEL_RECORDED, 1).await;
        for at in [NOW + 1, NOW + 3_600, NOW + 7 * 86_400] {
            publish(&f, &node(&f, TIP), &s, at).await;
        }
        m.assert_async().await;
        let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
        assert_eq!(
            sent,
            json!({ "outcome": "cancelled", "cancelTxHash": txid("c1") }),
            "{first:?}"
        );
        assert_eq!(
            listing(&f).market_status,
            Some(MarketStatus::Reported),
            "{first:?}"
        );
    }
}

/// Fix round 2, the sale: the same, for a mined purchase.
#[tokio::test]
async fn told_by_its_pending_post_reports_its_sale_whatever_the_first_upload_got() {
    for first in [
        FirstUpload::NotYet,
        FirstUpload::NoAnswer,
        FirstUpload::Refused,
        FirstUpload::StepsUnverified,
    ] {
        let f = told_by_pending_post_then(first).await;
        f.conn
            .execute(
                "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
                [txid("b1")],
            )
            .unwrap();
        let mut s = market().await;
        let (m, seen) = report_mock(&mut s, 200, SALE_RECORDED, 1).await;
        for at in [NOW + 300, NOW + 3_600, NOW + 7 * 86_400] {
            publish(&f, &node(&f, TIP), &s, at).await;
        }
        m.assert_async().await;
        let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
        assert_eq!(sent, json!({ "saleTxHash": txid("b1") }), "{first:?}");
        assert_eq!(
            listing(&f).market_status,
            Some(MarketStatus::Reported),
            "{first:?}"
        );
    }
}

/// Fix round 2: `market_told` is set by every outcome in which the market
/// took something of ours — the pending post, an accepted upload, our copy
/// served back — and by nothing else: a refused or unanswered post or
/// upload leaves it unset.
#[tokio::test]
async fn market_told_is_set_only_when_the_market_took_something_of_ours() {
    // A refused pending post: not told.
    let f = fx(ListingState::Finalizing, true);
    let mut s = market().await;
    let _post = s
        .mock("POST", "/api/v2/pending-listings")
        .with_status(409)
        .with_body(r#"{"error":"A pending listing with that transferTxHash already exists"}"#)
        .create_async()
        .await;
    publish(&f, &MockNodeRpc::new(), &s, NOW).await;
    assert_eq!(
        (listing(&f).market_status, listing(&f).market_told),
        (Some(MarketStatus::Refused), false)
    );
    // An unanswered then an accepted first upload: told only by the second.
    let f = fx(ListingState::Listed, true);
    let mut s = market().await;
    let down = s
        .mock("POST", "/api/upload-proof")
        .with_status(503)
        .with_body("<html>unavailable</html>")
        .create_async()
        .await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    assert!(!listing(&f).market_told, "no answer");
    down.remove_async().await;
    // Retrying now: keep-listed finds no copy and uploads.
    let _gone = s
        .mock("GET", format!("/listing/{NAME}/proof.json").as_str())
        .with_status(404)
        .with_body(PROOF_NOT_FOUND)
        .create_async()
        .await;
    let (m, _) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW + 300).await;
    m.assert_async().await;
    assert!(listing(&f).market_told, "upload accepted");
    // Our copy served back by a market we never uploaded to from here.
    let f = fx(ListingState::Listed, true);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_status = 'retrying', market_retry_at = ?1",
            [rfc3339(NOW)],
        )
        .unwrap();
    let mut s = market().await;
    let _get = copy_mock(&mut s, our_copy(&f), 1).await;
    keep(&f, &s, NOW).await;
    assert!(listing(&f).market_told, "our copy served back");
}

/// Carried from T5: the market was told the listing is cancelled, and then
/// the cancel died (Listed again, Uncancel): the listing is announced
/// afresh — uploaded at the next sync.
#[tokio::test]
async fn a_dead_cancel_puts_the_listing_back_on_the_market() {
    let f = listed_on_market(MarketStatus::Reported);
    assert_eq!(
        queries::uncancel_listing(&f.conn, &f.id, ListingState::Listed).unwrap(),
        0,
        "not Cancelling"
    );
    cancelling(&f, "broadcasted");
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_status = 'reported'",
            [],
        )
        .unwrap();
    assert_eq!(
        queries::uncancel_listing(&f.conn, &f.id, ListingState::Listed).unwrap(),
        1
    );
    let mut s = market().await;
    let (m, _) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    m.assert_async().await;
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Listed));
}

/// Ruling 2026-10-10 (fix round 1 of Step 6): the market serving our own
/// copy back counts as acceptance — an upload whose answer was lost (no
/// answer, Retrying) went through after all.
#[tokio::test]
async fn our_copy_served_back_counts_as_accepted() {
    let f = fx(ListingState::Listed, true);
    f.conn
        .execute(
            "UPDATE shakedex_listings SET market_status = 'retrying', market_attempts = 1",
            [],
        )
        .unwrap();
    let mut s = market().await;
    let get = copy_mock(&mut s, our_copy(&f), 1).await;
    let (up, _) = upload_mock(&mut s, 0).await;
    keep(&f, &s, NOW).await;
    get.assert_async().await;
    up.assert_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_accepted),
        (Some(MarketStatus::Listed), true)
    );
}

/// R28 (fix round 1 of Step 6): the write that records a mined cancel or a
/// mined sale clears the hourly check's `market_retry_at`, so the market is
/// told at the same or the next sync, not when the check would have been
/// due.
#[tokio::test]
async fn a_mined_cancel_or_sale_is_reported_at_the_next_sync() {
    for sale in [false, true] {
        let f = listed_on_market(MarketStatus::Listed);
        f.conn
            .execute(
                "UPDATE shakedex_listings SET market_retry_at = ?1",
                [rfc3339(NOW + 3_600)],
            )
            .unwrap();
        let body = if sale {
            assert_eq!(
                queries::sell_shakedex_listing(&f.conn, &f.id, &txid("b1"), (&txid("f1"), 0))
                    .unwrap(),
                1
            );
            SALE_RECORDED
        } else {
            cancelling(&f, "broadcasted");
            cancel_mined(&f, ListingState::CancelAwaitingFinalize);
            CANCEL_RECORDED
        };
        assert_eq!(listing(&f).market_retry_at, None, "sale {sale}");
        let mut s = market().await;
        let (m, _) = report_mock(&mut s, 200, body, 1).await;
        publish(&f, &node(&f, TIP), &s, NOW).await;
        m.assert_async().await;
        assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
    }
}

/// Ruling 2026-10-10 (fix round 2 of Step 6): a Lower price is no fresh
/// listing — the market holds this lock's listing — so it keeps the
/// acceptance and makes the new copy due for keep-listed at once. A cancel
/// signed before that re-upload still keeps the listing: the market's copy
/// (our older, dearer step) is replaced by the lowered one, which the
/// listing reads as ours (Listed, not "replaced by someone else's copy");
/// once the cancel is mined it is reported.
#[tokio::test]
async fn lowered_listing_is_kept_while_cancelling_and_then_reported() {
    let f = listed_on_market(MarketStatus::Listed);
    let old = listing(&f);
    let old_copy = our_copy(&f);
    let first = signed_step(&f.key, &f.payment, PRICE, sell::buy_now_lock_time(MTP));
    let lower = signed_step(&f.key, &f.payment, 3_000_000, sell::buy_now_lock_time(MTP));
    let file = file_of(
        &f.key,
        &f.payment,
        &[first.clone(), lower.clone()],
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    let steps = stored(&[first, lower.clone()]);
    let txid_f1 = txid("f1");
    assert_eq!(
        queries::lower_listing_price(
            &f.conn,
            &f.id,
            &queries::LoweredPrice {
                lock: (&txid_f1, 0),
                old_steps_json: &old.steps_json,
                steps_json: &steps,
                listing_file_json: &file,
                expires_at: old.expires_at.unwrap(),
            },
        )
        .unwrap(),
        1
    );
    cancelling(&f, "signed");
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_retry_at, l.market_accepted),
        (Some(MarketStatus::Retrying), None, true)
    );
    let mut s = market().await;
    let get = copy_mock(&mut s, old_copy, 1).await;
    let (up, seen) = upload_mock(&mut s, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW).await;
    keep(&f, &s, NOW).await;
    get.assert_async().await;
    up.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(ListingFile::parse(&sent, NET).unwrap().steps, vec![lower]);
    let l = listing(&f);
    assert_eq!(
        (l.state, l.market_status),
        (ListingState::Cancelling, Some(MarketStatus::Listed))
    );
    // Sent, then mined: reported.
    queries::update_tx_draft_status(&f.conn, "cd", "broadcasted", None, Some(&txid("c1"))).unwrap();
    cancel_mined(&f, ListingState::CancelAwaitingFinalize);
    let (report, body) = report_mock(&mut s, 200, CANCEL_RECORDED, 1).await;
    publish(&f, &node(&f, TIP), &s, NOW + 60).await;
    report.assert_async().await;
    let sent: serde_json::Value = serde_json::from_slice(&body.lock().unwrap()[0]).unwrap();
    assert_eq!(
        sent,
        json!({ "outcome": "cancelled", "cancelTxHash": txid("c1") })
    );
    assert_eq!(listing(&f).market_status, Some(MarketStatus::Reported));
}

/// R25, decision 3, SECURITY.md: `run_sync_steps`, as `namehold-syncd` runs
/// it (and as the app does), uploads a published Listed listing — the step
/// signed at Finalize & sign, byte for byte — and makes no send: no
/// `sendrawtransaction` reaches hsd, no draft is created or changed, the
/// listing's steps and file are what they were. The daemon holds no key; the
/// source check `shakedex_layering_tests::shakedex_jobs_hold_no_signing_call`
/// keeps a signing call out of the jobs.
#[tokio::test]
async fn daemon_publishes_but_never_signs_or_broadcasts() {
    use crate::commands::sync::SyncCaller;
    for caller in [SyncCaller::Daemon, SyncCaller::App] {
        // Real time: run_sync_steps reads the clock itself.
        let now = chrono::Utc::now().timestamp();
        let f = fx_at(ListingState::Listed, true, (now - 600) as u64);
        let before = listing(&f);
        let (after, seen) = sync_on_a_copy(&f, caller, now - 600, 1).await;
        assert_eq!(
            after.market_status,
            Some(MarketStatus::Listed),
            "{caller:?}"
        );
        let (_, sent) = proof_part(&seen[0]);
        let stored = ListingFile::parse(before.listing_file_json.as_deref().unwrap(), NET).unwrap();
        assert_eq!(
            ListingFile::parse(&sent, NET).unwrap().steps,
            stored.steps,
            "the signed step as stored"
        );
    }
}

/// R24, decision 3, SECURITY.md: `run_sync_steps`, as the daemon and as the
/// app run it, uploads a reverse auction's moved step once — signed at
/// Finalize & sign, byte for byte, keep-listed after it not uploading it
/// again — and makes no send: no `sendrawtransaction`, no
/// draft created or changed, the steps and file as they were.
#[tokio::test]
async fn daemon_advances_a_reverse_auction_but_never_signs_or_broadcasts() {
    use crate::commands::sync::SyncCaller;
    for caller in [SyncCaller::Daemon, SyncCaller::App] {
        // Real time: run_sync_steps reads the clock itself.
        let now = chrono::Utc::now().timestamp();
        let mtp0 = (now - 600) as u64;
        let f = fx_at(ListingState::Listed, true, mtp0);
        let schedule = schedule_at(mtp0);
        let steps = as_schedule_at(&f, &schedule, mtp0);
        // The market accepted step 2, current at `mtp0`; the hourly check is
        // due too, so keep-listed runs in the same sync after advance: one
        // upload in all (advance's; keep-listed then finds it not due).
        f.conn
            .execute(
                "UPDATE shakedex_listings SET market_status = 'listed', market_sent_step = ?1,
                 market_retry_at = ?2, market_accepted = 1, market_told = 1",
                [hex::encode(steps[2].signature), rfc3339(now - 60)],
            )
            .unwrap();
        let mtp = i64::try_from(valid_from_mtp(schedule[3].1)).unwrap();
        let (after, seen) = sync_on_a_copy(&f, caller, mtp, 1).await;
        let (_, sent) = proof_part(&seen[0]);
        assert_eq!(
            ListingFile::parse(&sent, NET).unwrap().steps,
            vec![steps[3].clone()],
            "{caller:?}: step 3, as signed"
        );
        assert_eq!(after.market_status, Some(MarketStatus::Listed));
        assert_eq!(
            after.market_sent_step,
            Some(hex::encode(steps[3].signature)),
            "{caller:?}"
        );
    }
}

/// Run `run_sync_steps` as `caller` on a copy of `f`'s database, against an
/// hsd answering the median time `mtp` and `f`'s lock coin (and refusing
/// to see a `sendrawtransaction`: expected 0), and a market accepting
/// `uploads` uploads. Asserts no send, no draft created or changed, and
/// the listing's steps and file as they were; returns the listing after
/// and the bodies uploaded.
async fn sync_on_a_copy(
    f: &Fx,
    caller: crate::commands::sync::SyncCaller,
    mtp: i64,
    uploads: usize,
) -> (ShakedexListing, Vec<Vec<u8>>) {
    use crate::commands::sync::{run_sync_steps, SyncStatus};
    static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut hsd = mockito::Server::new_async().await;
    let mut s = market().await;
    let path = std::env::temp_dir().join(format!(
        "namehold_publish_{}_{caller:?}_{}.db",
        std::process::id(),
        COPIES.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_file(&path);
    let db_path = path.to_str().unwrap().to_string();
    queries::set_setting(&f.conn, "node_rpc_url", &hsd.url()).unwrap();
    queries::set_setting(&f.conn, "learnhns_base_url", &s.url()).unwrap();
    f.conn
        .execute("VACUUM INTO ?1", rusqlite::params![db_path])
        .unwrap();
    let rpc = |srv: &mut ServerGuard, method: &str, result: serde_json::Value| {
        srv.mock("POST", "/")
            .match_body(Matcher::PartialJson(json!({ "method": method })))
            .with_header("content-type", "application/json")
            .with_body(crate::tests::shakedex_cmd_tests::rpc_ok(result))
    };
    let _chain = rpc(
        &mut hsd,
        "getblockchaininfo",
        json!({
            "chain": "main", "blocks": TIP, "headers": TIP,
            "verificationprogress": 1.0, "mediantime": mtp
        }),
    )
    .create_async()
    .await;
    let _coin = hsd
        .mock("GET", format!("/coin/{}/0", txid("f1")).as_str())
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 0, "height": TIP - 5, "value": LOCK_VALUE, "address": f.lock,
                "covenant": { "type": COV_FINALIZE, "action": "FINALIZE",
                              "items": [hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap()), "32000000"] },
                "coinbase": false, "hash": txid("f1"), "index": 0
            })
            .to_string(),
        )
        .create_async()
        .await;
    let send = hsd
        .mock("POST", "/")
        .match_body(Matcher::Regex("sendrawtransaction".into()))
        .expect(0)
        .create_async()
        .await;
    let (up, seen) = upload_mock(&mut s, uploads).await;
    let before = listing(f);
    let drafts = |c: &Connection| -> Vec<(String, String)> {
        let mut st = c
            .prepare("SELECT id, status FROM wallet_tx_drafts ORDER BY id")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let drafts_before = drafts(&f.conn);
    let status = std::sync::Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));
    run_sync_steps(&status, &db_path, PROFILE, caller).await;
    send.assert_async().await;
    up.assert_async().await;
    let after_conn = Connection::open(&db_path).unwrap();
    let after = queries::get_shakedex_listing(&after_conn, &f.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            after.steps_json.as_str(),
            after.listing_file_json.as_deref()
        ),
        (
            before.steps_json.as_str(),
            before.listing_file_json.as_deref()
        ),
        "{caller:?}: the steps and file as they were"
    );
    assert_eq!(
        drafts(&after_conn),
        drafts_before,
        "{caller:?}: no draft created or changed"
    );
    drop(after_conn);
    let _ = std::fs::remove_file(&path);
    let bodies = seen.lock().unwrap().clone();
    (after, bodies)
}

/// Lower `f`'s price to 3 HNS (a second step, `queries::lower_listing_price`);
/// the new step.
fn lower_price(f: &Fx) -> PriceStep {
    let old = listing(f);
    let first = signed_step(&f.key, &f.payment, PRICE, sell::buy_now_lock_time(MTP));
    let lower = signed_step(&f.key, &f.payment, 3_000_000, sell::buy_now_lock_time(MTP));
    let file = file_of(
        &f.key,
        &f.payment,
        &[first.clone(), lower.clone()],
        MTP + sell::LISTING_LIFETIME_SECS,
    );
    let steps = stored(&[first, lower.clone()]);
    let txid_f1 = txid("f1");
    assert_eq!(
        queries::lower_listing_price(
            &f.conn,
            &f.id,
            &queries::LoweredPrice {
                lock: (&txid_f1, 0),
                old_steps_json: &old.steps_json,
                steps_json: &steps,
                listing_file_json: &file,
                expires_at: old.expires_at.unwrap(),
            },
        )
        .unwrap(),
        1
    );
    lower
}

/// Fix round 3 of Step 6: a Lower price sets `market_changed`, which a
/// failure leaves alone. After no answer, the next run still reads the
/// market's differing copy as our own older one: uploaded over it, Listed
/// (not "replaced by someone else's copy"), the flag cleared. A differing
/// copy found later, with the flag clear, is someone else's:
/// ReplacedReuploaded.
#[tokio::test]
async fn lowered_listing_after_no_answer_is_uploaded_over_our_older_copy() {
    let f = listed_on_market(MarketStatus::Listed);
    let old_copy = our_copy(&f);
    let lower = lower_price(&f);
    assert!(listing(&f).market_changed);
    let mut s = market().await;
    let path = format!("/listing/{NAME}/proof.json");
    let down = s
        .mock("GET", path.as_str())
        .with_status(503)
        .with_body("<html>unavailable</html>")
        .expect(1)
        .create_async()
        .await;
    keep(&f, &s, NOW).await;
    down.assert_async().await;
    down.remove_async().await;
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_attempts, l.market_changed),
        (Some(MarketStatus::Retrying), 1, true),
        "a failure leaves the flag"
    );
    let get = copy_mock(&mut s, old_copy, 1).await;
    let (up, seen) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW + 300).await;
    get.assert_async().await;
    up.assert_async().await;
    let (_, sent) = proof_part(&seen.lock().unwrap()[0]);
    assert_eq!(ListingFile::parse(&sent, NET).unwrap().steps, vec![lower]);
    let l = listing(&f);
    assert_eq!(
        (l.market_status, l.market_changed),
        (Some(MarketStatus::Listed), false)
    );
    get.remove_async().await;
    up.remove_async().await;
    // An hour later, someone else's copy (another price): replaced.
    let mut theirs: serde_json::Value = serde_json::from_str(&our_copy(&f)).unwrap();
    theirs["data"][0]["price"] = 1.into();
    let get = copy_mock(&mut s, theirs.to_string(), 1).await;
    let (up, _) = upload_mock(&mut s, 1).await;
    keep(&f, &s, NOW + 300 + 3_600).await;
    get.assert_async().await;
    up.assert_async().await;
    assert_eq!(
        listing(&f).market_status,
        Some(MarketStatus::ReplacedReuploaded)
    );
}

/// Fix round 3 of Step 6: a reorg back to an unsent cancel sets
/// `market_changed` (what the market was told changed: it may hold a copy
/// marked cancelled), so a differing copy is read as our own: Listed. With
/// the flag clear, the same differing copy is someone else's.
#[tokio::test]
async fn reorg_reset_listing_reads_a_differing_copy_by_its_flag() {
    for flag in [true, false] {
        let f = listed_on_market(MarketStatus::Reported);
        queries::insert_tx_draft(&f.conn, "cd", PROFILE, "x", "00", "{}", "{}").unwrap();
        queries::update_tx_draft_status(&f.conn, "cd", "signed", None, Some(&txid("c1"))).unwrap();
        f.conn
            .execute(
                "UPDATE shakedex_listings SET state = 'cancel_awaiting_finalize',
                 cancel_draft_id = 'cd', cancel_txid = ?1, cancel_vout = 0",
                [txid("c1")],
            )
            .unwrap();
        assert_eq!(
            queries::mark_listing_cancel_unmined(&f.conn, &f.id, &txid("c1")).unwrap(),
            1
        );
        assert!(listing(&f).market_changed);
        if !flag {
            f.conn
                .execute("UPDATE shakedex_listings SET market_changed = 0", [])
                .unwrap();
        }
        let mut theirs: serde_json::Value = serde_json::from_str(&our_copy(&f)).unwrap();
        theirs["data"][0]["price"] = 1.into();
        let mut s = market().await;
        let get = copy_mock(&mut s, theirs.to_string(), 1).await;
        let (up, _) = upload_mock(&mut s, 1).await;
        keep(&f, &s, NOW).await;
        get.assert_async().await;
        up.assert_async().await;
        let want = if flag {
            MarketStatus::Listed
        } else {
            MarketStatus::ReplacedReuploaded
        };
        let l = listing(&f);
        assert_eq!(
            (l.state, l.market_status, l.market_changed),
            (ListingState::Cancelling, Some(want), false),
            "flag {flag}"
        );
    }
}

/// The market check sites of the three jobs (`market_lock_lost`), one
/// listing each would send otherwise.
#[derive(Clone, Copy, Debug)]
enum Job {
    /// Publish: a mined sale reported.
    Report,
    /// Publish: the day-0 pending listing.
    Announce,
    /// Publish: the first upload of a Listed listing.
    FirstUpload,
    /// Advance: a reverse auction whose step moved.
    Advance,
    /// Keep listed: the hourly check of the market's copy.
    Keep,
    /// Keep listed: a Refused listing's expiry refresh (no market call, a
    /// bookkeeping write).
    RefreshRefused,
}

impl Job {
    const ALL: [Job; 6] = [
        Job::Report,
        Job::Announce,
        Job::FirstUpload,
        Job::Advance,
        Job::Keep,
        Job::RefreshRefused,
    ];

    /// The method of the market request the job sends; `None` for one that
    /// only writes its bookkeeping.
    fn method(self) -> Option<&'static str> {
        match self {
            Job::Keep => Some("GET"),
            Job::RefreshRefused => None,
            _ => Some("POST"),
        }
    }

    /// A listing the job acts on, and the wall clock and median time it
    /// runs at.
    async fn fixture(self) -> (Fx, i64, u64) {
        match self {
            Job::Report => {
                let f = listed_on_market(MarketStatus::Listed);
                f.conn
                    .execute(
                        "UPDATE shakedex_listings SET state = 'sold', sold_txid = ?1",
                        [txid("b1")],
                    )
                    .unwrap();
                (f, NOW, MTP)
            }
            Job::Announce => {
                let f = fx(ListingState::Locking, true);
                queries::update_tx_draft_status(
                    &f.conn,
                    "lockd",
                    "broadcasted",
                    None,
                    Some(&txid("e1")),
                )
                .unwrap();
                (f, NOW, MTP)
            }
            Job::FirstUpload => (fx(ListingState::Listed, true), NOW, MTP),
            Job::Advance => {
                let (f, _) = auction_on_market().await;
                let (mtp, later) = moved();
                (f, later, mtp)
            }
            Job::Keep => (listed_on_market(MarketStatus::Listed), NOW, MTP),
            Job::RefreshRefused => {
                let f = listed_on_market(MarketStatus::Refused);
                f.conn
                    .execute("UPDATE shakedex_listings SET market_retry_at = NULL", [])
                    .unwrap();
                set_expiry(&f, (NOW + 86_400) as u64);
                (f, NOW, MTP)
            }
        }
    }

    async fn run(self, f: &Fx, s: &ServerGuard, now: i64, mtp: u64, lock: &LockLost) {
        let n = node_at(f, TIP, mtp);
        let m = client(s);
        match self {
            Job::Report | Job::Announce | Job::FirstUpload => {
                publish_listings_with_client(&f.conn, &n, &m, PROFILE, now, lock).await
            }
            Job::Advance => {
                advance_reverse_auctions_with_client(&f.conn, &n, &m, PROFILE, now, lock).await
            }
            Job::Keep | Job::RefreshRefused => {
                keep_listed_with_client(&f.conn, &n, &m, PROFILE, now, lock).await
            }
        }
        .unwrap();
    }
}

/// The market bookkeeping of `f`'s listing.
fn bookkeeping(f: &Fx) -> (Option<MarketStatus>, Option<String>, Option<String>) {
    let l = listing(f);
    (l.market_status, l.market_retry_at, l.market_sent_step)
}

/// How a run's sync lock stands.
#[derive(Clone, Copy, Debug)]
enum Lock {
    /// Ours (no row: the job runs outside a sync).
    Held,
    /// The heartbeat raised `LockLost`.
    Raised,
    /// Another process's row, the heartbeat not run yet.
    OtherOwner,
    /// This process's own row: held, as a sync holds it.
    OwnRow,
    /// The lock row cannot be read (the table is gone).
    Unreadable,
}

/// One market job under `lock`: whether the market saw a request of the
/// job's method, and whether its bookkeeping changed. The market answers
/// every request with a 502 (no answer): a request is counted, not taken.
async fn job_under(job: Job, lock: Lock) -> (bool, bool, LockLost) {
    let (f, now, mtp) = job.fixture().await;
    let flag = LockLost::default();
    match lock {
        Lock::Held => {}
        Lock::Raised => flag.raise(),
        Lock::OtherOwner => {
            f.conn
                .execute(
                    "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
                     VALUES (?1, 99999, 'app', 0, 0)",
                    [PROFILE],
                )
                .unwrap();
        }
        Lock::OwnRow => {
            f.conn
                .execute(
                    "INSERT INTO sync_locks (profile_id, owner_pid, owner_type, acquired_at, heartbeat_at)
                     VALUES (?1, ?2, 'app', 0, 0)",
                    rusqlite::params![PROFILE, std::process::id()],
                )
                .unwrap();
        }
        Lock::Unreadable => {
            f.conn.execute_batch("DROP TABLE sync_locks").unwrap();
        }
    }
    let before = bookkeeping(&f);
    let mut s = market().await;
    // Counted by hand: mockito's `matched()` is "exactly one hit".
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let _seen = match job.method() {
        Some(method) => {
            let hits = hits.clone();
            Some(
                s.mock(method, Matcher::Any)
                    .with_status(502)
                    .with_body_from_request(move |_| {
                        hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        b"<html>502 Bad Gateway</html>".to_vec()
                    })
                    .create_async()
                    .await,
            )
        }
        None => None,
    };
    job.run(&f, &s, now, mtp, &flag).await;
    let called = hits.load(std::sync::atomic::Ordering::SeqCst) > 0;
    (called, bookkeeping(&f) != before, flag)
}

/// A sync whose lock another process took over makes no market call and
/// writes no market bookkeeping, at every check site of the three jobs:
/// each stops before the listing it would otherwise send, and leaves it to
/// the process that holds the lock. Lost is the heartbeat's flag, or the
/// lock row naming another process before the heartbeat has seen it (which
/// raises the flag, so the rest of the sync stops too). A lock row that
/// cannot be read stops the job the same way but is no takeover: the flag
/// stays down. Each site's controls, with the lock ours (no row, or our own
/// row), do reach the market (or, for the expiry refresh, write its
/// bookkeeping) and leave the flag down.
#[tokio::test]
async fn market_jobs_stop_before_a_listing_once_the_sync_lock_is_lost() {
    for job in Job::ALL {
        for lock in [Lock::Held, Lock::OwnRow] {
            let (called, changed, flag) = job_under(job, lock).await;
            assert!(
                called || (job.method().is_none() && changed),
                "{job:?} {lock:?}: the control acts"
            );
            assert!(!flag.is_lost(), "{job:?} {lock:?}: still ours");
        }
        for lock in [Lock::Raised, Lock::OtherOwner, Lock::Unreadable] {
            let (called, changed, flag) = job_under(job, lock).await;
            assert!(!called, "{job:?} {lock:?}: no market call");
            assert!(!changed, "{job:?} {lock:?}: no bookkeeping written");
            let takeover = !matches!(lock, Lock::Unreadable);
            assert_eq!(
                flag.is_lost(),
                takeover,
                "{job:?} {lock:?}: only a takeover stops the sync"
            );
        }
    }
}
