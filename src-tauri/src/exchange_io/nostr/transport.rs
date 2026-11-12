//! The Nostr meeting point (spec 2026-11-12 R3–R7, R12): offers as
//! addressable events, messages as NIP-44-encrypted regular events, both
//! signed by the relay key that signed their content.
//!
//! The content is what a `namehold-relay` received before: the `SignedOffer`
//! and the `Envelope` of `relay_msg`, which stay their own verifiers. This
//! layer adds one binding: an event's author is the x coordinate of the key
//! its content names (`makerRelayPubkey`, or `from`). So only the maker can
//! renew or withdraw an offer, and only the sender of a message can be its
//! author.

use std::collections::HashMap;

use async_trait::async_trait;
use secp256k1::{Secp256k1, SecretKey};
use serde_json::json;

use crate::error::AppError;
use crate::exchange_io::meeting::{MeetingPoint, NOSTR_CURSOR_PREFIX};
use crate::exchange_io::nostr::client::NostrClient;
use crate::exchange_io::nostr::event::{self, Event};
use crate::exchange_io::nostr::nip44;
use crate::noncustodial::exchange::relay_msg::{self, Envelope, SignedOffer};

pub const OFFER_KIND: u32 = 37491;
pub const MESSAGE_KIND: u32 = 7491;
/// An offer event lives a few heartbeats (NIP-40), not the offer's week.
pub const OFFER_EVENT_SECS: u64 = 300;
/// Offers renewed within this window are live (as on `namehold-relay`).
pub const LIVE_WINDOW_SECS: u64 = crate::relay_server::handlers::LIVE_WINDOW_SECS;
/// How long a message is kept (as on `namehold-relay`).
pub const MESSAGE_SECS: u64 = crate::relay_server::handlers::MESSAGE_TTL_SECS;
/// The inbox reads this far before its cursor: relays have no arrival
/// order, and a sender's clock may lag (strfry refuses events more than
/// 900 s in the future).
pub const INBOX_SLACK_SECS: u64 = 900;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn x_only(key: &SecretKey) -> [u8; 32] {
    key.x_only_public_key(&Secp256k1::new()).0.serialize()
}

fn x_only_bytes(hex_x: &str) -> Option<[u8; 32]> {
    hex::decode(hex_x).ok()?.try_into().ok()
}

fn refuse(why: impl Into<String>) -> AppError {
    AppError::InvalidInput(why.into())
}

pub struct NostrMeetingPoint {
    client: NostrClient,
    clock: fn() -> u64,
}

impl NostrMeetingPoint {
    pub fn new(client: NostrClient) -> NostrMeetingPoint {
        NostrMeetingPoint {
            client,
            clock: now_secs,
        }
    }

    /// A meeting point that reads the time from `clock` (tests).
    pub fn with_clock(client: NostrClient, clock: fn() -> u64) -> NostrMeetingPoint {
        NostrMeetingPoint { client, clock }
    }

    fn offer_event(&self, d: &str, network: &str, content: String, key: &SecretKey) -> Event {
        let now = (self.clock)();
        event::sign(
            key,
            now,
            OFFER_KIND,
            vec![
                vec!["d".into(), d.into()],
                vec!["n".into(), network.into()],
                vec!["expiration".into(), (now + OFFER_EVENT_SECS).to_string()],
            ],
            content,
        )
    }
}

/// The newest version of each (author, `d`) address; on a tie the lowest id
/// (NIP-01).
fn newest_per_address(events: Vec<Event>) -> Vec<Event> {
    let mut best: HashMap<(String, String), Event> = HashMap::new();
    for e in events {
        let Some(d) = e.tag("d").map(str::to_owned) else {
            continue;
        };
        let key = (e.pubkey.clone(), d);
        let replace = match best.get(&key) {
            None => true,
            Some(b) => e.created_at > b.created_at || (e.created_at == b.created_at && e.id < b.id),
        };
        if replace {
            best.insert(key, e);
        }
    }
    best.into_values().collect()
}

#[async_trait]
impl MeetingPoint for NostrMeetingPoint {
    async fn publish_offer(&self, o: &SignedOffer, key: &SecretKey) -> Result<(), AppError> {
        if relay_msg::relay_pubkey_hex(key) != o.offer.maker_relay_pubkey {
            return Err(refuse("the offer is not signed by this relay key"));
        }
        let content = serde_json::to_string(o).map_err(|e| AppError::Other(e.to_string()))?;
        let e = self.offer_event(&o.offer.id(), &o.offer.network, content, key);
        self.client.publish(&e).await.map(|_| ())
    }

