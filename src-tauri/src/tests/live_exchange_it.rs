//! The HNS side of the exchange on a real regtest hsd (spec R2, R3, R8,
//! R11). **Skipped unless `HNS_IT_NODE_URL` is set**, like `live_node_it`:
//!
//! ```sh
//! bash scripts/regtest.sh start
//! HNS_IT_NODE_URL=http://127.0.0.1:14037 HNS_IT_NODE_API_KEY=test \
//!   cargo test --manifest-path src-tauri/Cargo.toml live_exchange -- --nocapture --test-threads=1
//! ```
//!
//! What only a node can show: that hsd's mempool and miner accept the
//! Buyer's `0xc1` claim signed **before** the lock existed, refuse it for a
//! lock of another value, refuse a refund before its time and accept it
//! after; and that the node observer finds the lock, its confirmations and
//! the revealed secret.

use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use crate::exchange_io::hns_observe;
use crate::noncustodial::address;
use crate::noncustodial::exchange::hns_htlc;
use crate::noncustodial::exchange::script::HtlcParams;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{ChainSource, NodeRpcClient};
use crate::noncustodial::tx::{sighash, Covenant, Input, Outpoint, Output, Transaction};
use crate::noncustodial::tx_evidence::{taken_by_node_with_client, Taken};

const NET: Network = Network::Regtest;

fn it_client() -> Option<NodeRpcClient> {
    let url = std::env::var("HNS_IT_NODE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let key = std::env::var("HNS_IT_NODE_API_KEY").unwrap_or_default();
    Some(NodeRpcClient::new(&url, &key, ChainSource::LocalNode))
}

/// A key unique to this run and label (no key material in the repo).
fn key(label: &str) -> SecretKey {
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    SecretKey::from_byte_array(Sha256::digest(format!("{label} {run}").as_bytes()).into()).unwrap()
}

fn pubkey(k: &SecretKey) -> [u8; 33] {
    PublicKey::from_secret_key(&Secp256k1::new(), k).serialize()
}

fn addr(k: &SecretKey) -> String {
    address::address_from_pubkey(NET, &pubkey(k)).unwrap()
}

async fn mine(node: &NodeRpcClient, n: u32, to: &str) {
    node.generate_to_address(n, to).await.unwrap();
}

/// A spendable coin of `funder`'s: mine to it, then past coinbase maturity.
async fn fund(node: &NodeRpcClient, funder: &SecretKey) -> crate::noncustodial::rpc::NodeCoin {
    let a = addr(funder);
    mine(node, 1, &a).await;
    mine(node, 3, &addr(&key("someone else"))).await;
    let coins = node.get_coins_by_address(&a).await.unwrap();
    coins.into_iter().next().expect("a mined coin")
}

/// Pay `value` from `coin` to the lock of `p`, change back to the funder.
fn lock_tx(
    funder: &SecretKey,
    coin: &crate::noncustodial::rpc::NodeCoin,
    p: &HtlcParams,
    value: u64,
) -> Transaction {
    let mut tx = Transaction::new();
    let mut h = [0u8; 32];
    hex::decode_to_slice(&coin.txid, &mut h).unwrap();
    tx.inputs.push(Input::new(Outpoint {
        hash: h,
        index: coin.vout,
    }));
    tx.outputs.push(hns_htlc::lock_output(p, value));
    let fee = 100_000;
    tx.outputs.push(Output {
        value: coin.value as u64 - value - fee,
        address: crate::noncustodial::tx::output_address_from_string(NET, &addr(funder)).unwrap(),
        covenant: Covenant::default(),
    });
    let hash160 = address::pubkey_to_hash160(&pubkey(funder));
    tx.sign_p2wpkh_input(0, funder, &hash160, coin.value as u64, sighash::ALL)
        .unwrap();
    tx
}

async fn tip_mtp(node: &NodeRpcClient) -> u64 {
    node.get_blockchain_info()
        .await
        .unwrap()
        .mediantime
        .unwrap()
}

#[tokio::test]
async fn live_exchange_hns_claim_signed_before_the_lock_is_mined_and_reveals_the_secret() {
    let Some(node) = it_client() else {
        eprintln!("skip: HNS_IT_NODE_URL not set");
        return;
    };
    let secret: [u8; 32] = Sha256::digest(key("secret").secret_bytes()).into();
    let claim_key = key("buyer claim");
    let refund_key = key("seller refund");
    let mtp = tip_mtp(&node).await;
    let p = HtlcParams {
        hash: Sha256::digest(secret).into(),
        claim_pubkey: pubkey(&claim_key),
        refund_pubkey: pubkey(&refund_key),
        refund_lock_time: hns_htlc::encode_time_lock(mtp + 86_400).unwrap(),
    };
    let value = 1_000_000_000; // 1000 HNS
                               // 1. The Buyer signs its claim now: no lock exists yet.
    let buyer_payout = addr(&key("buyer payout"));
    let claim = hns_htlc::sign_claim(NET, &p, value, &claim_key, &buyer_payout, 20_000).unwrap();

    // 2. The Seller locks.
    let seller = key("seller funds");
    let coin = fund(&node, &seller).await;
    let funding = lock_tx(&seller, &coin, &p, value);
    node.send_raw_transaction(&funding.to_hex()).await.unwrap();
    mine(&node, 1, &addr(&key("miner"))).await;

    // 3. The observer finds exactly that coin, confirmed, unspent.
    let terms_like = terms_for(&p, value);
    let seen = observe_params(&node, &terms_like).await;
    let lock = seen.lock.clone().expect("the lock is found");
    assert_eq!(lock.outpoint.txid, funding.txid());
    assert_eq!(lock.outpoint.vout, 0);
    assert_eq!(lock.confirmations, 1);
    assert!(lock.spend.is_none());

    // 4. The presigned claim, finished with the real outpoint and the
    //    secret, enters the mempool and is mined.
    let mut h = [0u8; 32];
    hex::decode_to_slice(funding.txid(), &mut h).unwrap();
    let tx = claim
        .finish(NET, &p, Outpoint { hash: h, index: 0 }, &secret)
        .unwrap();
    assert_eq!(
        send(&node, &tx).await,
        Taken::Yes,
        "hsd refused the 0xc1 claim"
    );
    // In the mempool the observer already sees the secret.
    let seen = observe_params(&node, &terms_like).await;
    let s = seen
        .lock
        .unwrap()
        .spend
        .expect("the claim is seen in the mempool");
    assert_eq!(s.confirmations, 0);
    assert_eq!(s.secret, Some(secret));
    mine(&node, 1, &addr(&key("miner"))).await;
    let seen = observe_params(&node, &terms_like).await;
    let s = seen.lock.unwrap().spend.unwrap();
    assert_eq!(s.confirmations, 1);
    assert_eq!(s.txid, tx.txid());
    let paid = node.get_coins_by_address(&buyer_payout).await.unwrap();
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0].value as u64, value - 20_000);
}

