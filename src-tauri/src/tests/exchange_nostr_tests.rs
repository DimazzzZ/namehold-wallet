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
