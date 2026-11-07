//! Offers and trade messages: signatures, addressing, sizes (spec R18).

use super::exchange_fixture::{self as fx, START};
use crate::noncustodial::exchange::relay_msg::{
    self, Body, Envelope, LockAnnouncement, Message, Offer, Side, SignedOffer,
};

fn offer(maker: &secp256k1::SecretKey) -> Offer {
    Offer {
        version: 1,
        network: "regtest".into(),
        side: Side::Sell,
        price_msat_per_hns: 20_000,
        min_hns: 1_000_000,
        max_hns: 10_000_000_000,
        maker_relay_pubkey: relay_msg::relay_pubkey_hex(maker),
        created_at: START,
        expires_at: START + 3600,
        nonce: relay_msg::random_hex16(),
    }
}

#[test]
fn a_signed_offer_verifies_and_any_change_breaks_it() {
    let k = relay_msg::new_relay_key();
    let o = offer(&k).sign(&k).unwrap();
    assert!(o.verify());
    let json = serde_json::to_string(&o).unwrap();
    assert_eq!(SignedOffer::parse(&json).unwrap(), o);
    let mut cheaper = o.clone();
    cheaper.offer.price_msat_per_hns -= 1;
    assert!(!cheaper.verify());
    let mut other = o.clone();
    other.offer.maker_relay_pubkey = relay_msg::relay_pubkey_hex(&relay_msg::new_relay_key());
    assert!(!other.verify());
    assert!(
        offer(&k).sign(&relay_msg::new_relay_key()).is_err(),
        "only the maker's key signs"
    );
}

#[test]
fn offer_ids_are_unique_by_nonce() {
    let k = relay_msg::new_relay_key();
    assert_ne!(offer(&k).id(), offer(&k).id());
}

#[test]
fn an_offer_of_bad_shape_is_refused() {
    let k = relay_msg::new_relay_key();
    let mut o = offer(&k);
    assert!(o.check_shape(START).is_ok());
    assert!(o.check_shape(START + 3600).is_err(), "expired");
    o.max_hns = 0;
    assert!(o.check_shape(START).is_err());
    let mut o = offer(&k);
    o.network = "litecoin".into();
    assert!(o.check_shape(START).is_err());
}

fn message(from: &secp256k1::SecretKey, to: &str, body: Body) -> Message {
    Message {
        version: 1,
        network: "regtest".into(),
        offer_id: "ab".repeat(32),
        trade_id: "cd".repeat(16),
        from: relay_msg::relay_pubkey_hex(from),
        to: to.into(),
        sent_at: START,
        body,
    }
}

#[test]
fn a_message_reaches_only_its_addressee_and_only_signed() {
    let a = relay_msg::new_relay_key();
    let b = relay_msg::new_relay_key();
    let bh = relay_msg::relay_pubkey_hex(&b);
    let terms = fx::terms_at(START);
    let e = message(
        &a,
        &bh,
        Body::Accept {
            terms: Box::new(terms.clone()),
        },
    )
    .sign(&a)
    .unwrap();
    let json = serde_json::to_string(&e).unwrap();
    let got = Envelope::parse_for(&json, &bh, "regtest").unwrap();
    assert_eq!(
        got.message.body,
        Body::Accept {
            terms: Box::new(terms)
        }
    );
    assert!(Envelope::parse_for(&json, &relay_msg::relay_pubkey_hex(&a), "regtest").is_err());
    assert!(Envelope::parse_for(&json, &bh, "main").is_err());
    // Tampered body.
    let mut t = e.clone();
    t.message.body = Body::Decline { reason: "x".into() };
    assert!(Envelope::parse_for(&serde_json::to_string(&t).unwrap(), &bh, "regtest").is_err());
    // Signed by someone else claiming to be `a`.
    let forged = message(&a, &bh, Body::Decline { reason: "x".into() });
    assert!(forged.sign(&b).is_err());
}

#[test]
fn messages_round_trip_every_kind() {
    let a = relay_msg::new_relay_key();
    let bh = relay_msg::relay_pubkey_hex(&relay_msg::new_relay_key());
    let lock = LockAnnouncement {
        terms_id: "11".repeat(32),
        txid: "22".repeat(32),
        vout: 0,
    };
    for body in [
        Body::Take(relay_msg::TakeRequest {
            hns_amount: 5,
            hns_pubkey: "02".repeat(33),
            btc_pubkey: "03".repeat(33),
            hash: Some("44".repeat(32)),
            start: START,
        }),
        Body::BtcLocked(lock.clone()),
        Body::HnsLocked(lock),
        Body::Decline {
            reason: "no".into(),
        },
    ] {
        let e = message(&a, &bh, body.clone()).sign(&a).unwrap();
        let back =
            Envelope::parse_for(&serde_json::to_string(&e).unwrap(), &bh, "regtest").unwrap();
        assert_eq!(back.message.body, body);
    }
}

#[test]
fn an_oversized_message_is_refused_before_parsing() {
    let big = "x".repeat(relay_msg::MAX_MESSAGE_BYTES + 1);
    assert!(Envelope::parse_for(&big, "", "regtest")
        .unwrap_err()
        .to_string()
        .contains("too large"));
    assert!(SignedOffer::parse(&big)
        .unwrap_err()
        .to_string()
        .contains("too large"));
}

#[test]
fn only_the_maker_can_withdraw_its_offer() {
    let k = relay_msg::new_relay_key();
    let o = offer(&k);
    let id = o.id();
    let sig = relay_msg::sign_withdraw(&k, &id);
    assert!(relay_msg::verify_withdraw(&o.maker_relay_pubkey, &id, &sig));
    assert!(!relay_msg::verify_withdraw(
        &o.maker_relay_pubkey,
        &"00".repeat(32),
        &sig
    ));
    let other = relay_msg::new_relay_key();
    assert!(!relay_msg::verify_withdraw(
        &o.maker_relay_pubkey,
        &id,
        &relay_msg::sign_withdraw(&other, &id)
    ));
}

#[test]
fn a_status_crosses_the_bridge_in_camel_case() {
    use crate::noncustodial::exchange::engine::{Chain, Status};
    let s = serde_json::to_value(Status::AwaitingRefund {
        chain: Chain::Btc,
        opens_at: 5,
    })
    .unwrap();
    assert_eq!(
        s,
        serde_json::json!({"kind": "awaitingRefund", "chain": "btc", "opensAt": 5})
    );
    let s = serde_json::to_value(Status::AwaitingReveal { refund_opens_at: 7 }).unwrap();
    assert_eq!(
        s,
        serde_json::json!({"kind": "awaitingReveal", "refundOpensAt": 7})
    );
}
