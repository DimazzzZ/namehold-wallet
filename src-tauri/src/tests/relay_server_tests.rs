//! The relay and watchtower server (spec §8): every rule of the API against
//! the handlers with an in-memory store, and the watchtower round against
//! the fixture's real signed trade with a mock node and a fake Bitcoin
//! source.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bitcoin::block::Header;
use bitcoin::{Address, BlockHash, OutPoint, Transaction as BtcTx, TxMerkleNode, Txid};
use secp256k1::SecretKey;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::exchange_io::btc_source::{BtcSource, Outspend, SourceUtxo};
use crate::noncustodial::exchange::engine::{Chain, SentLog, Status};
use crate::noncustodial::exchange::relay_msg::{
    relay_pubkey_hex, sign_withdraw, Body, Envelope, Message, Offer, Side, SignedOffer,
    MAX_MESSAGE_BYTES, MSG_VERSION,
};
use crate::noncustodial::exchange::watch::WatchJob;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::BlockchainInfo;
use crate::noncustodial::tx::Outpoint;
use crate::relay_server::config::Config;
use crate::relay_server::handlers::{self, ApiError, Reply};
use crate::relay_server::store::{self, Store};
use crate::relay_server::watchtower::{self, NetworkIo, FINISHED_GRACE_SECS};
use crate::tests::exchange_fixture::{self as fx, Trade, NET};
use crate::tests::mock_node_rpc::{node_status_with_indexes, MockNodeRpc};

const NOW: u64 = 1_800_000_000;

/// regtest with a watchtower, testnet as relay only; main is not served.
fn config() -> Config {
    Config::parse(
        r#"{
            "listen": "127.0.0.1:0",
            "dbPath": "relay.db",
            "networks": {
                "regtest": {
                    "node": { "url": "http://127.0.0.1:14037", "apiKey": "test" },
                    "btcSources": ["bitcoind+http://user:pass@127.0.0.1:18443"]
                },
                "testnet": {}
            }
        }"#,
    )
    .unwrap()
}

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

/// A relay key from a label (never a real key).
fn key(label: &str) -> SecretKey {
    SecretKey::from_byte_array(Sha256::digest(label.as_bytes()).into()).unwrap()
}

fn offer_on(k: &SecretKey, network: &str, nonce: u32, now: u64, expires_in: u64) -> SignedOffer {
    Offer {
        version: MSG_VERSION,
        network: network.into(),
        side: Side::Sell,
        price_msat_per_hns: 20_000,
        min_hns: 1_000_000,
        max_hns: 5_000_000_000,
        maker_relay_pubkey: relay_pubkey_hex(k),
        created_at: now,
        expires_at: now + expires_in,
        nonce: format!("{nonce:032x}"),
    }
    .sign(k)
    .unwrap()
}

fn offer(k: &SecretKey, nonce: u32, now: u64) -> SignedOffer {
    offer_on(k, "regtest", nonce, now, 3600)
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

fn message_with(from: &SecretKey, to: &SecretKey, n: u64, version: u8) -> Envelope {
    Message {
        version,
        network: "regtest".into(),
        offer_id: "ab".repeat(32),
        trade_id: "cd".repeat(16),
        from: relay_pubkey_hex(from),
        to: relay_pubkey_hex(to),
        sent_at: NOW + n,
        body: Body::Decline {
            reason: format!("message {n}"),
        },
    }
    .sign(from)
    .unwrap()
}

fn message(from: &SecretKey, to: &SecretKey, n: u64) -> Envelope {
    message_with(from, to, n, MSG_VERSION)
}

fn status_of(r: &handlers::ApiResult) -> u16 {
    match r {
        Ok(Reply { status, .. }) => *status,
        Err(ApiError { status, .. }) => *status,
    }
}

fn listed(s: &Store, network: &str, now: u64) -> Vec<String> {
    let r = handlers::list_offers(s, &config(), &format!("network={network}"), now).unwrap();
    assert_eq!(r.body["now"], now);
    r.body["offers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let o: SignedOffer = serde_json::from_value(o.clone()).unwrap();
            o.offer.id()
        })
        .collect()
}

fn inbox(s: &Store, to: &SecretKey, after: Option<&str>, now: u64) -> (Vec<Envelope>, String) {
    let mut q = format!("to={}", relay_pubkey_hex(to));
    if let Some(a) = after {
        q.push_str(&format!("&after={a}"));
    }
    let r = handlers::list_messages(s, &q, now).unwrap();
    let msgs = r.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| serde_json::from_value(m.clone()).unwrap())
        .collect();
    (msgs, r.body["cursor"].as_str().unwrap().to_owned())
}

// ------------------------------------------------------------------ config

#[test]
fn config_refuses_a_node_over_plain_http_to_another_machine() {
    let e = Config::parse(
        r#"{"listen":"127.0.0.1:0","dbPath":"r.db","networks":{"regtest":{
            "node":{"url":"http://203.0.113.5:14037","apiKey":"test"},
            "btcSources":["bitcoind+http://user:pass@127.0.0.1:18443"]}}}"#,
    )
    .unwrap_err();
    assert!(
        e.to_string().contains("https, or http to this machine"),
        "{e}"
    );
}

#[test]
fn config_refuses_a_watchtower_without_a_bitcoin_source() {
    // regtest has no default source.
    let e = Config::parse(
        r#"{"listen":"127.0.0.1:0","dbPath":"r.db","networks":{"regtest":{
            "node":{"url":"http://127.0.0.1:14037","apiKey":"test"}}}}"#,
    )
    .unwrap_err();
    assert!(e.to_string().contains("at least one Bitcoin source"), "{e}");
}

