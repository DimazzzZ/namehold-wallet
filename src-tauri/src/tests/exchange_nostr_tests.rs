//! The exchange's Nostr transport (spec 2026-11-12): events (R1), NIP-44
//! (R2), the client and the meeting point.
//!
//! Vectors come from outside this code: `note_from_nos_lol.json` is a kind-1
//! note read from `wss://nos.lol` on 2026-11-12, kept byte for byte, and
//! `nip44.vectors.json` is paulmillr/nip44's file, whose SHA-256
//! (`269ed0f6…5040`) is the one NIP-44 publishes.

use crate::exchange_io::nostr::event::{self, Event};
use crate::noncustodial::exchange::relay_msg;

const NOTE: &str = include_str!("fixtures/nostr/note_from_nos_lol.json");

fn note() -> Event {
    serde_json::from_str(NOTE).unwrap()
}

// ------------------------------------------------------------------- R1

#[test]
fn event_from_a_real_relay_recomputes_its_id_and_verifies() {
    let e = note();
    assert_eq!(
        hex::encode(event::id_of(
            &e.pubkey,
            e.created_at,
            e.kind,
            &e.tags,
            &e.content
        )),
        e.id
    );
    assert!(e.verify());
}

#[test]
fn event_with_any_field_changed_does_not_verify() {
    let base = note();
    let mut changed: Vec<Event> = Vec::new();
    let mut e = base.clone();
    e.content.push(' ');
    changed.push(e);
    let mut e = base.clone();
    e.created_at += 1;
    changed.push(e);
    let mut e = base.clone();
    e.kind += 1;
    changed.push(e);
    let mut e = base.clone();
    e.tags.push(vec!["t".into(), "x".into()]);
    changed.push(e);
    let mut e = base.clone();
    e.sig
        .replace_range(0..2, if &e.sig[0..2] == "00" { "01" } else { "00" });
    changed.push(e);
    let mut e = base.clone();
    // Another valid key: the id no longer names this author.
    e.pubkey = relay_msg::relay_pubkey_hex(&relay_msg::new_relay_key())[2..].to_owned();
    changed.push(e);
    let mut e = base;
    e.id = "00".repeat(32);
    changed.push(e);
    for (i, e) in changed.iter().enumerate() {
        assert!(!e.verify(), "change {i} still verifies");
    }
}

/// NIP-01: escape only `\n " \ \r \t \b \f`; every other character,
/// control characters and non-ASCII included, goes in verbatim.
#[test]
fn event_serialisation_escapes_exactly_what_nip01_lists() {
    let content = "a\n\"\\\r\t\u{8}\u{c}\u{1}é😀/";
    let tags = vec![vec!["p".to_owned(), "x\ny".to_owned()]];
    let got = event::serialize_for_id("ab", 1700000000, 7491, &tags, content);
    let want =
        "[0,\"ab\",1700000000,7491,[[\"p\",\"x\\ny\"]],\"a\\n\\\"\\\\\\r\\t\\b\\f\u{1}é😀/\"]";
    assert_eq!(got, want);
}

#[test]
fn event_signed_with_a_relay_key_names_its_x_coordinate_and_verifies() {
    let key = relay_msg::new_relay_key();
    let tags = vec![vec!["n".to_owned(), "regtest".to_owned()]];
    let e = event::sign(&key, 1800000000, 37491, tags.clone(), "{}".into());
    assert_eq!(e.pubkey, relay_msg::relay_pubkey_hex(&key)[2..]);
    assert_eq!(e.kind, 37491);
    assert_eq!(e.tags, tags);
    assert_eq!(e.id.len(), 64);
    assert_eq!(e.sig.len(), 128);
    assert!(e.verify());
    // The x coordinate of a compressed key, and nothing else.
    assert_eq!(
        event::x_only_of(&relay_msg::relay_pubkey_hex(&key)).as_deref(),
        Some(e.pubkey.as_str())
    );
    assert_eq!(event::x_only_of("04ab"), None);
    // x = 2^256 - 1 is above the field prime: no point.
    assert_eq!(event::x_only_of(&format!("02{}", "ff".repeat(32))), None);
}

#[test]
fn event_tag_values_are_read_by_name() {
    let e = event::sign(
        &relay_msg::new_relay_key(),
        1,
        37491,
        vec![
            vec!["d".into(), "abc".into()],
            vec!["expiration".into(), "99".into()],
        ],
        String::new(),
    );
    assert_eq!(e.tag("d"), Some("abc"));
    assert_eq!(e.tag("expiration"), Some("99"));
    assert_eq!(e.tag("p"), None);
}

// ------------------------------------------------------------------- R2

mod nip44 {
    use crate::exchange_io::nostr::nip44;
    use secp256k1::SecretKey;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    const VECTORS: &str = include_str!("fixtures/nostr/nip44.vectors.json");

