//! A whole trade, end to end, through the code the app and the daemon run
//! (spec §3.3): an offer, a take, both answers through the relay, both
//! Commits signed with the real Commit code, and every later step taken by
//! the exchange job alone (`job::tick`): the Seller's daemon proving the BTC
//! lock, the Buyer's revealing on HNS, the Seller's claiming on Bitcoin.
//!
//! HNS is a real regtest hsd; Bitcoin is `exchange_fake_btc::FakeBtc`, an
//! in-memory chain with real headers that runs every transaction through
//! Bitcoin Core's interpreter. **Skipped unless `HNS_IT_NODE_URL` is set**:
//!
//! ```sh
//! bash scripts/regtest.sh start
//! HNS_IT_NODE_URL=http://127.0.0.1:14037 HNS_IT_NODE_API_KEY=test \
//!   cargo test --manifest-path src-tauri/Cargo.toml live_exchange_e2e -- --nocapture --test-threads=1
//! ```

use rusqlite::Connection;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use super::exchange_fake_btc::{FakeBtc, FakeRelay};
use super::exchange_fixture as fx;
use crate::exchange_io::btc_source::BtcSource;
use crate::exchange_io::job::{self, Ctx};
use crate::exchange_io::meeting::{MeetingPoint, Watchtower};
use crate::exchange_io::store::{self, Trade};
use crate::exchange_io::{btc_wallet, runner};
use crate::noncustodial::address;
use crate::noncustodial::exchange::engine::{Role, Status};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::relay_msg::{
    self, Body, LockAnnouncement, Message, Offer, Side, TakeRequest,
};
use crate::noncustodial::exchange::{btc_account, btc_htlc, commit, hns_htlc, keys};
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{ChainSource, NodeRpcClient};
use crate::noncustodial::tx::{sighash, Covenant, Input, Outpoint, Output, Transaction};

const NET: Network = Network::Regtest;
const BUYER: &str = "e2e-buyer";
const SELLER: &str = "e2e-seller";