#[test]
fn config_refuses_a_network_spelled_otherwise_than_canonically() {
    for name in ["mainnet", "bitcoin", "Main"] {
        let e = Config::parse(&format!(
            r#"{{"listen":"127.0.0.1:0","dbPath":"r.db","networks":{{"{name}":{{}}}}}}"#
        ));
        assert!(e.is_err(), "{name}");
    }
}

#[test]
fn config_without_a_node_serves_the_relay_but_no_watchtower() {
    let c = config();
    assert!(c.watches(Network::Regtest));
    assert_eq!(c.serves("testnet"), Some(Network::Testnet));
    assert!(!c.watches(Network::Testnet));
    assert_eq!(c.serves("main"), None);
    assert_eq!(c.networks[&Network::Testnet].btc_sources.len(), 2);
}

#[test]
fn config_debug_output_hides_the_api_key_and_the_source_password() {
    let shown = format!("{:?}", config());
    assert!(!shown.contains("\"test\""), "{shown}");
    assert!(!shown.contains("pass@"), "{shown}");
}

// ------------------------------------------------------------------ health

#[test]
fn health_reports_the_version_and_the_served_networks() {
    let r = handlers::health(&config());
    assert_eq!(r.status, 200);
    assert_eq!(r.body["ok"], true);
    assert_eq!(r.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        r.body["networks"],
        serde_json::json!(["regtest", "testnet"])
    );
}

// ------------------------------------------------------------------ offers

#[test]
fn a_signed_offer_is_stored_and_listed() {
    let s = store();
    let o = offer(&key("maker"), 1, NOW);
    let r = handlers::post_offer(&s, &config(), &json(&o), NOW).unwrap();
    assert_eq!(r.status, 201);
    assert_eq!(r.body["id"], o.offer.id());
    assert_eq!(listed(&s, "regtest", NOW + 10), vec![o.offer.id()]);
}

#[test]
fn an_offer_with_a_bad_signature_is_refused() {
    let s = store();
    let mut o = offer(&key("maker"), 1, NOW);
    o.offer.price_msat_per_hns += 1;
    let r = handlers::post_offer(&s, &config(), &json(&o), NOW);
    assert_eq!(status_of(&r), 400);
    assert!(r.unwrap_err().message.contains("signature"));
    assert!(listed(&s, "regtest", NOW).is_empty());
}

#[test]
fn an_offer_signed_by_another_key_than_its_maker_key_is_refused() {
    let s = store();
    let mut o = offer(&key("maker"), 1, NOW);
    o.signature = offer(&key("someone else"), 1, NOW).signature;
    assert_eq!(
        status_of(&handlers::post_offer(&s, &config(), &json(&o), NOW)),
        400
    );
}

#[test]
fn a_body_over_16_kib_is_refused_with_413_on_every_post() {
    let s = store();
    let big = format!("{{\"pad\":\"{}\"}}", "x".repeat(MAX_MESSAGE_BYTES));
    let c = config();
    assert_eq!(status_of(&handlers::post_offer(&s, &c, &big, NOW)), 413);
    assert_eq!(status_of(&handlers::post_message(&s, &c, &big, NOW)), 413);
    assert_eq!(status_of(&handlers::post_watch(&s, &c, &big, NOW)), 413);
    assert_eq!(
        status_of(&handlers::withdraw_offer(&s, &"ab".repeat(32), &big, NOW)),
        413
    );
    let r = handlers::dispatch(&s, &c, "POST", "/v1/offers", "", &big, NOW);
    assert_eq!(r.status, 413);
    assert!(r.body["error"].as_str().unwrap().contains("larger than"));
}

#[test]
fn a_body_of_exactly_16_kib_is_read() {
    let s = store();
    // Valid JSON padded with whitespace to the limit: it is parsed, and
    // refused only for what it says.
    let o = json(&offer(&key("maker"), 1, NOW));
    let padded = format!("{o}{}", " ".repeat(MAX_MESSAGE_BYTES - o.len()));
    assert_eq!(padded.len(), MAX_MESSAGE_BYTES);
    let r = handlers::post_offer(&s, &config(), &padded, NOW).unwrap();
    assert_eq!(r.status, 201);
}

#[test]
fn an_offer_that_fails_its_shape_check_is_refused() {
    let s = store();
    let k = key("maker");
    // Already expired.
    let expired = offer_on(&k, "regtest", 1, NOW - 7200, 3600);
    let r = handlers::post_offer(&s, &config(), &json(&expired), NOW);
    assert_eq!(status_of(&r), 400);
    assert!(r.unwrap_err().message.contains("expired"));
    // Empty amounts, signed.
    let mut o = offer(&k, 2, NOW).offer;
    o.min_hns = 0;
    let r = handlers::post_offer(&s, &config(), &json(&o.sign(&k).unwrap()), NOW);
    assert_eq!(status_of(&r), 400);
}

#[test]
fn an_offer_of_a_network_the_relay_does_not_serve_is_refused() {
    let s = store();
    let o = offer_on(&key("maker"), "main", 1, NOW, 3600);
    let r = handlers::post_offer(&s, &config(), &json(&o), NOW);
    assert_eq!(status_of(&r), 400);
    assert!(r.unwrap_err().message.contains("does not serve"));
    // An alias spelling is not the network either.
    let o = offer_on(&key("maker"), "mainnet", 2, NOW, 3600);
    assert_eq!(
        status_of(&handlers::post_offer(&s, &config(), &json(&o), NOW)),
        400
    );
}