    fn v2() -> Value {
        serde_json::from_str::<Value>(VECTORS).unwrap()["v2"].clone()
    }
    fn b32(v: &Value) -> [u8; 32] {
        hex::decode(v.as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap()
    }
    fn sec(v: &Value) -> Option<SecretKey> {
        SecretKey::from_byte_array(b32(v)).ok()
    }

    #[test]
    fn nip44_conversation_key_matches_every_vector() {
        let v = v2();
        for c in v["valid"]["get_conversation_key"].as_array().unwrap() {
            let got = nip44::conversation_key(&sec(&c["sec1"]).unwrap(), &b32(&c["pub2"])).unwrap();
            assert_eq!(hex::encode(got), c["conversation_key"].as_str().unwrap());
        }
    }

    #[test]
    fn nip44_refuses_every_invalid_conversation_key() {
        let v = v2();
        for c in v["invalid"]["get_conversation_key"].as_array().unwrap() {
            let refused = match sec(&c["sec1"]) {
                None => true,
                Some(s) => nip44::conversation_key(&s, &b32(&c["pub2"])).is_err(),
            };
            assert!(refused, "accepted: {}", c["note"]);
        }
    }

    #[test]
    fn nip44_message_keys_match_every_vector() {
        let v = v2();
        let mk = &v["valid"]["get_message_keys"];
        let ck = b32(&mk["conversation_key"]);
        for k in mk["keys"].as_array().unwrap() {
            let (chacha_key, chacha_nonce, hmac_key) = nip44::message_keys(&ck, &b32(&k["nonce"]));
            assert_eq!(hex::encode(chacha_key), k["chacha_key"].as_str().unwrap());
            assert_eq!(
                hex::encode(chacha_nonce),
                k["chacha_nonce"].as_str().unwrap()
            );
            assert_eq!(hex::encode(hmac_key), k["hmac_key"].as_str().unwrap());
        }
    }

    #[test]
    fn nip44_padded_length_matches_every_vector() {
        let v = v2();
        for p in v["valid"]["calc_padded_len"].as_array().unwrap() {
            let (len, padded) = (p[0].as_u64().unwrap(), p[1].as_u64().unwrap());
            assert_eq!(nip44::padded_len(len as usize) as u64, padded, "len {len}");
        }
    }

    #[test]
    fn nip44_encrypts_and_decrypts_every_vector_both_ways() {
        let v = v2();
        for c in v["valid"]["encrypt_decrypt"].as_array().unwrap() {
            let (s1, s2) = (sec(&c["sec1"]).unwrap(), sec(&c["sec2"]).unwrap());
            let secp = secp256k1::Secp256k1::new();
            let x = |s: &SecretKey| s.x_only_public_key(&secp).0.serialize();
            let ck = nip44::conversation_key(&s1, &x(&s2)).unwrap();
            assert_eq!(hex::encode(ck), c["conversation_key"].as_str().unwrap());
            // The recipient derives the same key from its side.
            assert_eq!(nip44::conversation_key(&s2, &x(&s1)).unwrap(), ck);
            let plaintext = c["plaintext"].as_str().unwrap();
            let payload = nip44::encrypt_with_nonce(plaintext, &ck, &b32(&c["nonce"])).unwrap();
            assert_eq!(payload, c["payload"].as_str().unwrap());
            assert_eq!(nip44::decrypt(&payload, &ck).unwrap(), plaintext);
        }
    }

    #[test]
    fn nip44_long_messages_match_their_checksums() {
        let v = v2();
        for c in v["valid"]["encrypt_decrypt_long_msg"].as_array().unwrap() {
            let plaintext = c["pattern"]
                .as_str()
                .unwrap()
                .repeat(c["repeat"].as_u64().unwrap() as usize);
            assert_eq!(
                hex::encode(Sha256::digest(plaintext.as_bytes())),
                c["plaintext_sha256"].as_str().unwrap()
            );
            let ck = b32(&c["conversation_key"]);
            let payload = nip44::encrypt_with_nonce(&plaintext, &ck, &b32(&c["nonce"])).unwrap();
            assert_eq!(
                hex::encode(Sha256::digest(payload.as_bytes())),
                c["payload_sha256"].as_str().unwrap()
            );
            assert_eq!(nip44::decrypt(&payload, &ck).unwrap(), plaintext);
        }
    }

    #[test]
    fn nip44_refuses_every_invalid_payload_and_length() {
        let v = v2();
        for c in v["invalid"]["decrypt"].as_array().unwrap() {
            let got = nip44::decrypt(c["payload"].as_str().unwrap(), &b32(&c["conversation_key"]));
            assert!(got.is_err(), "decrypted: {}", c["note"]);
        }
        // The vectors' lengths, which this code also keeps to: 1..=65535
        // bytes, no extended prefix.
        let ck = [7u8; 32];
        for n in v["invalid"]["encrypt_msg_lengths"].as_array().unwrap() {
            let n = n.as_u64().unwrap() as usize;
            assert!(
                nip44::encrypt_with_nonce(&"a".repeat(n), &ck, &[1u8; 32]).is_err(),
                "encrypted {n} bytes"
            );
        }
    }

    #[test]
    fn nip44_random_nonce_round_trips_and_never_repeats() {
        let ck = [9u8; 32];
        let a = nip44::encrypt("hello", &ck).unwrap();
        let b = nip44::encrypt("hello", &ck).unwrap();
        assert_ne!(a, b);
        assert_eq!(nip44::decrypt(&a, &ck).unwrap(), "hello");
        assert_eq!(nip44::decrypt(&b, &ck).unwrap(), "hello");
        // Another conversation cannot read it.
        assert!(nip44::decrypt(&a, &[8u8; 32]).is_err());
    }
}

// ------------------------------------------------------------------- R8

mod client {
    use super::super::exchange_fake_nostr::{FakeRelay, Mode};
    use crate::exchange_io::nostr::client::NostrClient;
    use crate::exchange_io::nostr::event;
    use crate::noncustodial::exchange::relay_msg;
    use serde_json::json;
    use std::time::Duration;

