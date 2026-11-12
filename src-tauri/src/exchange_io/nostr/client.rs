//! A minimal NIP-01 client: publish one event to every relay, or read one
//! filter from every relay, all relays at once (spec 2026-11-12 R8).
//!
//! A publish counts when at least `min(2, relays)` relays answer `OK true`;
//! a read counts when at least one relay ends its answer with `EOSE`. Every
//! event read is checked (id and signature, R1) and duplicates across relays
//! are dropped. One WebSocket per relay per call: the exchange step runs
//! about once a minute, so nothing is kept open between calls.
//!
//! TLS is the project's rustls with the ring provider, given explicitly, and
//! the platform's certificate verifier. A WebSocket handshake answered with
//! a redirect fails; it is never followed.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rustls_platform_verifier::BuilderVerifierExt;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};

use crate::error::AppError;
use crate::exchange_io::nostr::event::Event;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest frame a relay may send us; exchange events are a few kilobytes.
const MAX_FRAME_BYTES: usize = 1 << 20;
/// Most events kept from one relay for one read.
const MAX_EVENTS_PER_RELAY: usize = 1000;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn tls_config() -> Result<Arc<rustls::ClientConfig>, AppError> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .with_platform_verifier()
                .map_err(|e| e.to_string())
                .map(|b| Arc::new(b.with_no_client_auth()))
        })
        .clone()
        .map_err(|e| AppError::Other(format!("TLS for Nostr relays: {e}")))
}

pub struct NostrClient {
    relays: Vec<String>,
    connect: Duration,
    answer: Duration,
}

impl NostrClient {
    pub fn new(relays: Vec<String>) -> NostrClient {
        NostrClient {
            relays,
            connect: CONNECT_TIMEOUT,
            answer: ANSWER_TIMEOUT,
        }
    }

    pub fn with_timeouts(mut self, connect: Duration, answer: Duration) -> NostrClient {
        self.connect = connect;
        self.answer = answer;
        self
    }

    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    async fn open(&self, url: &str) -> Result<Socket, String> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES));
        let connector = if url.starts_with("wss://") {
            Some(Connector::Rustls(tls_config().map_err(|e| e.to_string())?))
        } else {
            Some(Connector::Plain)
        };
        match tokio::time::timeout(
            self.connect,
            connect_async_tls_with_config(url, Some(config), false, connector),
        )
        .await
        {
            Err(_) => Err("no connection in time".into()),
            Ok(Err(e)) => Err(e.to_string()),
            Ok(Ok((socket, _))) => Ok(socket),
        }
    }

    /// Send `event` to every relay; `Ok(accepted)` when enough took it.
    pub async fn publish(&self, event: &Event) -> Result<usize, AppError> {
        if self.relays.is_empty() {
            return Err(AppError::InvalidInput("no Nostr relay is set".into()));
        }
        let frame = json!(["EVENT", event]).to_string();
        let answers = futures_util::future::join_all(
            self.relays
                .iter()
                .map(|url| self.publish_one(url, &frame, &event.id)),
        )
        .await;
        let accepted = answers.iter().filter(|a| a.is_ok()).count();
        let need = self.relays.len().min(2);
        if accepted >= need {
            return Ok(accepted);
        }
        let why: Vec<String> = self
            .relays
            .iter()
            .zip(&answers)
            .filter_map(|(url, a)| a.as_ref().err().map(|e| format!("{url}: {e}")))
            .collect();
        Err(AppError::Other(format!(
            "Nostr: {accepted} of {} relays took the event ({need} needed): {}",
            self.relays.len(),
            why.join("; ")
        )))
    }

    async fn publish_one(&self, url: &str, frame: &str, id: &str) -> Result<(), String> {
        let mut socket = self.open(url).await?;
        socket
            .send(Message::Text(frame.to_owned().into()))
            .await
            .map_err(|e| e.to_string())?;
        let answer = tokio::time::timeout(self.answer, async {
            while let Some(msg) = socket.next().await {
                let Ok(Message::Text(text)) = msg else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if v[0] == "OK" && v[1] == id {
                    let reason = v[3].as_str().unwrap_or("").to_owned();
                    return if v[2] == true {
                        Ok(())
                    } else {
                        Err(if reason.is_empty() {
                            "refused".into()
                        } else {
                            reason
                        })
                    };
                }
            }
            Err("closed without an answer".into())
        })
        .await
        .unwrap_or_else(|_| Err("no answer in time".into()));
        let _ = socket.close(None).await;
        answer
    }

    /// Every valid event matching `filter`, from all relays, without
    /// duplicates; an error only when no relay answered.
    pub async fn query(&self, filter: Value) -> Result<Vec<Event>, AppError> {
        if self.relays.is_empty() {
            return Err(AppError::InvalidInput("no Nostr relay is set".into()));
        }
        let sub = crate::noncustodial::exchange::relay_msg::random_hex16();
        let frame = json!(["REQ", sub, filter]).to_string();
        let answers = futures_util::future::join_all(
            self.relays
                .iter()
                .map(|url| self.query_one(url, &frame, &sub)),
        )
        .await;
        let mut seen = HashSet::new();
        let mut events = Vec::new();
        let mut why = Vec::new();
        let mut answered = 0;
        for (url, a) in self.relays.iter().zip(answers) {
            match a {
                Ok(list) => {
                    answered += 1;
                    for e in list {
                        if e.verify() && seen.insert(e.id.clone()) {
                            events.push(e);
                        }
                    }
                }
                Err(e) => why.push(format!("{url}: {e}")),
            }
        }
        if answered == 0 {
            return Err(AppError::Other(format!(
                "no Nostr relay answered: {}",
                why.join("; ")
            )));
        }
        Ok(events)
    }

    async fn query_one(&self, url: &str, frame: &str, sub: &str) -> Result<Vec<Event>, String> {
        let mut socket = self.open(url).await?;
        socket
            .send(Message::Text(frame.to_owned().into()))
            .await
            .map_err(|e| e.to_string())?;
        let result = tokio::time::timeout(self.answer, async {
            let mut events = Vec::new();
            while let Some(msg) = socket.next().await {
                let Ok(Message::Text(text)) = msg else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if v[1] != sub {
                    continue;
                }
                match v[0].as_str() {
                    Some("EVENT") => {
                        if events.len() < MAX_EVENTS_PER_RELAY {
                            if let Ok(e) = serde_json::from_value::<Event>(v[2].clone()) {
                                events.push(e);
                            }
                        }
                    }
                    Some("EOSE") => return Ok(events),
                    Some("CLOSED") => {
                        return Err(format!("refused the read: {}", v[2].as_str().unwrap_or("")))
                    }
                    _ => {}
                }
            }
            Err("closed before the end of its answer".into())
        })
        .await
        .unwrap_or_else(|_| Err("no answer in time".into()));
        let _ = socket
            .send(Message::Text(json!(["CLOSE", sub]).to_string().into()))
            .await;
        let _ = socket.close(None).await;
        result
    }
}