#[test]
fn a_maker_key_may_have_at_most_20_live_offers() {
    let s = store();
    let c = config();
    let k = key("busy maker");
    for i in 0..20 {
        let r = handlers::post_offer(&s, &c, &json(&offer(&k, i, NOW)), NOW).unwrap();
        assert_eq!(r.status, 201, "offer {i}");
    }
    let r = handlers::post_offer(&s, &c, &json(&offer(&k, 20, NOW)), NOW);
    assert_eq!(status_of(&r), 429);
    // Another key is not limited by this one.
    let other = offer(&key("other maker"), 0, NOW);
    assert_eq!(
        status_of(&handlers::post_offer(&s, &c, &json(&other), NOW)),
        201
    );
    // Renewing one of the twenty is not a 21st.
    assert_eq!(
        status_of(&handlers::post_offer(
            &s,
            &c,
            &json(&offer(&k, 3, NOW)),
            NOW + 60
        )),
        200
    );
}

#[test]
fn a_silent_offer_that_comes_back_counts_against_the_limit() {
    let s = store();
    let c = config();
    let k = key("maker");
    let first = offer(&k, 0, NOW);
    handlers::post_offer(&s, &c, &json(&first), NOW).unwrap();
    // It goes silent; twenty others go live.
    let later = NOW + 300;
    for i in 1..=20 {
        let o = offer_on(&k, "regtest", i, later, 3600);
        assert_eq!(
            status_of(&handlers::post_offer(&s, &c, &json(&o), later)),
            201
        );
    }
    // Its heartbeat would make it the 21st live offer.
    let r = handlers::post_offer(&s, &c, &json(&first), later);
    assert_eq!(status_of(&r), 429);
}

#[test]
fn reposting_an_offer_is_its_heartbeat() {
    let s = store();
    let c = config();
    let o = offer(&key("maker"), 1, NOW);
    handlers::post_offer(&s, &c, &json(&o), NOW).unwrap();
    let r = handlers::post_offer(&s, &c, &json(&o), NOW + 120).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body["id"], o.offer.id());
    let row = store::get_offer(&s.lock().unwrap(), &o.offer.id())
        .unwrap()
        .unwrap();
    assert_eq!((row.created_at, row.last_seen), (NOW, NOW + 120));
    // Still one offer, live 180 s after the heartbeat.
    assert_eq!(listed(&s, "regtest", NOW + 300), vec![o.offer.id()]);
}

#[test]
fn an_offer_not_renewed_for_180_seconds_leaves_the_list() {
    let s = store();
    let c = config();
    let o = offer(&key("maker"), 1, NOW);
    handlers::post_offer(&s, &c, &json(&o), NOW).unwrap();
    assert_eq!(listed(&s, "regtest", NOW + 180), vec![o.offer.id()]);
    assert!(listed(&s, "regtest", NOW + 181).is_empty());
    // A heartbeat brings it back.
    handlers::post_offer(&s, &c, &json(&o), NOW + 200).unwrap();
    assert_eq!(listed(&s, "regtest", NOW + 201), vec![o.offer.id()]);
}

#[test]
fn an_offer_is_not_listed_from_its_expiry() {
    let s = store();
    let c = config();
    let o = offer_on(&key("maker"), "regtest", 1, NOW, 100);
    handlers::post_offer(&s, &c, &json(&o), NOW).unwrap();
    handlers::post_offer(&s, &c, &json(&o), NOW + 90).unwrap();
    assert_eq!(listed(&s, "regtest", NOW + 99), vec![o.offer.id()]);
    assert!(listed(&s, "regtest", NOW + 100).is_empty());
    // And it cannot be renewed past it.
    assert_eq!(
        status_of(&handlers::post_offer(&s, &c, &json(&o), NOW + 100)),
        400
    );
}

#[test]
fn offers_are_listed_newest_first_at_most_500_and_by_network() {
    let s = store();
    let c = config();
    let mut ids = Vec::new();
    for i in 0..501u32 {
        let k = key(&format!("maker {}", i / 20));
        let o = offer(&k, i, NOW);
        // Ten a second; arrival breaks a tie.
        handlers::post_offer(&s, &c, &json(&o), NOW + u64::from(i) / 10).unwrap();
        ids.push(o.offer.id());
    }
    let t = offer_on(&key("testnet maker"), "testnet", 0, NOW, 3600);
    handlers::post_offer(&s, &c, &json(&t), NOW).unwrap();

    let got = listed(&s, "regtest", NOW + 60);
    assert_eq!(got.len(), 500);
    let newest_first: Vec<String> = ids.iter().rev().take(500).cloned().collect();
    assert_eq!(got, newest_first);
    assert_eq!(listed(&s, "testnet", NOW + 60), vec![t.offer.id()]);
}

#[test]
fn listing_offers_needs_a_served_network() {
    let s = store();
    let c = config();
    assert_eq!(
        status_of(&handlers::list_offers(&s, &c, "network=main", NOW)),
        400
    );
    assert_eq!(status_of(&handlers::list_offers(&s, &c, "", NOW)), 400);
}

#[test]
fn a_withdrawal_signed_by_the_maker_takes_the_offer_down_for_good() {
    let s = store();
    let c = config();
    let k = key("maker");
    let o = offer(&k, 1, NOW);
    let id = o.offer.id();
    handlers::post_offer(&s, &c, &json(&o), NOW).unwrap();
    let body = serde_json::json!({ "signature": sign_withdraw(&k, &id) }).to_string();
    let r = handlers::withdraw_offer(&s, &id, &body, NOW + 5).unwrap();
    assert_eq!((r.status, r.body.clone()), (200, serde_json::json!({})));
    assert!(listed(&s, "regtest", NOW + 6).is_empty());
    // Someone re-posting the signed offer does not bring it back.
    let again = handlers::post_offer(&s, &c, &json(&o), NOW + 10);
    assert_eq!(status_of(&again), 410);
    assert!(listed(&s, "regtest", NOW + 11).is_empty());
}

