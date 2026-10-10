//! The market jobs (T6): R23's publishing, R25's keeping listed, R28's
//! reports. The node is `MockNodeRpc`; LearnHNS Market is a mockito server
//! answering in the market's own shapes (`learnhns_tests`' constants, copied
//! from its source; no live write is ever recorded).

use mockito::{Matcher, ServerGuard};
use rusqlite::Connection;
use serde_json::json;

use crate::db::queries::{self, ListingMode, ListingState, MarketStatus, ShakedexListing};
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
use crate::noncustodial::shakedex::template::StepTemplate;
use crate::noncustodial::sync::COV_FINALIZE;
use crate::noncustodial::tx::output_address_from_string;
use crate::shakedex_jobs::{keep_listed_with_client, publish_listings_with_client};
use crate::tests::learnhns_tests::{
    proof_part, recording, Seen, COIN_NOT_SEEN, PENDING_ACCEPTED, UPLOAD_ACCEPTED,
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
        let step = signed_step(&key, &payment, PRICE, sell::buy_now_lock_time(MTP));
        (
            stored(std::slice::from_ref(&step)),
            Some(file_of(
                &key,
                &payment,
                &[step],
                MTP + sell::LISTING_LIFETIME_SECS,
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
        expires_at: locked.then_some((MTP + sell::LISTING_LIFETIME_SECS) as i64),
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
    let coin: NodeCoin = serde_json::from_value(json!({
        "version": 0, "height": height, "value": LOCK_VALUE, "address": f.lock,
        "covenant": { "type": COV_FINALIZE, "action": "FINALIZE",
                      "items": [hex::encode(crate::noncustodial::names::hash_name(NAME).unwrap()), "32000000"] },
        "coinbase": false, "hash": txid("f1"), "index": 0
    }))
    .unwrap();
    MockNodeRpc::new()
        .with_blockchain_info(tip())
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
    publish_listings_with_client(&f.conn, node, &client(s), PROFILE, now)
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
    // Announced on day 0 (Pending): Pending is a status the first upload
    // takes, so only the state keeps a Finalizing listing off the market.
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
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW)
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
        keep_listed_with_client(&f.conn, &n, &client(&s), PROFILE, NOW)
            .await
            .unwrap();
        any.assert_async().await;
        get.assert_async().await;
        assert_eq!(listing(&f).market_status, None);
    }
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

/// Deviation 9: a reverse auction gets its pending post on day 0
/// (`listingMode: "reverse-auction"`) but no upload once Listed (T8's).
#[tokio::test]
async fn reverse_auction_gets_the_pending_post_only() {
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

    let g = fx(ListingState::Listed, true);
    g.conn
        .execute("UPDATE shakedex_listings SET mode = 'reverse_auction'", [])
        .unwrap();
    let mut s = market().await;
    let (m, _) = upload_mock(&mut s, 0).await;
    let n = node(&g, TIP);
    publish(&g, &n, &s, NOW).await;
    m.assert_async().await;
    assert_eq!(listing(&g).market_status, None);
    assert_eq!(n.call_count(), 0);
}

fn rfc3339(secs: i64) -> String {
    crate::shakedex_jobs::rfc3339(secs)
}