#[tokio::test]
async fn live_exchange_hns_claim_does_not_fit_a_lock_of_another_value() {
    let Some(node) = it_client() else {
        eprintln!("skip: HNS_IT_NODE_URL not set");
        return;
    };
    let secret: [u8; 32] = Sha256::digest(key("secret 2").secret_bytes()).into();
    let claim_key = key("buyer claim 2");
    let mtp = tip_mtp(&node).await;
    let p = HtlcParams {
        hash: Sha256::digest(secret).into(),
        claim_pubkey: pubkey(&claim_key),
        refund_pubkey: pubkey(&key("seller refund 2")),
        refund_lock_time: hns_htlc::encode_time_lock(mtp + 86_400).unwrap(),
    };
    let value = 500_000_000;
    let claim =
        hns_htlc::sign_claim(NET, &p, value, &claim_key, &addr(&key("payout 2")), 20_000).unwrap();
    let seller = key("seller funds 2");
    let coin = fund(&node, &seller).await;
    // The Seller locks one doo more than the terms.
    let funding = lock_tx(&seller, &coin, &p, value + 1);
    node.send_raw_transaction(&funding.to_hex()).await.unwrap();
    mine(&node, 1, &addr(&key("miner"))).await;
    // The observer does not call it the lock.
    let seen = observe_params(&node, &terms_for(&p, value)).await;
    assert!(seen.lock.is_none());
    assert!(seen.mismatched_payment);
    // And hsd refuses the claim on it.
    let mut h = [0u8; 32];
    hex::decode_to_slice(funding.txid(), &mut h).unwrap();
    let tx = claim
        .finish(NET, &p, Outpoint { hash: h, index: 0 }, &secret)
        .unwrap();
    assert_eq!(
        send(&node, &tx).await,
        Taken::No,
        "hsd took a claim of another value"
    );
}

#[tokio::test]
async fn live_exchange_hns_refund_is_refused_before_its_time_and_mined_after() {
    let Some(node) = it_client() else {
        eprintln!("skip: HNS_IT_NODE_URL not set");
        return;
    };
    let refund_key = key("seller refund 3");
    let mtp = tip_mtp(&node).await;
    let value = 300_000_000;
    let seller = key("seller funds 3");
    // A lock whose refund opens in a day: refused now.
    let future = HtlcParams {
        hash: [3u8; 32],
        claim_pubkey: pubkey(&key("buyer claim 3")),
        refund_pubkey: pubkey(&refund_key),
        refund_lock_time: hns_htlc::encode_time_lock(mtp + 86_400).unwrap(),
    };
    let coin = fund(&node, &seller).await;
    let funding = lock_tx(&seller, &coin, &future, value);
    node.send_raw_transaction(&funding.to_hex()).await.unwrap();
    mine(&node, 1, &addr(&key("miner"))).await;
    let mut h = [0u8; 32];
    hex::decode_to_slice(funding.txid(), &mut h).unwrap();
    let refund = hns_htlc::sign_refund(
        NET,
        &future,
        Outpoint { hash: h, index: 0 },
        value,
        &refund_key,
        &addr(&seller),
        20_000,
    )
    .unwrap();
    assert_eq!(
        send(&node, &refund).await,
        Taken::No,
        "hsd took an early refund"
    );

    // A lock whose refund time is already behind the median time: mined.
    let past = HtlcParams {
        refund_lock_time: hns_htlc::encode_time_lock(mtp - 3600).unwrap(),
        ..future.clone()
    };
    assert!(hns_htlc::time_lock_open(past.refund_lock_time, mtp));
    let coin = fund(&node, &seller).await;
    let funding = lock_tx(&seller, &coin, &past, value);
    node.send_raw_transaction(&funding.to_hex()).await.unwrap();
    mine(&node, 1, &addr(&key("miner"))).await;
    hex::decode_to_slice(funding.txid(), &mut h).unwrap();
    let refund = hns_htlc::sign_refund(
        NET,
        &past,
        Outpoint { hash: h, index: 0 },
        value,
        &refund_key,
        &addr(&seller),
        20_000,
    )
    .unwrap();
    assert_eq!(send(&node, &refund).await, Taken::Yes);
    mine(&node, 1, &addr(&key("miner"))).await;
    let seen = observe_params(&node, &terms_for(&past, value)).await;
    let s = seen.lock.unwrap().spend.expect("the refund is seen");
    assert_eq!(s.secret, None, "a refund carries no secret");
    assert_eq!(s.confirmations, 1);
}