#[test]
fn a_withdrawal_not_signed_by_the_maker_is_refused() {
    let s = store();
    let c = config();
    let o = offer(&key("maker"), 1, NOW);
    let id = o.offer.id();
    handlers::post_offer(&s, &c, &json(&o), NOW).unwrap();
    let forged = serde_json::json!({ "signature": sign_withdraw(&key("thief"), &id) }).to_string();
    assert_eq!(
        status_of(&handlers::withdraw_offer(&s, &id, &forged, NOW)),
        403
    );
    // A maker signature over another id does not withdraw this one.
    let other = serde_json::json!({ "signature": sign_withdraw(&key("maker"), &"00".repeat(32)) })
        .to_string();
    assert_eq!(
        status_of(&handlers::withdraw_offer(&s, &id, &other, NOW)),
        403
    );
    assert_eq!(listed(&s, "regtest", NOW), vec![id.clone()]);
    // An unknown offer.
    let sig = serde_json::json!({ "signature": sign_withdraw(&key("maker"), &"11".repeat(32)) })
        .to_string();
    assert_eq!(
        status_of(&handlers::withdraw_offer(&s, &"11".repeat(32), &sig, NOW)),
        404
    );
}

// ---------------------------------------------------------------- messages

#[test]
fn a_signed_message_is_kept_for_its_addressee_under_the_hash_of_its_body() {
    let s = store();
    let (a, b) = (key("alice"), key("bob"));
    let body = json(&message(&a, &b, 1));
    let r = handlers::post_message(&s, &config(), &body, NOW).unwrap();
    assert_eq!(r.status, 201);
    assert_eq!(r.body["id"], hex::encode(Sha256::digest(body.as_bytes())));
    let (got, _) = inbox(&s, &b, None, NOW);
    assert_eq!(got, vec![message(&a, &b, 1)]);
    // The client's own check accepts what the relay returns.
    let back = serde_json::to_string(&got[0]).unwrap();
    assert!(Envelope::parse_for(&back, &relay_pubkey_hex(&b), "regtest").is_ok());
    assert!(inbox(&s, &a, None, NOW).0.is_empty());
}

#[test]
fn a_message_with_a_bad_signature_is_refused() {
    let s = store();
    let mut e = message(&key("alice"), &key("bob"), 1);
    e.message.sent_at += 1;
    let r = handlers::post_message(&s, &config(), &json(&e), NOW);
    assert_eq!(status_of(&r), 400);
    assert!(inbox(&s, &key("bob"), None, NOW).0.is_empty());
}

#[test]
fn a_message_of_an_unknown_version_is_refused() {
    let s = store();
    let e = message_with(&key("alice"), &key("bob"), 1, MSG_VERSION + 1);
    assert!(e.verify(), "signed correctly, refused for its version only");
    let r = handlers::post_message(&s, &config(), &json(&e), NOW);
    assert_eq!(status_of(&r), 400);
    assert!(r.unwrap_err().message.contains("version"));
}

#[test]
fn a_message_of_a_network_not_served_or_to_no_key_is_refused() {
    let s = store();
    let (a, b) = (key("alice"), key("bob"));
    let mut m = message(&a, &b, 1).message;
    m.network = "main".into();
    let r = handlers::post_message(&s, &config(), &json(&m.sign(&a).unwrap()), NOW);
    assert_eq!(status_of(&r), 400);
    let mut m = message(&a, &b, 2).message;
    // 33 bytes, but no compressed key starts with 0x05.
    m.to = "05".repeat(33);
    let r = handlers::post_message(&s, &config(), &json(&m.sign(&a).unwrap()), NOW);
    assert_eq!(status_of(&r), 400);
}

#[test]
fn reposting_the_same_message_stores_it_once() {
    let s = store();
    let (a, b) = (key("alice"), key("bob"));
    let body = json(&message(&a, &b, 1));
    handlers::post_message(&s, &config(), &body, NOW).unwrap();
    let r = handlers::post_message(&s, &config(), &body, NOW + 1).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(inbox(&s, &b, None, NOW + 2).0.len(), 1);
}

#[test]
fn a_sender_key_may_send_at_most_200_messages_an_hour() {
    let s = store();
    let c = config();
    let (a, b) = (key("chatty"), key("bob"));
    for n in 0..200 {
        let r = handlers::post_message(&s, &c, &json(&message(&a, &b, n)), NOW + n / 10).unwrap();
        assert_eq!(r.status, 201, "message {n}");
    }
    let r = handlers::post_message(&s, &c, &json(&message(&a, &b, 200)), NOW + 30);
    assert_eq!(status_of(&r), 429);
    // Another sender is not limited by this one.
    let r = handlers::post_message(&s, &c, &json(&message(&key("quiet"), &b, 0)), NOW + 30);
    assert_eq!(status_of(&r), 201);
    // The hour is rolling: once the first ten are an hour old, ten more.
    let later = NOW + 3600;
    for n in 200..210 {
        let r = handlers::post_message(&s, &c, &json(&message(&a, &b, n)), later).unwrap();
        assert_eq!(r.status, 201, "message {n}");
    }
    let r = handlers::post_message(&s, &c, &json(&message(&a, &b, 210)), later);
    assert_eq!(status_of(&r), 429);
}