    async fn withdraw_offer(
        &self,
        o: &SignedOffer,
        signature: &str,
        key: &SecretKey,
    ) -> Result<(), AppError> {
        let id = o.offer.id();
        if relay_msg::relay_pubkey_hex(key) != o.offer.maker_relay_pubkey {
            return Err(refuse("the offer is not signed by this relay key"));
        }
        let content = json!({
            "withdrawn": id,
            "maker": o.offer.maker_relay_pubkey,
            "signature": signature,
        })
        .to_string();
        let tomb = self.offer_event(&id, &o.offer.network, content, key);
        self.client.publish(&tomb).await?;
        // NIP-09: relays that honour it drop every version up to now. The
        // replacement above is what hides the offer either way.
        let deletion = event::sign(
            key,
            tomb.created_at,
            5,
            vec![
                vec!["a".into(), format!("{OFFER_KIND}:{}:{id}", tomb.pubkey)],
                vec!["k".into(), OFFER_KIND.to_string()],
            ],
            String::new(),
        );
        let _ = self.client.publish(&deletion).await;
        Ok(())
    }

    async fn offers(&self, network: &str, now: u64) -> Result<(Vec<SignedOffer>, usize), AppError> {
        let events = self
            .client
            .query(json!({
                "kinds": [OFFER_KIND],
                "#n": [network],
                "since": now.saturating_sub(LIVE_WINDOW_SECS),
            }))
            .await?;
        let mut good = Vec::new();
        let mut dropped = 0;
        for e in newest_per_address(events) {
            if serde_json::from_str::<serde_json::Value>(&e.content)
                .is_ok_and(|v| v.get("withdrawn").is_some())
            {
                continue; // withdrawn by its maker: the newest version says so
            }
            let ok = SignedOffer::parse(&e.content).ok().filter(|o| {
                Some(o.offer.id().as_str()) == e.tag("d")
                    && event::x_only_of(&o.offer.maker_relay_pubkey).as_deref()
                        == Some(e.pubkey.as_str())
                    && o.offer.network == network
                    && o.offer.check_shape(now).is_ok()
            });
            match ok {
                Some(o) => good.push(o),
                None => dropped += 1,
            }
        }
        good.sort_by_key(|o| o.offer.id());
        Ok((good, dropped))
    }

    async fn send(&self, e: &Envelope, key: &SecretKey) -> Result<(), AppError> {
        if relay_msg::relay_pubkey_hex(key) != e.message.from {
            return Err(refuse("the message is not signed by this relay key"));
        }
        let to_x = event::x_only_of(&e.message.to)
            .and_then(|x| x_only_bytes(&x))
            .ok_or_else(|| refuse("the message's recipient is not a public key"))?;
        let ck = nip44::conversation_key(key, &to_x)?;
        let plain = serde_json::to_string(e).map_err(|e| AppError::Other(e.to_string()))?;
        let now = (self.clock)();
        let ev = event::sign(
            key,
            now,
            MESSAGE_KIND,
            vec![
                vec!["p".into(), hex::encode(to_x)],
                vec!["n".into(), e.message.network.clone()],
                vec!["expiration".into(), (now + MESSAGE_SECS).to_string()],
            ],
            nip44::encrypt(&plain, &ck)?,
        );
        self.client.publish(&ev).await.map(|_| ())
    }

    async fn inbox(
        &self,
        key: &SecretKey,
        network: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Envelope>, Option<String>), AppError> {
        let now = (self.clock)();
        let me = relay_msg::relay_pubkey_hex(key);
        let me_x = hex::encode(x_only(key));
        let last = cursor
            .and_then(|c| c.strip_prefix(NOSTR_CURSOR_PREFIX))
            .and_then(|c| c.parse::<u64>().ok());
        let since = match last {
            Some(t) => t.saturating_sub(INBOX_SLACK_SECS),
            None => now.saturating_sub(MESSAGE_SECS),
        };
        let mut events = self
            .client
            .query(json!({"kinds": [MESSAGE_KIND], "#p": [me_x], "since": since}))
            .await?;
        events.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        let mut msgs = Vec::new();
        let mut newest = last.unwrap_or(0);
        for e in &events {
            newest = newest.max(e.created_at.min(now));
            let Some(author) = x_only_bytes(&e.pubkey) else {
                continue;
            };
            let Ok(ck) = nip44::conversation_key(key, &author) else {
                continue;
            };
            let Ok(plain) = nip44::decrypt(&e.content, &ck) else {
                continue;
            };
            let Ok(env) = Envelope::parse_for(&plain, &me, network) else {
                continue;
            };
            if event::x_only_of(&env.message.from).as_deref() != Some(e.pubkey.as_str()) {
                continue; // a message re-sent under another author
            }
            msgs.push(env);
        }
        let next = (newest > 0).then(|| format!("{NOSTR_CURSOR_PREFIX}{newest}"));
        Ok((msgs, next.or_else(|| cursor.map(str::to_owned))))
    }
}
