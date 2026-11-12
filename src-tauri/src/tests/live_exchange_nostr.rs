//! The default Nostr relays, live (spec 2026-11-12 §5 "write limits").
//! **Skipped unless `HNS_IT_NOSTR=1`**: it publishes real, short-lived test
//! events to public relays.
//!
//! ```sh
//! HNS_IT_NOSTR=1 cargo test --manifest-path src-tauri/Cargo.toml live_exchange_nostr -- --nocapture --test-threads=1
//! ```
//!
//! Each relay is asked on its own, from a throwaway key, with a random
//! network tag, contents that say nothing of who sends them, and a 10-minute
//! NIP-40 expiration: an offer-kind event, its replacement, its deletion, an
//! encrypted message-kind event addressed by `p`, and a burst of 20 writes.
//! It passes when enough relays take and serve both kinds for the quorum.

use std::time::Duration;

use serde_json::json;

use crate::exchange_io::nostr::client::NostrClient;
use crate::exchange_io::nostr::{event, nip44};
use crate::noncustodial::exchange::relay_msg;

const RELAYS: [&str; 6] = [
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://nostr.oxtr.dev",
    "wss://offchain.pub",
    "wss://nostr.mom",
];
const OFFER_KIND: u32 = 37491;
const MESSAGE_KIND: u32 = 7491;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[derive(Debug, Default)]
struct Report {
    offer_taken: bool,
    offer_served: bool,
    replaced: bool,
    deleted: bool,
    message_taken: bool,
    message_served: bool,
    burst_taken: usize,
    notes: Vec<String>,
}

async fn offers(c: &NostrClient, net: &str) -> Result<Vec<event::Event>, crate::error::AppError> {
    c.query(json!({"kinds": [OFFER_KIND], "#n": [net]})).await
}

async fn check(url: &str) -> Report {
    let mut r = Report::default();
    let c = NostrClient::new(vec![url.to_owned()]);
    let key = relay_msg::new_relay_key();
    let net = format!("probe-{}", relay_msg::random_hex16());
    let exp = (now() + 600).to_string();
    let d = relay_msg::random_hex16();
    let tags = |extra: Vec<Vec<String>>| {
        let mut t = vec![
            vec!["n".to_owned(), net.clone()],
            vec!["expiration".to_owned(), exp.clone()],
        ];
        t.extend(extra);
        t
    };
    let t0 = now();
    let first = event::sign(
        &key,
        t0,
        OFFER_KIND,
        tags(vec![vec!["d".into(), d.clone()]]),
        "v1".into(),
    );
    match c.publish(&first).await {
        Ok(_) => r.offer_taken = true,
        Err(e) => r.notes.push(format!("offer: {e}")),
    }
    match offers(&c, &net).await {
        Ok(list) => r.offer_served = list.iter().any(|e| e.id == first.id),
        Err(e) => r.notes.push(format!("read offers: {e}")),
    }
    let second = event::sign(
        &key,
        t0 + 1,
        OFFER_KIND,
        tags(vec![vec!["d".into(), d.clone()]]),
        "v2".into(),
    );
    if let Err(e) = c.publish(&second).await {
        r.notes.push(format!("replacement: {e}"));
    }
    if let Ok(list) = offers(&c, &net).await {
        r.replaced = list.len() == 1 && list[0].id == second.id;
        if !r.replaced {
            r.notes
                .push(format!("after replacement {} versions served", list.len()));
        }
    }
    let author = &second.pubkey;
    let deletion = event::sign(
        &key,
        t0 + 2,
        5,
        vec![
            vec!["a".into(), format!("{OFFER_KIND}:{author}:{d}")],
            vec!["k".into(), OFFER_KIND.to_string()],
        ],
        String::new(),
    );
    if let Err(e) = c.publish(&deletion).await {
        r.notes.push(format!("deletion: {e}"));
    }
    if let Ok(list) = offers(&c, &net).await {
        r.deleted = list.is_empty();
    }
    let to = relay_msg::new_relay_key();
    let to_x = to
        .x_only_public_key(&secp256k1::Secp256k1::new())
        .0
        .serialize();
    let ck = nip44::conversation_key(&key, &to_x).unwrap();
    let msg = event::sign(
        &key,
        t0 + 3,
        MESSAGE_KIND,
        tags(vec![vec!["p".into(), hex::encode(to_x)]]),
        nip44::encrypt("test", &ck).unwrap(),
    );
    match c.publish(&msg).await {
        Ok(_) => r.message_taken = true,
        Err(e) => r.notes.push(format!("message: {e}")),
    }
    match c
        .query(json!({"kinds": [MESSAGE_KIND], "#p": [hex::encode(to_x)], "since": t0}))
        .await
    {
        Ok(list) => r.message_served = list.iter().any(|e| e.id == msg.id),
        Err(e) => r.notes.push(format!("read messages: {e}")),
    }
    // A heartbeat-like burst: 20 offers of one key, one after another.
    let mut refusals = Vec::new();
    for i in 0..20u64 {
        let e = event::sign(
            &key,
            t0 + 4 + i,
            OFFER_KIND,
            tags(vec![vec!["d".into(), format!("{d}-{i}")]]),
            "burst".into(),
        );
        match c.publish(&e).await {
            Ok(_) => r.burst_taken += 1,
            Err(e) => refusals.push(e.to_string()),
        }
    }
    if let Some(first_refusal) = refusals.first() {
        r.notes.push(format!("burst refusal: {first_refusal}"));
    }
    r
}

#[tokio::test]
async fn live_exchange_nostr_default_relays_take_and_serve_our_kinds() {
    if std::env::var("HNS_IT_NOSTR").ok().as_deref() != Some("1") {
        eprintln!("skip: HNS_IT_NOSTR not set");
        return;
    }
    let reports = futures_util::future::join_all(RELAYS.iter().map(|u| check(u))).await;
    let mut usable = 0;
    for (url, r) in RELAYS.iter().zip(&reports) {
        eprintln!(
            "{url}: offer taken {} served {} | replaced {} | deleted {} | message taken {} served {} | burst {}/20 | {}",
            r.offer_taken, r.offer_served, r.replaced, r.deleted, r.message_taken, r.message_served, r.burst_taken,
            r.notes.join(" / ")
        );
        if r.offer_taken && r.offer_served && r.message_taken && r.message_served {
            usable += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        usable >= 2,
        "only {usable} relays take and serve both kinds"
    );
}