#[test]
fn messages_are_paged_by_cursor_in_arrival_order() {
    let s = store();
    let c = config();
    let b = key("bob");
    let mut sent = Vec::new();
    // 250 messages from two senders, interleaved (the rate limit is per key).
    for n in 0..250u64 {
        let from = key(if n % 2 == 0 { "alice" } else { "carol" });
        let e = message(&from, &b, n);
        handlers::post_message(&s, &c, &json(&e), NOW + n / 50).unwrap();
        sent.push(e);
    }
    // Noise for someone else.
    handlers::post_message(
        &s,
        &c,
        &json(&message(&key("alice"), &key("dave"), 999)),
        NOW,
    )
    .unwrap();

    let (page1, cursor1) = inbox(&s, &b, None, NOW + 10);
    assert_eq!(page1, sent[..200].to_vec());
    let (page2, cursor2) = inbox(&s, &b, Some(&cursor1), NOW + 10);
    assert_eq!(page2, sent[200..].to_vec());
    let (page3, cursor3) = inbox(&s, &b, Some(&cursor2), NOW + 10);
    assert!(page3.is_empty());
    assert_eq!(cursor3, cursor2, "nothing new: the same cursor back");
    // A message stored later is found after that cursor.
    let late = message(&key("alice"), &b, 1000);
    handlers::post_message(&s, &c, &json(&late), NOW + 20).unwrap();
    assert_eq!(inbox(&s, &b, Some(&cursor3), NOW + 20).0, vec![late]);
}

#[test]
fn a_malformed_cursor_or_a_missing_addressee_is_refused() {
    let s = store();
    let to = relay_pubkey_hex(&key("bob"));
    assert_eq!(
        status_of(&handlers::list_messages(
            &s,
            &format!("to={to}&after=x"),
            NOW
        )),
        400
    );
    assert_eq!(status_of(&handlers::list_messages(&s, "", NOW)), 400);
}

#[test]
fn messages_are_kept_seven_days() {
    let s = store();
    let c = config();
    let (a, b) = (key("alice"), key("bob"));
    handlers::post_message(&s, &c, &json(&message(&a, &b, 1)), NOW).unwrap();
    handlers::post_message(&s, &c, &json(&message(&a, &b, 2)), NOW + 100).unwrap();
    let week = handlers::MESSAGE_TTL_SECS;
    assert_eq!(inbox(&s, &b, None, NOW + week - 1).0.len(), 2);
    // Older than seven days: not returned, even before a purge.
    assert_eq!(inbox(&s, &b, None, NOW + week + 1).0.len(), 1);
    let p = handlers::purge(&s, NOW + week + 1).unwrap();
    assert_eq!(p.messages, 1);
    assert_eq!(
        inbox(&s, &b, None, NOW + week + 1).0,
        vec![message(&a, &b, 2)]
    );
}

// ------------------------------------------------------------------- purge

#[test]
fn a_purge_removes_expired_and_long_silent_offers_but_keeps_a_withdrawal_until_expiry() {
    let s = store();
    let c = config();
    let k = key("maker");
    let week = handlers::SILENT_OFFER_TTL_SECS;
    let short = offer_on(&k, "regtest", 1, NOW, 100);
    let silent = offer_on(&k, "regtest", 2, NOW, 30 * 86_400);
    let withdrawn = offer_on(&k, "regtest", 3, NOW, 30 * 86_400);
    for o in [&short, &silent, &withdrawn] {
        handlers::post_offer(&s, &c, &json(o), NOW).unwrap();
    }
    let id = withdrawn.offer.id();
    let body = serde_json::json!({ "signature": sign_withdraw(&k, &id) }).to_string();
    handlers::withdraw_offer(&s, &id, &body, NOW).unwrap();

    assert_eq!(handlers::purge(&s, NOW + 100).unwrap().offers, 1);
    assert_eq!(handlers::purge(&s, NOW + week + 1).unwrap().offers, 1);
    let conn = s.lock().unwrap();
    assert!(store::get_offer(&conn, &short.offer.id())
        .unwrap()
        .is_none());
    assert!(store::get_offer(&conn, &silent.offer.id())
        .unwrap()
        .is_none());
    assert!(store::get_offer(&conn, &id).unwrap().is_some());
    drop(conn);
    // So its re-post is still refused.
    assert_eq!(
        status_of(&handlers::post_offer(
            &s,
            &c,
            &json(&withdrawn),
            NOW + week + 2
        )),
        410
    );
}

// -------------------------------------------------------------- watch jobs

fn seller_job(t: &Trade) -> WatchJob {
    WatchJob::from_holdings(&t.seller())
}

fn buyer_job(t: &Trade) -> WatchJob {
    WatchJob::from_holdings(&t.buyer())
}

#[test]
fn a_checked_watch_job_is_accepted_and_its_status_can_be_read() {
    let s = store();
    let t = fx::trade();
    let job = seller_job(&t);
    let r = handlers::post_watch(&s, &config(), &json(&job), NOW).unwrap();
    assert_eq!(r.status, 201);
    let id = format!("{}-seller", t.terms.id());
    assert_eq!(r.body["id"], id);

    let st = handlers::get_watch(&s, &id).unwrap();
    assert_eq!(st.status, 200);
    let status: Status = serde_json::from_value(st.body["status"].clone()).unwrap();
    assert_eq!(status, Status::CannotCheck { chain: Chain::Hns });
    let sent: SentLog = serde_json::from_value(st.body["sent"].clone()).unwrap();
    assert_eq!(sent, SentLog::default());
    assert_eq!(st.body["updatedAt"], NOW);

    let b = handlers::post_watch(&s, &config(), &json(&buyer_job(&t)), NOW).unwrap();
    assert_eq!(b.body["id"], format!("{}-buyer", t.terms.id()));
}

#[test]
fn a_watch_job_that_fails_its_check_is_refused() {
    let s = store();
    let t = fx::trade();
    let mut job = seller_job(&t);
    job.btc_claims.clear();
    let r = handlers::post_watch(&s, &config(), &json(&job), NOW);
    assert_eq!(status_of(&r), 400);
    assert!(r.unwrap_err().message.contains("watch job is refused"));
    // A Buyer's job that carries the Seller's transactions.
    let mut job = buyer_job(&t);
    job.btc_claims = t.seller().btc_claims;
    assert_eq!(
        status_of(&handlers::post_watch(&s, &config(), &json(&job), NOW)),
        400
    );
    // Not JSON of a job.
    assert_eq!(
        status_of(&handlers::post_watch(
            &s,
            &config(),
            "{\"role\":\"buyer\"}",
            NOW
        )),
        400
    );
}