    fn ev(content: &str) -> event::Event {
        event::sign(
            &relay_msg::new_relay_key(),
            1_800_000_000,
            7491,
            vec![],
            content.into(),
        )
    }

    fn client(urls: &[&str]) -> NostrClient {
        NostrClient::new(urls.iter().map(|u| u.to_string()).collect())
            .with_timeouts(Duration::from_secs(2), Duration::from_secs(2))
    }

    /// A port nothing listens on.
    async fn dead_url() -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", l.local_addr().unwrap());
        drop(l);
        url
    }

    #[tokio::test]
    async fn quorum_publish_needs_two_accepts_of_three() {
        let (a, b, c) = (
            FakeRelay::start(Mode::Accept).await,
            FakeRelay::start(Mode::Accept).await,
            FakeRelay::start(Mode::Refuse).await,
        );
        let e = ev("x");
        let n = client(&[&a.url, &b.url, &c.url]).publish(&e).await.unwrap();
        assert_eq!(n, 2);
        assert_eq!(a.events(), vec![e.clone()]);
        assert_eq!(b.events(), vec![e.clone()]);
        // The refusing relay was asked too.
        assert_eq!(c.received(), vec![e]);
    }

    #[tokio::test]
    async fn quorum_publish_fails_with_one_accept_and_says_why() {
        let (a, b, c) = (
            FakeRelay::start(Mode::Accept).await,
            FakeRelay::start(Mode::Refuse).await,
            FakeRelay::start(Mode::Refuse).await,
        );
        let err = client(&[&a.url, &b.url, &c.url])
            .publish(&ev("x"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("1 of 3"), "{err}");
        assert!(err.contains("blocked: a test relay"), "{err}");
    }

    #[tokio::test]
    async fn quorum_one_relay_needs_its_one_accept() {
        let a = FakeRelay::start(Mode::Accept).await;
        assert_eq!(client(&[&a.url]).publish(&ev("x")).await.unwrap(), 1);
        let r = FakeRelay::start(Mode::Refuse).await;
        assert!(client(&[&r.url]).publish(&ev("x")).await.is_err());
    }

    #[tokio::test]
    async fn quorum_dead_and_silent_relays_count_as_refusals() {
        let (a, silent, dead) = (
            FakeRelay::start(Mode::Accept).await,
            FakeRelay::start(Mode::Silent).await,
            dead_url().await,
        );
        let started = std::time::Instant::now();
        let err = client(&[&a.url, &silent.url, &dead])
            .publish(&ev("x"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("1 of 3"), "{err}");
        // Each relay is asked at the same time, so the answer timeout is
        // paid once.
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn quorum_query_merges_relays_dedupes_and_drops_forged_events() {
        let (a, b) = (
            FakeRelay::start(Mode::Accept).await,
            FakeRelay::start(Mode::Accept).await,
        );
        let (e1, e2) = (ev("one"), ev("two"));
        a.put(e1.clone());
        a.put(e2.clone());
        b.put(e2.clone());
        // A forged copy: the content changed, id and signature kept.
        let mut forged = ev("three");
        forged.content = "changed".into();
        b.add_extra(serde_json::to_value(&forged).unwrap());
        b.add_extra(json!({"not": "an event"}));
        let mut got = client(&[&a.url, &b.url])
            .query(json!({"kinds": [7491]}))
            .await
            .unwrap();
        got.sort_by(|x, y| x.content.cmp(&y.content));
        assert_eq!(got, vec![e1, e2]);
    }

    #[tokio::test]
    async fn quorum_query_needs_one_answering_relay() {
        let (a, silent, dead) = (
            FakeRelay::start(Mode::Accept).await,
            FakeRelay::start(Mode::Silent).await,
            dead_url().await,
        );
        a.put(ev("only"));
        let got = client(&[&silent.url, &dead, &a.url])
            .query(json!({"kinds": [7491]}))
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        let err = client(&[&silent.url, &dead])
            .query(json!({"kinds": [7491]}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("no Nostr relay answered"), "{err}");
    }
}