fn it_client() -> Option<NodeRpcClient> {
    let url = std::env::var("HNS_IT_NODE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let key = std::env::var("HNS_IT_NODE_API_KEY").unwrap_or_default();
    Some(NodeRpcClient::new(&url, &key, ChainSource::LocalNode))
}

fn now() -> u64 {
    crate::exchange_io::env::now_secs()
}

fn run_key(label: &str) -> SecretKey {
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    SecretKey::from_byte_array(Sha256::digest(format!("{label} {run}").as_bytes()).into()).unwrap()
}

fn hns_addr_of(k: &SecretKey) -> String {
    address::address_from_pubkey(
        NET,
        &PublicKey::from_secret_key(&Secp256k1::new(), k).serialize(),
    )
    .unwrap()
}

/// A trade-local miner address, so mining pays nobody in the test.
fn miner() -> String {
    hns_addr_of(&run_key("miner"))
}

async fn tick(
    conn: &Connection,
    profile: &str,
    node: &NodeRpcClient,
    srcs: &[Box<dyn BtcSource>],
    relay: &FakeRelay,
    watch: &FakeRelay,
) -> job::TickReport {
    let ctx = Ctx {
        conn,
        profile_id: profile,
        network: NET,
        policy: Policy::for_network(NET),
        node: Some(node as &dyn NodeRpc),
        sources: srcs,
        relay: Some(relay as &dyn MeetingPoint),
        watchtower: Some(watch as &dyn Watchtower),
        now: now(),
    };
    let r = job::tick(&ctx).await;
    for e in &r.errors {
        eprintln!("{profile}: {e}");
    }
    r
}

fn only_trade(conn: &Connection, profile: &str) -> Trade {
    let mut v = store::trades(conn, profile).unwrap();
    assert_eq!(v.len(), 1, "{profile} has one trade");
    v.remove(0)
}

fn enable(conn: &Connection, profile: &str, m: &crate::noncustodial::hd::ExtendedPrivKey) {
    store::insert_account(
        conn,
        &store::Account {
            profile_id: profile.into(),
            network: NET.as_str().into(),
            btc_xpub: btc_account::account_xpub(m, NET, 0).unwrap(),
            swap_xpub: keys::branch_xpub(m, NET, 0).unwrap(),
            account: 0,
            next_trade_index: 0,
            next_btc_receive: 0,
            next_btc_change: 0,
        },
        now(),
    )
    .unwrap();
}

#[tokio::test]
async fn live_exchange_e2e_an_honest_trade_settles_both_sides_through_the_job_alone() {
    let Some(node) = it_client() else {
        eprintln!("skip: HNS_IT_NODE_URL not set");
        return;
    };
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    let bm = fx::buyer_master();
    let sm = fx::seller_master();
    enable(&conn, BUYER, &bm);
    enable(&conn, SELLER, &sm);
    let fake = FakeBtc::new(now() as u32);
    let srcs: Vec<Box<dyn BtcSource>> = vec![Box::new(fake.clone())];
    let relay = FakeRelay::default();
    let watch = FakeRelay::default();
    let p = Policy::for_network(NET);

    // 1. The Seller's offer: up to 10 000 HNS at 20 sats per HNS.
    let offer_key = relay_msg::new_relay_key();
    let offer = Offer {
        version: 1,
        network: NET.as_str().into(),
        side: Side::Sell,
        price_msat_per_hns: 20_000,
        min_hns: 100_000_000,
        max_hns: 10_000_000_000,
        maker_relay_pubkey: relay_msg::relay_pubkey_hex(&offer_key),
        created_at: now(),
        expires_at: now() + 86_400,
        nonce: relay_msg::random_hex16(),
    }
    .sign(&offer_key)
    .unwrap();
    store::insert_offer(
        &conn,
        SELLER,
        &store::OwnOffer {
            id: offer.offer.id(),
            network: NET.as_str().into(),
            side: Side::Sell,
            signed: offer.clone(),
            relay_key: offer_key,
            live: true,
            last_published_at: None,
            last_error: None,
        },
        now(),
    )
    .unwrap();
    tick(&conn, SELLER, &node, &srcs, &relay, &watch).await;
    assert_eq!(
        relay.offers.lock().unwrap().len(),
        1,
        "the heartbeat published the offer"
    );

    // 2. The Buyer takes 1000 HNS (as `exchange_take_offer` does).
    let amount = 1_000_000_000u64;
    let acct = store::account(&conn, BUYER).unwrap().unwrap();
    let index = store::take_trade_indexes(&conn, BUYER, 1).unwrap();
    let (hpk, bpk) = keys::trade_pubkeys(&acct.swap_xpub, index).unwrap();
    let nonce = keys::new_secret_nonce();
    let req = TakeRequest {
        hns_amount: amount,
        hns_pubkey: hex::encode(hpk),
        btc_pubkey: hex::encode(bpk),
        hash: Some(hex::encode(
            keys::trade_keys(&bm, NET, 0, index).unwrap().hash(&nonce),
        )),
        start: now(),
    };
    let taker_key = relay_msg::new_relay_key();
    let trade_id = relay_msg::random_hex16();
    store::insert_trade(
        &conn,
        &Trade {
            profile_id: BUYER.into(),
            trade_id: trade_id.clone(),
            network: NET.as_str().into(),
            role: Role::Buyer,
            maker: false,
            offer: offer.clone(),
            trade_index: index,
            relay_key: taker_key,
            counterparty_key: Some(offer.offer.maker_relay_pubkey.clone()),
            hns_amount: amount,
            btc_sats: 20_000,
            price_msat: 20_000,
            terms: None,
            holdings: None,
            status: None,
            declined: None,
            last_error: None,
            watchtower: "none".into(),
            hns_lock: None,
            request: Some(req.clone()),
            secret_nonce: Some(nonce),
            terminal_at: None,
            finished: false,
            version: 0,
            created_at: now(),
            updated_at: now(),
        },
    )
    .unwrap();
    relay
        .send(
            &Message {
                version: 1,
                network: NET.as_str().into(),
                offer_id: offer.offer.id(),
                trade_id: trade_id.clone(),
                from: relay_msg::relay_pubkey_hex(&taker_key),
                to: offer.offer.maker_relay_pubkey.clone(),
                sent_at: now(),
                body: Body::Take(req),
            }
            .sign(&taker_key)
            .unwrap(),
            &taker_key,
        )
        .await
        .unwrap();

    // 3. The Seller's job answers; the Buyer's job keeps the terms.
    tick(&conn, SELLER, &node, &srcs, &relay, &watch).await;
    tick(&conn, BUYER, &node, &srcs, &relay, &watch).await;
    tick(&conn, BUYER, &node, &srcs, &relay, &watch).await;
    let bt = only_trade(&conn, BUYER);
    let terms = bt
        .terms
        .clone()
        .expect("the Buyer accepted the maker's terms");
    assert_eq!(terms.btc_amount, 20_000);
    assert!(
        matches!(bt.status, Some(Status::AwaitingBuyerCommit { .. })),
        "{:?}",
        bt.status
    );
    let st = only_trade(&conn, SELLER);
    assert_eq!(
        st.terms.as_ref().unwrap().id(),
        terms.id(),
        "both hold the same terms"
    );

    // 4. The Buyer's Commit (as `exchange_commit` does: store, then send).
    let acct = store::account(&conn, BUYER).unwrap().unwrap();
    fake.fund(
        &btc_account::address_at(&acct.btc_xpub, NET, 0, 0).unwrap(),
        1_000_000,
    );
    let coins = btc_wallet::coins(srcs[0].as_ref(), &acct, NET)
        .await
        .unwrap();
    let buyer_hns_to = hns_addr_of(&run_key("buyer payout"));
    let c = commit::buyer_commit(&commit::BuyerCommitInput {
        master: &bm,
        account: 0,
        trade_index: bt.trade_index,
        secret_nonce: &nonce,
        terms: &terms,
        policy: &p,
        btc_coins: &coins,
        btc_change_to: &btc_account::address_at(&acct.btc_xpub, NET, 1, 0).unwrap(),
        btc_refund_to: &btc_account::address_at(&acct.btc_xpub, NET, 0, 1).unwrap(),
        btc_rate: 1,
        hns_rate: 1,
        hns_to: &buyer_hns_to,
    })
    .unwrap();
    let mut bt = bt;
    bt.holdings = Some(c.holdings.clone());
    assert!(store::commit_trade(&conn, &mut bt, now()).unwrap());
    runner::send_btc(&srcs, &btc_htlc::encode_tx(&c.funding))
        .await
        .unwrap();
    relay
        .send(
            &Message {
                version: 1,
                network: NET.as_str().into(),
                offer_id: offer.offer.id(),
                trade_id: trade_id.clone(),
                from: relay_msg::relay_pubkey_hex(&taker_key),
                to: terms.seller_relay_pubkey.clone(),
                sent_at: now(),
                body: Body::BtcLocked(LockAnnouncement {
                    terms_id: terms.id(),
                    txid: c.btc_lock.txid.clone(),
                    vout: c.btc_lock.vout,
                }),
            }
            .sign(&taker_key)
            .unwrap(),
            &taker_key,
        )
        .await
        .unwrap();

    // Unconfirmed: the Seller's job does not offer the Commit yet.
    tick(&conn, SELLER, &node, &srcs, &relay, &watch).await;
    let st = only_trade(&conn, SELLER);
    assert!(
        matches!(
            st.status,
            Some(Status::AwaitingBtcLock {
                confirmations: 0,
                ..
            })
        ),
        "{:?}",
        st.status
    );
    fake.mine(1);
    tick(&conn, SELLER, &node, &srcs, &relay, &watch).await;
    let st = only_trade(&conn, SELLER);
    assert!(
        matches!(st.status, Some(Status::AwaitingSellerCommit { .. })),
        "{:?}",
        st.status
    );

    // 5. The Seller's Commit: an HNS funding of the lock, then the rest.
    let funder = run_key("seller hns");
    node.generate_to_address(1, &hns_addr_of(&funder))
        .await
        .unwrap();
    node.generate_to_address(3, &miner()).await.unwrap();
    let coin = node
        .get_coins_by_address(&hns_addr_of(&funder))
        .await
        .unwrap()
        .remove(0);
    let hp = terms.hns_params().unwrap();
    let mut funding = Transaction::new();
    let mut h = [0u8; 32];
    hex::decode_to_slice(&coin.txid, &mut h).unwrap();
    funding.inputs.push(Input::new(Outpoint {
        hash: h,
        index: coin.vout,
    }));
    funding
        .outputs
        .push(hns_htlc::lock_output(&hp, terms.hns_amount));
    funding.outputs.push(Output {
        value: coin.value as u64 - terms.hns_amount - 100_000,
        address: crate::noncustodial::tx::output_address_from_string(NET, &hns_addr_of(&funder))
            .unwrap(),
        covenant: Covenant::default(),
    });
    let fpk = PublicKey::from_secret_key(&Secp256k1::new(), &funder).serialize();
    funding
        .sign_p2wpkh_input(
            0,
            &funder,
            &address::pubkey_to_hash160(&fpk),
            coin.value as u64,
            sighash::ALL,
        )
        .unwrap();
    let st = only_trade(&conn, SELLER);
    let sacct = store::account(&conn, SELLER).unwrap().unwrap();
    let seller_btc_to = btc_account::address_at(&sacct.btc_xpub, NET, 0, 0).unwrap();
    let sc = commit::seller_commit(&commit::SellerCommitInput {
        master: &sm,
        account: 0,
        trade_index: st.trade_index,
        terms: &terms,
        policy: &p,
        hns_funding: &funding,
        btc_lock: st
            .holdings
            .as_ref()
            .unwrap()
            .btc_lock
            .as_ref()
            .expect("the announcement was kept"),
        hns_rate: 1,
        hns_refund_to: &fx::hns_address(&sm, 901),
        btc_rate: 1,
        btc_claim_to: &seller_btc_to,
    })
    .unwrap();
    let mut st = st;
    st.holdings = Some(sc.holdings.clone());
    st.hns_lock = Some((sc.hns_lock.txid.clone(), sc.hns_lock.vout));
    assert!(store::commit_trade(&conn, &mut st, now()).unwrap());
    let hns_io = runner::HnsIo {
        node: &node,
        settings: &std::collections::HashMap::new(),
        network: NET,
    };
    assert!(
        runner::send_hns(&hns_io, &funding.to_hex()).await.unwrap(),
        "hsd took the HNS lock"
    );
    node.generate_to_address(1, &miner()).await.unwrap();

    // 6. From here on only the jobs act.
    tick(&conn, BUYER, &node, &srcs, &relay, &watch).await;
    let bt = only_trade(&conn, BUYER);
    assert_eq!(
        bt.status,
        Some(Status::Revealing),
        "the Buyer's job revealed"
    );
    assert!(bt.holdings.as_ref().unwrap().sent.hns_claim_at.is_some());

    tick(&conn, SELLER, &node, &srcs, &relay, &watch).await;
    let st = only_trade(&conn, SELLER);
    assert!(
        matches!(st.status, Some(Status::Claiming { .. })),
        "{:?}",
        st.status
    );
    let lock_op = bitcoin::OutPoint {
        txid: btc_htlc::parse_txid(&c.btc_lock.txid).unwrap(),
        vout: c.btc_lock.vout,
    };
    let claim = fake
        .spender_of(&lock_op)
        .expect("the Seller's job claimed the BTC lock");
    assert_eq!(claim.output[0].script_pubkey, seller_btc_to.script_pubkey());

    node.generate_to_address(1, &miner()).await.unwrap();
    fake.mine(1);
    tick(&conn, BUYER, &node, &srcs, &relay, &watch).await;
    tick(&conn, SELLER, &node, &srcs, &relay, &watch).await;
    assert_eq!(only_trade(&conn, BUYER).status, Some(Status::Completed));
    assert_eq!(only_trade(&conn, SELLER).status, Some(Status::Completed));

    // Both paid: the Buyer's HNS on the node, the Seller's BTC on the chain.
    let got = node.get_coins_by_address(&buyer_hns_to).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].value as u64, c.hns_claim.to_value);
    assert!(claim.output[0].value.to_sat() >= terms.btc_amount - sc.btc_claims[0].fee);

    // Both sides were handed to the watchtower, neither with the secret.
    let jobs = watch.jobs.lock().unwrap();
    assert_eq!(jobs.len(), 2);
    let secret = hex::encode(keys::trade_keys(&bm, NET, 0, index).unwrap().secret(&nonce));
    for j in jobs.iter() {
        assert!(!serde_json::to_string(j).unwrap().contains(&secret));
    }
}