#[test]
fn a_watch_job_of_a_network_without_a_watchtower_is_refused() {
    let s = store();
    let t = fx::trade();
    for network in ["testnet", "main"] {
        let mut job = seller_job(&t);
        job.terms.network = network.into();
        let r = handlers::post_watch(&s, &config(), &json(&job), NOW);
        assert_eq!(status_of(&r), 400, "{network}");
    }
}

#[test]
fn reposting_the_same_watch_job_is_a_no_op_and_another_one_under_its_id_is_refused() {
    let s = store();
    let t = fx::trade();
    let job = seller_job(&t);
    handlers::post_watch(&s, &config(), &json(&job), NOW).unwrap();
    let again = handlers::post_watch(&s, &config(), &json(&job), NOW + 60).unwrap();
    assert_eq!(again.status, 200);
    let id = format!("{}-seller", t.terms.id());
    let row = store::get_job(&s.lock().unwrap(), &id).unwrap().unwrap();
    assert_eq!(row.updated_at, NOW, "nothing changed");

    // The same trade and role, another job (the watchtower does not read
    // a rung's fee field).
    let mut other = job.clone();
    other.btc_claims[0].fee += 1;
    let r = handlers::post_watch(&s, &config(), &json(&other), NOW);
    assert_eq!(status_of(&r), 409);
}

#[test]
fn an_unknown_watch_job_is_not_found() {
    assert_eq!(status_of(&handlers::get_watch(&store(), "nope-buyer")), 404);
}

// ---------------------------------------------------------------- dispatch

