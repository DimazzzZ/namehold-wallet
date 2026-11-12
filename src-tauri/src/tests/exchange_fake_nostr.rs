//! A Nostr relay for tests: a WebSocket server on 127.0.0.1 that speaks the
//! part of NIP-01 the exchange uses (`EVENT` → `OK`, `REQ` → `EVENT`s and
//! `EOSE`, `CLOSE`), keeps addressable events by (kind, author, `d`) and
//! applies kind-5 deletions of an address, as NIP-01 and NIP-09 describe.
//! It can also refuse every event, stay silent, or serve raw extra frames
//! (a forged event, say).

use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::exchange_io::nostr::event::Event;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Accept,
    Refuse,
    Silent,
    /// Drop the TCP connection on the first frame, with no close handshake.
    Drop,
}

#[derive(Default)]
pub struct State {
    pub events: Vec<Event>,
    /// Every `EVENT` received, kept even when refused.
    pub received: Vec<Event>,
    /// Every `REQ` filter received.
    pub filters: Vec<Value>,
    /// Raw frames sent before `EOSE` on every `REQ` (`["EVENT", sub, …]`
    /// with the subscription id put in by the relay).
    pub extra_events: Vec<Value>,
    pub mode: Option<Mode>,
}

pub struct FakeRelay {
    pub url: String,
    pub state: Arc<Mutex<State>>,
}

impl FakeRelay {
    pub async fn start(mode: Mode) -> FakeRelay {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            mode: Some(mode),
            ..State::default()
        }));
        let st = state.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let st = st.clone();
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(tcp).await else {
                        return;
                    };
                    while let Some(Ok(msg)) = ws.next().await {
                        if st.lock().unwrap().mode == Some(Mode::Drop) {
                            return; // the socket is dropped unclosed
                        }
                        let Message::Text(text) = msg else { continue };
                        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        for out in handle(&st, &frame) {
                            if ws
                                .send(Message::Text(out.to_string().into()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                });
            }
        });
        FakeRelay { url, state }
    }

    pub fn events(&self) -> Vec<Event> {
        self.state.lock().unwrap().events.clone()
    }
    pub fn received(&self) -> Vec<Event> {
        self.state.lock().unwrap().received.clone()
    }
    pub fn filters(&self) -> Vec<Value> {
        self.state.lock().unwrap().filters.clone()
    }
    pub fn add_extra(&self, event: Value) {
        self.state.lock().unwrap().extra_events.push(event);
    }
    pub fn put(&self, e: Event) {
        self.state.lock().unwrap().events.push(e);
    }
}

fn address(e: &Event) -> Option<(u32, String, String)> {
    (30000..40000).contains(&e.kind).then(|| {
        (
            e.kind,
            e.pubkey.clone(),
            e.tag("d").unwrap_or("").to_owned(),
        )
    })
}

fn handle(st: &Arc<Mutex<State>>, frame: &Value) -> Vec<Value> {
    let mut s = st.lock().unwrap();
    let mode = s.mode.unwrap_or(Mode::Accept);
    match frame[0].as_str() {
        Some("EVENT") => {
            let Ok(e) = serde_json::from_value::<Event>(frame[1].clone()) else {
                return vec![];
            };
            s.received.push(e.clone());
            match mode {
                Mode::Silent | Mode::Drop => vec![],
                Mode::Refuse => vec![json!(["OK", e.id, false, "blocked: a test relay"])],
                Mode::Accept => {
                    if !e.verify() {
                        return vec![json!(["OK", e.id, false, "invalid: bad signature"])];
                    }
                    if e.kind == 5 {
                        for a in e
                            .tags
                            .iter()
                            .filter(|t| t.first().map(String::as_str) == Some("a"))
                        {
                            let parts: Vec<&str> = a[1].splitn(3, ':').collect();
                            if parts.len() == 3 && parts[1] == e.pubkey {
                                let (k, d) = (parts[0].parse::<u32>().unwrap_or(0), parts[2]);
                                s.events.retain(|x| {
                                    !(x.kind == k
                                        && x.pubkey == e.pubkey
                                        && x.tag("d") == Some(d)
                                        && x.created_at <= e.created_at)
                                });
                            }
                        }
                    }
                    if let Some(addr) = address(&e) {
                        if s.events.iter().any(|x| {
                            address(x).as_ref() == Some(&addr) && x.created_at > e.created_at
                        }) {
                            return vec![json!(["OK", e.id, true, "duplicate: have a newer one"])];
                        }
                        s.events.retain(|x| address(x).as_ref() != Some(&addr));
                    }
                    s.events.push(e.clone());
                    vec![json!(["OK", e.id, true, ""])]
                }
            }
        }
        Some("REQ") => {
            let sub = frame[1].clone();
            let filter = frame[2].clone();
            s.filters.push(filter.clone());
            if mode == Mode::Silent {
                return vec![];
            }
            let mut out: Vec<Value> = s
                .events
                .iter()
                .filter(|e| matches(&filter, e))
                .map(|e| json!(["EVENT", sub, e]))
                .collect();
            for extra in &s.extra_events {
                out.push(json!(["EVENT", sub, extra]));
            }
            out.push(json!(["EOSE", sub]));
            out
        }
        _ => vec![],
    }
}

/// NIP-01 filter matching, for the fields the exchange uses.
pub fn matches(f: &Value, e: &Event) -> bool {
    if let Some(kinds) = f["kinds"].as_array() {
        if !kinds.iter().any(|k| k.as_u64() == Some(e.kind as u64)) {
            return false;
        }
    }
    if let Some(authors) = f["authors"].as_array() {
        if !authors
            .iter()
            .any(|a| a.as_str() == Some(e.pubkey.as_str()))
        {
            return false;
        }
    }
    if let Some(since) = f["since"].as_u64() {
        if e.created_at < since {
            return false;
        }
    }
    if let Some(until) = f["until"].as_u64() {
        if e.created_at > until {
            return false;
        }
    }
    if let Some(obj) = f.as_object() {
        for (k, vals) in obj.iter().filter(|(k, _)| k.starts_with('#')) {
            let name = &k[1..];
            let wanted: Vec<&str> = vals
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let has = e.tags.iter().any(|t| {
                t.first().map(String::as_str) == Some(name)
                    && t.get(1).is_some_and(|v| wanted.contains(&v.as_str()))
            });
            if !has {
                return false;
            }
        }
    }
    true
}
