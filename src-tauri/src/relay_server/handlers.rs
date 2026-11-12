//! The relay and watchtower API v1 (spec §8), without the HTTP binding:
//! each endpoint is a function of the store, the configuration, the request
//! and the clock, so every rule is tested without a socket.
//!
//! The server keeps nothing it cannot verify: an offer or a message is
//! stored only once its signature checks (`relay_msg`), a watch job only
//! once `WatchJob::check` passes. It never sees a private key or a secret
//! that is not already public, and nothing it stores can move a coin.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::noncustodial::exchange::engine::{self, Role, SentLog};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::relay_msg::{self, Envelope, SignedOffer, MAX_MESSAGE_BYTES};
use crate::noncustodial::exchange::watch::WatchJob;
use crate::relay_server::config::Config;
use crate::relay_server::store::{self, JobRow, OfferRow, Store};

/// An offer is listed while its maker renewed it within this many seconds.
pub const LIVE_WINDOW_SECS: u64 = 180;
/// At most this many live offers per maker key.
pub const MAX_LIVE_OFFERS_PER_KEY: u64 = 20;
/// `GET /v1/offers` lists at most this many.
pub const MAX_OFFERS_LISTED: u64 = 500;
/// At most this many messages per sender key in any hour.
pub const MAX_MESSAGES_PER_HOUR: u64 = 200;
const HOUR: u64 = 3600;
/// Messages are kept this long.
pub const MESSAGE_TTL_SECS: u64 = 7 * 24 * HOUR;
/// `GET /v1/messages` returns at most this many.
pub const MAX_MESSAGES_LISTED: u64 = 200;
/// A finished watch job is kept this long, then purged.
pub const FINISHED_JOB_TTL_SECS: u64 = 30 * 24 * HOUR;
/// An offer nobody renewed for this long is purged (a withdrawn one is kept
/// until it expires, so its withdrawal holds).
pub const SILENT_OFFER_TTL_SECS: u64 = 7 * 24 * HOUR;

/// A successful answer: an HTTP status and its JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

/// A refusal: an HTTP status and the sentence of `{"error": …}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    pub fn new(status: u16, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            message: message.into(),
        }
    }

    fn bad(message: impl Into<String>) -> ApiError {
        ApiError::new(400, message)
    }

    pub fn body(&self) -> Value {
        json!({ "error": self.message })
    }

    pub fn into_reply(self) -> Reply {
        Reply {
            status: self.status,
            body: self.body(),
        }
    }
}

/// A refusal of the request is its sentence; anything else is the server's
/// fault, logged by the binding and answered without detail.
impl From<AppError> for ApiError {
    fn from(e: AppError) -> ApiError {
        match e {
            AppError::InvalidInput(m) => ApiError::bad(m),
            AppError::NotFound(m) => ApiError::new(404, m),
            other => ApiError::new(500, format!("the server failed: {other}")),
        }
    }
}

pub type ApiResult = Result<Reply, ApiError>;

fn reply(status: u16, body: Value) -> ApiResult {
    Ok(Reply { status, body })
}

fn check_size(body: &str) -> Result<(), ApiError> {
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(ApiError::new(
            413,
            format!("the body is larger than {MAX_MESSAGE_BYTES} bytes"),
        ));
    }
    Ok(())
}

/// A compressed secp256k1 public key, 66 lowercase hex.
fn is_pubkey_hex(s: &str) -> bool {
    s.len() == 66
        && !s.chars().any(|c| c.is_ascii_uppercase())
        && hex::decode(s)
            .ok()
            .is_some_and(|b| secp256k1::PublicKey::from_slice(&b).is_ok())
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn query_param(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

fn stored_json(s: &str) -> Result<Value, ApiError> {
    serde_json::from_str(s)
        .map_err(|e| ApiError::new(500, format!("the server's record is corrupted: {e}")))
}

fn role_name(r: Role) -> &'static str {
    match r {
        Role::Buyer => "buyer",
        Role::Seller => "seller",
    }
}

// ------------------------------------------------------------------ health

/// `GET /v1/health`.
pub fn health(cfg: &Config) -> Reply {
    Reply {
        status: 200,
        body: json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "networks": cfg.network_names(),
        }),
    }
}