/// A lock made by a miner. What only a node shows: hsd lists the coinbase
/// transaction at the lock's address like any other, its output has exactly
/// the lock's script and value, and hsd refuses the claim of it (a premature
/// spend of a coinbase; on mainnet for 100 blocks, and `sendrawtransaction`
/// still sends such a claim to the node's peers). The observer must not call
/// it the lock.
#[tokio::test]
async fn live_exchange_a_coinbase_output_is_not_the_lock() {
    let Some(node) = it_client() else {
        eprintln!("skip: HNS_IT_NODE_URL not set");
        return;
    };
    let secret: [u8; 32] = Sha256::digest(key("secret 4").secret_bytes()).into();
    let claim_key = key("buyer claim 4");
    let mtp = tip_mtp(&node).await;
    let p = HtlcParams {
        hash: Sha256::digest(secret).into(),
        claim_pubkey: pubkey(&claim_key),
        refund_pubkey: pubkey(&key("seller refund 4")),
        refund_lock_time: hns_htlc::encode_time_lock(mtp + 86_400).unwrap(),
    };
    // The Seller mines the lock: one block whose reward pays the lock's address.
    let lock_addr = hns_htlc::lock_address(NET, &p).unwrap();
    mine(&node, 1, &lock_addr).await;
    let coins = node.get_coins_by_address(&lock_addr).await.unwrap();
    assert_eq!(coins.len(), 1, "the block reward is the only coin there");
    let value = coins[0].value as u64;

    // One confirmation is all regtest terms ask for, yet it is no lock.
    let seen = observe_params(&node, &terms_for(&p, value)).await;
    assert!(
        seen.lock.is_none(),
        "a coinbase output was taken as the lock"
    );
    assert!(seen.mismatched_payment);
    assert!(seen.revealed.is_none());

    // And hsd refuses the claim of it: the coin is not mature.
    let claim =
        hns_htlc::sign_claim(NET, &p, value, &claim_key, &addr(&key("payout 4")), 20_000).unwrap();
    let mut h = [0u8; 32];
    hex::decode_to_slice(&coins[0].txid, &mut h).unwrap();
    let tx = claim
        .finish(
            NET,
            &p,
            Outpoint {
                hash: h,
                index: coins[0].vout,
            },
            &secret,
        )
        .unwrap();
    assert_eq!(
        send(&node, &tx).await,
        Taken::No,
        "hsd took a claim of an immature coinbase output"
    );
}

/// Terms around bare lock parameters, for the observer (only the network,
/// the HNS amount and the HNS lock's parameters are read).
fn terms_for(p: &HtlcParams, value: u64) -> crate::noncustodial::exchange::terms::Terms {
    let t = super::exchange_fixture::terms_at(super::exchange_fixture::START);
    crate::noncustodial::exchange::terms::Terms {
        hns_amount: value,
        hash: hex::encode(p.hash),
        buyer_hns_claim_pubkey: hex::encode(p.claim_pubkey),
        seller_hns_refund_pubkey: hex::encode(p.refund_pubkey),
        hns_refund_time: hns_htlc::time_lock_secs(p.refund_lock_time).unwrap(),
        ..t
    }
}

async fn observe_params(
    node: &NodeRpcClient,
    t: &crate::noncustodial::exchange::terms::Terms,
) -> crate::noncustodial::exchange::engine::HnsSeen {
    hns_observe::observe(node, t, None, false).await.unwrap()
}

/// hsd's `sendrawtransaction` answers the txid whatever its mempool does
/// (spec 2026-10-07): ask the node whether it took the transaction.
async fn send(node: &NodeRpcClient, tx: &Transaction) -> Taken {
    let hex = tx.to_hex();
    node.send_raw_transaction(&hex).await.unwrap();
    taken_by_node_with_client(node, &hex, &tx.txid()).await
}