#[test]
fn dispatch_routes_the_v1_endpoints_and_refuses_the_rest() {
    let s = store();
    let c = config();
    let d = |m: &str, p: &str, q: &str, b: &str| handlers::dispatch(&s, &c, m, p, q, b, NOW);
    assert_eq!(d("GET", "/v1/health", "", "").status, 200);
    let o = offer(&key("maker"), 1, NOW);
    let r = d("POST", "/v1/offers", "", &json(&o));
    assert_eq!(r.status, 201);
    let r = d("GET", "/v1/offers", "network=regtest", "");
    assert_eq!(r.body["offers"].as_array().unwrap().len(), 1);
    let id = o.offer.id();
    let w = serde_json::json!({ "signature": sign_withdraw(&key("maker"), &id) }).to_string();
    assert_eq!(
        d("POST", &format!("/v1/offers/{id}/withdraw"), "", &w).status,
        200
    );
    let e = message(&key("alice"), &key("bob"), 1);
    assert_eq!(d("POST", "/v1/messages", "", &json(&e)).status, 201);
    let to = relay_pubkey_hex(&key("bob"));
    let r = d("GET", "/v1/messages", &format!("to={to}"), "");
    assert_eq!(r.body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(r.body["cursor"], "1");
    assert_eq!(d("GET", "/v1/watch/x-buyer", "", "").status, 404);

    let r = d("GET", "/v2/health", "", "");
    assert_eq!(r.status, 404);
    assert!(r.body["error"].is_string());
    assert_eq!(d("DELETE", "/v1/offers", "", "").status, 405);
    assert_eq!(d("POST", "/v1/health", "", "").status, 405);
    assert_eq!(
        d("GET", &format!("/v1/offers/{id}/withdraw"), "", "").status,
        405
    );
    assert_eq!(
        d("POST", "/v1/offers/not-an-id/withdraw", "", &w).status,
        404
    );
}

// ------------------------------------------------------------- store file

#[test]
fn the_store_file_is_opened_in_wal_mode_and_keeps_its_rows() {
    let dir = std::env::temp_dir().join(format!(
        "namehold-relay-test-{}-{}",
        std::process::id(),
        relay_msg_nonce()
    ));
    let path = dir.join("relay.db");
    {
        let s = Store::open(&path).unwrap();
        let mode: String = s
            .lock()
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        handlers::post_offer(&s, &config(), &json(&offer(&key("m"), 1, NOW)), NOW).unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(listed(&s, "regtest", NOW).len(), 1);
    drop(s);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700, "a directory it creates is owner-only");
        assert_eq!(mode(&path), 0o600, "the database is owner-only");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A database put in an existing directory leaves that directory's mode
/// alone (it may be `/var/lib`).
#[cfg(unix)]
#[test]
fn the_store_leaves_an_existing_directory_as_it_is() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!(
        "namehold-relay-test-{}-{}",
        std::process::id(),
        relay_msg_nonce()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    drop(Store::open(&dir.join("relay.db")).unwrap());
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
    let _ = std::fs::remove_dir_all(&dir);
}

fn relay_msg_nonce() -> String {
    crate::noncustodial::exchange::relay_msg::random_hex16()
}

// -------------------------------------------------------------- watchtower

/// A Bitcoin source that knows the given transactions, none of them
/// confirmed and none spent, and records what it is asked to send.
struct FakeBtc {
    mtp: u64,
    txs: HashMap<Txid, BtcTx>,
    sent: Arc<Mutex<Vec<String>>>,
}

impl FakeBtc {
    fn new(mtp: u64, txs: &[BtcTx]) -> (FakeBtc, Arc<Mutex<Vec<String>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        (
            FakeBtc {
                mtp,
                txs: txs.iter().map(|t| (t.compute_txid(), t.clone())).collect(),
                sent: sent.clone(),
            },
            sent,
        )
    }
}

#[async_trait]
impl BtcSource for FakeBtc {
    fn label(&self) -> String {
        "fake".into()
    }
    async fn tip(&self) -> Result<(u64, u64), AppError> {
        Ok((200, self.mtp))
    }
    async fn raw_tx(&self, txid: &Txid) -> Result<Option<BtcTx>, AppError> {
        Ok(self.txs.get(txid).cloned())
    }
    async fn tx_block(&self, _txid: &Txid) -> Result<Option<(u64, BlockHash)>, AppError> {
        Ok(None)
    }
    async fn merkle(
        &self,
        _txid: &Txid,
        _block: &BlockHash,
    ) -> Result<(Vec<TxMerkleNode>, u32), AppError> {
        Err(AppError::Rpc("no blocks here".into()))
    }
    async fn header_at(&self, _height: u64) -> Result<Header, AppError> {
        Err(AppError::Rpc("no blocks here".into()))
    }
    async fn header(&self, _hash: &BlockHash) -> Result<Header, AppError> {
        Err(AppError::Rpc("no blocks here".into()))
    }
    async fn outspend(&self, o: &OutPoint) -> Result<Option<Outspend>, AppError> {
        Ok(self.txs.contains_key(&o.txid).then_some(Outspend::Unspent))
    }
    async fn utxos(&self, _address: &Address) -> Result<Vec<SourceUtxo>, AppError> {
        Ok(Vec::new())
    }
    async fn fee_rate(&self) -> Result<u64, AppError> {
        Ok(2)
    }
    async fn broadcast(&self, tx_hex: &str) -> Result<Txid, AppError> {
        let tx: BtcTx = bitcoin::consensus::deserialize(&hex::decode(tx_hex).unwrap()).unwrap();
        self.sent.lock().unwrap().push(tx_hex.to_owned());
        Ok(tx.compute_txid())
    }
}

/// The HNS chain as an indexed regtest node shows it: tip 100 at `mtp`,
/// these transactions (hex, height) at the lock's address.
fn hns_node(chain: &str, mtp: u64, txs: Vec<(String, i64)>) -> MockNodeRpc {
    MockNodeRpc::new()
        .with_node_status(node_status_with_indexes(true, true))
        .with_blockchain_info(BlockchainInfo {
            blocks: 100,
            chain: Some(chain.into()),
            mediantime: Some(mtp),
            ..Default::default()
        })
        .with_txs_by_address(
            txs.into_iter()
                .map(|(hex, height)| serde_json::json!({ "hex": hex, "height": height }))
                .collect(),
        )
}

/// The Buyer's claim of the HNS lock, as it appears on chain with `x`.
fn hns_claim_hex(t: &Trade) -> String {
    let hp = t.terms.hns_params().unwrap();
    let lock = t.hns_lock();
    let mut hash = [0u8; 32];
    hex::decode_to_slice(&lock.txid, &mut hash).unwrap();
    t.buyer
        .holdings
        .hns_claim
        .as_ref()
        .unwrap()
        .finish(
            NET,
            &hp,
            Outpoint {
                hash,
                index: lock.vout,
            },
            &t.secret(),
        )
        .unwrap()
        .to_hex()
}

async fn run_tick(
    s: &Store,
    node: &MockNodeRpc,
    btc: FakeBtc,
    now: u64,
) -> Vec<watchtower::JobOutcome> {
    let sources: Vec<Box<dyn BtcSource>> = vec![Box::new(btc)];
    let mut io = HashMap::new();
    io.insert(
        Network::Regtest,
        NetworkIo {
            node: Some(node as &dyn NodeRpc),
            sources: &sources,
        },
    );
    watchtower::tick(s, &io, now).await.unwrap()
}

fn watch_state(s: &Store, id: &str) -> (Status, SentLog, Value) {
    let r = handlers::get_watch(s, id).unwrap();
    (
        serde_json::from_value(r.body["status"].clone()).unwrap(),
        serde_json::from_value(r.body["sent"].clone()).unwrap(),
        r.body["updatedAt"].clone(),
    )
}

#[tokio::test]
async fn a_seller_job_claims_the_btc_once_the_hns_chain_shows_the_claim() {
    let s = store();
    let t = fx::trade();
    let id = handlers::post_watch(&s, &config(), &json(&seller_job(&t)), NOW)
        .unwrap()
        .body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let now = t.terms.start + 600;
    let mtp = now - 600;
    let lock = (t.hns_funding.to_hex(), 90);

    // The HNS lock alone: nothing to send, the Seller waits for the reveal.
    let (btc, sent) = FakeBtc::new(mtp, std::slice::from_ref(&t.buyer.funding));
    let node = hns_node("regtest", mtp, vec![lock.clone()]);
    let out = run_tick(&s, &node, btc, now).await;
    assert_eq!(out.len(), 1);
    assert!(sent.lock().unwrap().is_empty());
    assert!(matches!(
        watch_state(&s, &id).0,
        Status::AwaitingReveal { .. }
    ));

    // The Buyer's claim on the HNS chain: the watchtower sends the BTC claim
    // with the secret it read there.
    let (btc, sent) = FakeBtc::new(mtp, std::slice::from_ref(&t.buyer.funding));
    let node = hns_node("regtest", mtp, vec![lock, (hns_claim_hex(&t), 99)]);
    let out = run_tick(&s, &node, btc, now + 60).await;
    assert!(out[0].errors.is_empty(), "{:?}", out[0].errors);
    let sent = sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let claim: BtcTx = bitcoin::consensus::deserialize(&hex::decode(&sent[0]).unwrap()).unwrap();
    assert_eq!(claim.input[0].witness.nth(1), Some(&t.secret()[..]));
    assert_eq!(
        claim.compute_txid(),
        crate::noncustodial::exchange::btc_htlc::decode(&t.seller().btc_claims[0].tx_hex)
            .unwrap()
            .compute_txid(),
        "the Seller's own presigned claim, rung 0"
    );
    let (status, log, updated) = watch_state(&s, &id);
    assert_eq!(status, Status::Claiming { rung: 0 });
    assert_eq!(log.btc_claim_first_at, Some(now + 60));
    assert_eq!(updated, now + 60);
}

#[tokio::test]
async fn a_buyer_job_refunds_the_btc_once_its_refund_time_has_passed() {
    let s = store();
    let t = fx::trade();
    let id = handlers::post_watch(&s, &config(), &json(&buyer_job(&t)), NOW)
        .unwrap()
        .body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let refund_time = t.terms.btc_refund_time;

    // The Seller never locked HNS. Before the refund time: wait.
    let (btc, sent) = FakeBtc::new(refund_time - 60, std::slice::from_ref(&t.buyer.funding));
    let node = hns_node("regtest", refund_time - 60, vec![]);
    run_tick(&s, &node, btc, refund_time - 30).await;
    assert!(sent.lock().unwrap().is_empty());
    assert_eq!(
        watch_state(&s, &id).0,
        Status::AwaitingRefund {
            chain: Chain::Btc,
            opens_at: refund_time
        }
    );

    // Past it by the chain's median time: the Buyer's own refund goes out.
    let (btc, sent) = FakeBtc::new(refund_time + 1, std::slice::from_ref(&t.buyer.funding));
    let node = hns_node("regtest", refund_time + 1, vec![]);
    run_tick(&s, &node, btc, refund_time + 60).await;
    assert_eq!(
        *sent.lock().unwrap(),
        vec![t.buyer().btc_refunds[0].tx_hex.clone()]
    );
    let (status, log, _) = watch_state(&s, &id);
    assert_eq!(status, Status::Refunding { chain: Chain::Btc });
    assert_eq!(log.btc_refund_first_at, Some(refund_time + 60));
}

#[tokio::test]
async fn a_buyer_job_never_refunds_once_the_hns_lock_is_claimed() {
    let s = store();
    let t = fx::trade();
    let id = handlers::post_watch(&s, &config(), &json(&buyer_job(&t)), NOW)
        .unwrap()
        .body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let late = t.terms.btc_refund_time + 3600;
    let (btc, sent) = FakeBtc::new(late, std::slice::from_ref(&t.buyer.funding));
    let node = hns_node(
        "regtest",
        late,
        vec![(t.hns_funding.to_hex(), 90), (hns_claim_hex(&t), 95)],
    );
    run_tick(&s, &node, btc, late).await;
    assert!(
        sent.lock().unwrap().is_empty(),
        "the BTC is the Seller's once the secret is public"
    );
    let (status, log, _) = watch_state(&s, &id);
    assert_eq!(status, Status::Completed);
    assert_eq!(log.btc_refund_first_at, None);
}

#[tokio::test]
async fn a_watchtower_whose_node_is_on_another_network_sends_nothing() {
    let s = store();
    let t = fx::trade();
    let id = handlers::post_watch(&s, &config(), &json(&buyer_job(&t)), NOW)
        .unwrap()
        .body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let late = t.terms.btc_refund_time + 3600;
    // A mainnet node shows no lock at all for a regtest trade.
    let (btc, sent) = FakeBtc::new(late, std::slice::from_ref(&t.buyer.funding));
    let node = hns_node("main", late, vec![]);
    let out = run_tick(&s, &node, btc, late).await;
    assert!(sent.lock().unwrap().is_empty());
    assert!(out[0].errors.iter().any(|e| e.contains("not 'regtest'")));
    assert_eq!(
        watch_state(&s, &id).0,
        Status::CannotCheck { chain: Chain::Hns }
    );
    let row = store::get_job(&s.lock().unwrap(), &id).unwrap().unwrap();
    assert!(row.last_error.unwrap().contains("not 'regtest'"));
}

#[tokio::test]
async fn a_finished_job_stops_running_after_a_day_and_is_purged_after_30_days() {
    let s = store();
    let t = fx::trade();
    let id = handlers::post_watch(&s, &config(), &json(&buyer_job(&t)), NOW)
        .unwrap()
        .body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let done = t.terms.btc_refund_time + 3600;
    let claimed = || {
        hns_node(
            "regtest",
            done,
            vec![(t.hns_funding.to_hex(), 90), (hns_claim_hex(&t), 95)],
        )
    };
    let btc = || FakeBtc::new(done, std::slice::from_ref(&t.buyer.funding)).0;
    run_tick(&s, &claimed(), btc(), done).await;
    let row = store::get_job(&s.lock().unwrap(), &id).unwrap().unwrap();
    assert_eq!(row.finished_at, Some(done));

    // Re-checked for a day, the first finish time kept…
    assert_eq!(run_tick(&s, &claimed(), btc(), done + 3600).await.len(), 1);
    let row = store::get_job(&s.lock().unwrap(), &id).unwrap().unwrap();
    assert_eq!(row.finished_at, Some(done));
    // …then no longer run, still readable…
    assert!(run_tick(&s, &claimed(), btc(), done + FINISHED_GRACE_SECS)
        .await
        .is_empty());
    assert_eq!(watch_state(&s, &id).0, Status::Completed);
    // …and gone after 30 days.
    let p = handlers::purge(&s, done + handlers::FINISHED_JOB_TTL_SECS - 1).unwrap();
    assert_eq!(p.jobs, 0);
    let p = handlers::purge(&s, done + handlers::FINISHED_JOB_TTL_SECS + 1).unwrap();
    assert_eq!(p.jobs, 1);
    assert_eq!(status_of(&handlers::get_watch(&s, &id)), 404);
}