// ------------------------------------------------------------------ offers

/// `POST /v1/offers`: publish a signed offer, or renew it (its heartbeat).
/// 201 when stored, 200 when renewed.
pub fn post_offer(store: &Store, cfg: &Config, body: &str, now: u64) -> ApiResult {
    check_size(body)?;
    let signed = SignedOffer::parse(body)?;
    let o = &signed.offer;
    o.check_shape(now)?;
    if cfg.serves(&o.network).is_none() {
        return Err(ApiError::bad(format!(
            "this relay does not serve the network '{}'",
            o.network
        )));
    }
    let id = o.id();
    let c = store.lock()?;
    let too_many = || {
        ApiError::new(
            429,
            format!("a maker key may have at most {MAX_LIVE_OFFERS_PER_KEY} live offers"),
        )
    };
    if let Some(row) = store::get_offer(&c, &id)? {
        if row.withdrawn_at.is_some() {
            return Err(ApiError::new(410, "the offer was withdrawn"));
        }
        // A silent offer that comes back becomes live again: it counts.
        if !store::offer_is_live(&c, &id, now, LIVE_WINDOW_SECS)?
            && store::count_live_by_maker(&c, &o.maker_relay_pubkey, now, LIVE_WINDOW_SECS)?
                >= MAX_LIVE_OFFERS_PER_KEY
        {
            return Err(too_many());
        }
        store::touch_offer(&c, &id, now)?;
        return reply(200, json!({ "id": id }));
    }
    if store::count_live_by_maker(&c, &o.maker_relay_pubkey, now, LIVE_WINDOW_SECS)?
        >= MAX_LIVE_OFFERS_PER_KEY
    {
        return Err(too_many());
    }
    let json = serde_json::to_string(&signed).map_err(AppError::from)?;
    store::insert_offer(
        &c,
        &OfferRow {
            id: id.clone(),
            network: o.network.clone(),
            maker_key: o.maker_relay_pubkey.clone(),
            json,
            created_at: now,
            last_seen: now,
            expires_at: o.expires_at,
            withdrawn_at: None,
        },
    )?;
    reply(201, json!({ "id": id }))
}

/// `GET /v1/offers?network=<n>`: the live offers of `n`, newest first.
pub fn list_offers(store: &Store, cfg: &Config, query: &str, now: u64) -> ApiResult {
    let Some(network) = query_param(query, "network") else {
        return Err(ApiError::bad("the network is missing (?network=…)"));
    };
    if cfg.serves(&network).is_none() {
        return Err(ApiError::bad(format!(
            "this relay does not serve the network '{network}'"
        )));
    }
    let c = store.lock()?;
    let rows = store::live_offers(&c, &network, now, LIVE_WINDOW_SECS, MAX_OFFERS_LISTED)?;
    let offers = rows
        .iter()
        .map(|r| stored_json(&r.json))
        .collect::<Result<Vec<_>, _>>()?;
    reply(200, json!({ "offers": offers, "now": now }))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WithdrawBody {
    signature: String,
}

/// `POST /v1/offers/<id>/withdraw`: the maker takes its offer down.
pub fn withdraw_offer(store: &Store, id: &str, body: &str, now: u64) -> ApiResult {
    check_size(body)?;
    let w: WithdrawBody =
        serde_json::from_str(body).map_err(|e| ApiError::bad(format!("bad withdrawal: {e}")))?;
    let c = store.lock()?;
    let Some(row) = store::get_offer(&c, id)? else {
        return Err(ApiError::new(404, "no such offer"));
    };
    if !relay_msg::verify_withdraw(&row.maker_key, id, &w.signature) {
        return Err(ApiError::new(
            403,
            "the withdrawal is not signed by the offer's maker key",
        ));
    }
    store::withdraw_offer(&c, id, now)?;
    reply(200, json!({}))
}

// ---------------------------------------------------------------- messages

/// `POST /v1/messages`: store a signed message for its addressee. Posting
/// the same body again stores nothing and answers 200.
pub fn post_message(store: &Store, cfg: &Config, body: &str, now: u64) -> ApiResult {
    check_size(body)?;
    let e: Envelope =
        serde_json::from_str(body).map_err(|e| ApiError::bad(format!("bad message: {e}")))?;
    let m = &e.message;
    if m.version != relay_msg::MSG_VERSION {
        return Err(ApiError::bad("unknown message version"));
    }
    if cfg.serves(&m.network).is_none() {
        return Err(ApiError::bad(format!(
            "this relay does not serve the network '{}'",
            m.network
        )));
    }
    if !is_pubkey_hex(&m.to) {
        return Err(ApiError::bad("the addressee is not a public key"));
    }
    if !e.verify() {
        return Err(ApiError::bad("the message's signature is invalid"));
    }
    let id = hex::encode(Sha256::digest(body.as_bytes()));
    let c = store.lock()?;
    if store::message_exists(&c, &id)? {
        return reply(200, json!({ "id": id }));
    }
    if store::count_from_since(&c, &m.from, now.saturating_sub(HOUR))? >= MAX_MESSAGES_PER_HOUR {
        return Err(ApiError::new(
            429,
            format!("a sender key may send at most {MAX_MESSAGES_PER_HOUR} messages an hour"),
        ));
    }
    let json = serde_json::to_string(&e).map_err(AppError::from)?;
    store::insert_message(&c, &id, &m.to, &m.from, &json, now)?;
    reply(201, json!({ "id": id }))
}

/// `GET /v1/messages?to=<pubkey>&after=<cursor>`: messages to `to` in
/// arrival order. The cursor is opaque to clients; it is the last stored
/// row's sequence number, and the one given back when nothing is new.
pub fn list_messages(store: &Store, query: &str, now: u64) -> ApiResult {
    let Some(to) = query_param(query, "to") else {
        return Err(ApiError::bad("the addressee is missing (?to=…)"));
    };
    let after = match query_param(query, "after") {
        None => 0,
        Some(a) => a
            .parse::<u64>()
            .map_err(|_| ApiError::bad("the cursor is malformed"))?,
    };
    let c = store.lock()?;
    let since = now.saturating_sub(MESSAGE_TTL_SECS);
    let rows = store::messages_after(&c, &to, after, since, MAX_MESSAGES_LISTED)?;
    let cursor = rows.last().map_or(after, |r| r.seq);
    let messages = rows
        .iter()
        .map(|r| stored_json(&r.json))
        .collect::<Result<Vec<_>, _>>()?;
    reply(
        200,
        json!({ "messages": messages, "cursor": cursor.to_string() }),
    )
}

// -------------------------------------------------------------- watch jobs

/// The id of a job: `<terms id>-<role>`.
pub fn job_id(job: &WatchJob) -> String {
    format!("{}-{}", job.terms.id(), role_name(job.role))
}

/// `POST /v1/watch`: register a party's presigned transactions. 201 when
/// stored; the same job again is a no-op (200); another job under the same
/// id is refused (409).
pub fn post_watch(store: &Store, cfg: &Config, body: &str, now: u64) -> ApiResult {
    check_size(body)?;
    let job: WatchJob =
        serde_json::from_str(body).map_err(|e| ApiError::bad(format!("bad watch job: {e}")))?;
    let Some(network) = cfg.serves(&job.terms.network) else {
        return Err(ApiError::bad(format!(
            "this relay does not serve the network '{}'",
            job.terms.network
        )));
    };
    if !cfg.watches(network) {
        return Err(ApiError::bad(format!(
            "this server runs no watchtower for '{}'",
            network.as_str()
        )));
    }
    job.check_fresh(now)?;
    let id = job_id(&job);
    let json = serde_json::to_string(&job).map_err(AppError::from)?;
    let c = store.lock()?;
    if let Some(row) = store::get_job(&c, &id)? {
        if row.json == json {
            return reply(200, json!({ "id": id }));
        }
        return Err(ApiError::new(
            409,
            "another job is registered for this trade and role",
        ));
    }
    // Nothing is checked yet: the status of a step that sees neither chain.
    let sent = SentLog::default();
    let status = engine::step(
        &job.holdings(sent.clone()),
        None,
        None,
        now,
        &Policy::for_network(network),
    )
    .status;
    store::insert_job(
        &c,
        &JobRow {
            id: id.clone(),
            network: network.as_str().to_owned(),
            json,
            status_json: serde_json::to_string(&status).map_err(AppError::from)?,
            sent_json: serde_json::to_string(&sent).map_err(AppError::from)?,
            created_at: now,
            updated_at: now,
            finished_at: None,
            last_error: None,
        },
    )?;
    reply(201, json!({ "id": id }))
}

/// `GET /v1/watch/<id>`: what the watchtower last saw and sent.
pub fn get_watch(store: &Store, id: &str) -> ApiResult {
    let c = store.lock()?;
    let Some(row) = store::get_job(&c, id)? else {
        return Err(ApiError::new(404, "no such watch job"));
    };
    reply(
        200,
        json!({
            "status": stored_json(&row.status_json)?,
            "sent": stored_json(&row.sent_json)?,
            "updatedAt": row.updated_at,
        }),
    )
}

// ------------------------------------------------------------------- purge

/// Remove what the server no longer keeps: expired offers, offers silent
/// for [`SILENT_OFFER_TTL_SECS`], messages older than [`MESSAGE_TTL_SECS`],
/// jobs finished more than [`FINISHED_JOB_TTL_SECS`] ago.
pub fn purge(store: &Store, now: u64) -> Result<store::Purged, AppError> {
    let c = store.lock()?;
    store::purge(
        &c,
        now,
        now.saturating_sub(SILENT_OFFER_TTL_SECS),
        now.saturating_sub(MESSAGE_TTL_SECS),
        now.saturating_sub(FINISHED_JOB_TTL_SECS),
    )
}

// ---------------------------------------------------------------- dispatch

/// Route one request. `path` has no query; `query` is the part after `?`.
pub fn dispatch(
    store: &Store,
    cfg: &Config,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    now: u64,
) -> Reply {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let result = match (method, segments.as_slice()) {
        ("GET", ["v1", "health"]) => Ok(health(cfg)),
        ("POST", ["v1", "offers"]) => post_offer(store, cfg, body, now),
        ("GET", ["v1", "offers"]) => list_offers(store, cfg, query, now),
        ("POST", ["v1", "offers", id, "withdraw"]) if is_hex64(id) => {
            withdraw_offer(store, id, body, now)
        }
        ("POST", ["v1", "messages"]) => post_message(store, cfg, body, now),
        ("GET", ["v1", "messages"]) => list_messages(store, query, now),
        ("POST", ["v1", "watch"]) => post_watch(store, cfg, body, now),
        ("GET", ["v1", "watch", id]) => get_watch(store, id),
        (_, s) if is_endpoint(s) => Err(ApiError::new(405, "method not allowed here")),
        _ => Err(ApiError::new(404, "no such endpoint")),
    };
    result.unwrap_or_else(ApiError::into_reply)
}

/// Whether `segments` name an endpoint (whatever the method).
fn is_endpoint(segments: &[&str]) -> bool {
    match segments {
        ["v1", "health" | "offers" | "messages" | "watch"] => true,
        ["v1", "offers", id, "withdraw"] => is_hex64(id),
        ["v1", "watch", _] => true,
        _ => false,
    }
}
