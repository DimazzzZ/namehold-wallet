//! The exchange's background step (spec §3.3), run by the sync in the app
//! and in the daemon. What it does, and what it never does:
//!
//! - renews this profile's live offers on the relay (the heartbeat);
//! - reads the relay keys' inboxes and answers a take with this profile's
//!   **public** keys (from the stored swap-branch public key) or a pooled
//!   hash: no private key and no secret is used;
//! - records the counterparty's answer and lock announcements, after
//!   checking them against the offer, the request and the policy;
//! - runs one engine step per trade (`runner::run_step`), which sends only
//!   transactions the user signed at a Commit;
//! - hands each committed side to the watchtower, without the secret.
//!
//! It holds no signing key: `tests::exchange_layering_tests` fails if this
//! file names one.

use async_trait::async_trait;
use rusqlite::Connection;

use crate::error::AppError;
use crate::exchange_io::btc_source::BtcSource;
use crate::exchange_io::relay_client::RelayClient;
use crate::exchange_io::runner;
use crate::exchange_io::store::{self, Trade};
use crate::noncustodial::exchange::engine::{Holdings, Role, SentLog, Status};
use crate::noncustodial::exchange::keys;
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::relay_msg::{
    self, Body, Envelope, LockAnnouncement, Message, Side, SignedOffer, TakeRequest,
};
use crate::noncustodial::exchange::terms::{OfferView, Terms, TermsDraft};
use crate::noncustodial::exchange::watch::WatchJob;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;

/// The relay as the job uses it; `RelayClient` in production, a fake in
/// tests.
#[async_trait]
pub trait Relay: Send + Sync {
    async fn publish_offer(&self, o: &SignedOffer) -> Result<(), AppError>;
    async fn send(&self, e: &Envelope) -> Result<(), AppError>;
    async fn inbox(
        &self,
        to: &str,
        network: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Envelope>, Option<String>), AppError>;
    async fn watch(&self, job: &WatchJob) -> Result<String, AppError>;
}

#[async_trait]
impl Relay for RelayClient {
    async fn publish_offer(&self, o: &SignedOffer) -> Result<(), AppError> {
        RelayClient::publish_offer(self, o).await
    }
    async fn send(&self, e: &Envelope) -> Result<(), AppError> {
        RelayClient::send(self, e).await
    }
    async fn inbox(
        &self,
        to: &str,
        network: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Envelope>, Option<String>), AppError> {
        RelayClient::inbox(self, to, network, cursor).await
    }
    async fn watch(&self, job: &WatchJob) -> Result<String, AppError> {
        RelayClient::watch(self, job).await
    }
}

/// How long a live offer may go without a heartbeat before it is renewed.
pub const HEARTBEAT_SECS: u64 = 60;
/// At most this many trades of one offer may be negotiating or awaiting the
/// Buyer's Commit at once.
pub const MAX_OPEN_PER_OFFER: usize = 3;
/// A trade is followed this long after it ended.
pub const FOLLOW_AFTER_END_SECS: u64 = 24 * 3600;

pub struct Ctx<'a> {
    pub conn: &'a Connection,
    pub profile_id: &'a str,
    pub network: Network,
    pub policy: Policy,
    pub node: Option<&'a dyn NodeRpc>,
    pub sources: &'a [Box<dyn BtcSource>],
    pub relay: Option<&'a dyn Relay>,
    pub watchtower: Option<&'a dyn Relay>,
    pub now: u64,
}

/// Something the person should hear about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A trade waits for this person's Commit until `deadline`.
    NeedsCommit {
        trade_id: String,
        deadline: u64,
    },
    Completed {
        trade_id: String,
    },
    Refunded {
        trade_id: String,
    },
    NeedsAttention {
        trade_id: String,
    },
}

#[derive(Debug, Default)]
pub struct TickReport {
    pub events: Vec<Event>,
    pub errors: Vec<String>,
}

fn is_terminal(s: &Status) -> bool {
    matches!(
        s,
        Status::Completed | Status::Refunded { .. } | Status::Expired
    )
}

/// One tick for one profile. Best effort: an error on one offer, message or
/// trade is reported and the rest go on.
pub async fn tick(ctx: &Ctx<'_>) -> TickReport {
    let mut report = TickReport::default();
    let Ok(Some(account)) = store::account(ctx.conn, ctx.profile_id) else {
        return report; // the exchange is not enabled for this profile
    };
    if let Some(relay) = ctx.relay {
        if let Err(e) = heartbeat(ctx, relay).await {
            report.errors.push(e.to_string());
        }
        if let Err(e) = read_inboxes(ctx, relay, &account).await {
            report.errors.push(e.to_string());
        }
    }
    match step_trades(ctx).await {
        Ok(ev) => report.events.extend(ev),
        Err(e) => report.errors.push(e.to_string()),
    }
    if let Some(w) = ctx.watchtower {
        if let Err(e) = register_watch_jobs(ctx, w).await {
            report.errors.push(e.to_string());
        }
    }
    report
}

async fn heartbeat(ctx: &Ctx<'_>, relay: &dyn Relay) -> Result<(), AppError> {
    for o in store::offers(ctx.conn, ctx.profile_id)? {
        if !o.live || o.network != ctx.network.as_str() {
            continue;
        }
        if o.signed.offer.expires_at <= ctx.now {
            store::withdraw_offer(ctx.conn, ctx.profile_id, &o.id)?;
            continue;
        }
        if o.last_published_at
            .is_some_and(|t| t + HEARTBEAT_SECS > ctx.now)
        {
            continue;
        }
        let err = relay
            .publish_offer(&o.signed)
            .await
            .err()
            .map(|e| e.to_string());
        store::set_offer_published(ctx.conn, &o.id, ctx.now, err.as_deref())?;
    }
    Ok(())
}

/// Every relay key this profile reads: its live offers' and its open
/// trades'.
fn inbox_keys(ctx: &Ctx<'_>) -> Result<Vec<(String, Option<String>)>, AppError> {
    let mut keys = Vec::new();
    for o in store::offers(ctx.conn, ctx.profile_id)? {
        if o.live {
            keys.push((
                relay_msg::relay_pubkey_hex(&o.relay_key),
                Some(o.id.clone()),
            ));
        }
    }
    for t in store::trades(ctx.conn, ctx.profile_id)? {
        if !t.finished && !t.maker {
            keys.push((relay_msg::relay_pubkey_hex(&t.relay_key), None));
        }
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

async fn read_inboxes(
    ctx: &Ctx<'_>,
    relay: &dyn Relay,
    account: &store::Account,
) -> Result<(), AppError> {
    for (key, _) in inbox_keys(ctx)? {
        let cursor = store::inbox_cursor(ctx.conn, &key)?;
        let (msgs, next) = relay
            .inbox(&key, ctx.network.as_str(), cursor.as_deref())
            .await?;
        for m in msgs {
            if let Err(e) = handle(ctx, relay, account, &m).await {
                eprintln!("exchange: message {} dropped: {e}", m.message.trade_id);
            }
        }
        store::set_inbox_cursor(ctx.conn, ctx.profile_id, &key, next.as_deref())?;
    }
    Ok(())
}

fn offer_view(o: &SignedOffer) -> OfferView<'_> {
    OfferView {
        offer_id: "",
        network: &o.offer.network,
        price_msat_per_hns: o.offer.price_msat_per_hns,
        min_hns: o.offer.min_hns,
        max_hns: o.offer.max_hns,
    }
}

fn reply(
    ctx: &Ctx<'_>,
    from: &secp256k1::SecretKey,
    to: &str,
    offer_id: &str,
    trade_id: &str,
    body: Body,
) -> Result<Envelope, AppError> {
    Message {
        version: relay_msg::MSG_VERSION,
        network: ctx.network.as_str().to_owned(),
        offer_id: offer_id.to_owned(),
        trade_id: trade_id.to_owned(),
        from: relay_msg::relay_pubkey_hex(from),
        to: to.to_owned(),
        sent_at: ctx.now,
        body,
    }
    .sign(from)
}

async fn handle(
    ctx: &Ctx<'_>,
    relay: &dyn Relay,
    account: &store::Account,
    e: &Envelope,
) -> Result<(), AppError> {
    let m = &e.message;
    match &m.body {
        Body::Take(req) => on_take(ctx, relay, account, m, req).await,
        Body::Accept { terms } => on_accept(ctx, m, terms),
        Body::BtcLocked(a) => on_btc_locked(ctx, m, a),
        Body::HnsLocked(a) => on_hns_locked(ctx, m, a),
        Body::Decline { reason } => on_decline(ctx, m, reason),
    }
}

/// A take of one of our offers: answer with our public half of the terms.
async fn on_take(
    ctx: &Ctx<'_>,
    relay: &dyn Relay,
    account: &store::Account,
    m: &Message,
    req: &TakeRequest,
) -> Result<(), AppError> {
    let offers = store::offers(ctx.conn, ctx.profile_id)?;
    let Some(o) = offers.into_iter().find(|o| o.id == m.offer_id && o.live) else {
        return Err(AppError::InvalidInput(
            "a take of an offer that is not ours or not live".into(),
        ));
    };
    if relay_msg::relay_pubkey_hex(&o.relay_key) != m.to {
        return Err(AppError::InvalidInput("a take sent to another key".into()));
    }
    if store::trade(ctx.conn, ctx.profile_id, &m.trade_id)?.is_some() {
        return Ok(()); // already answered
    }
    let decline = |reason: &str| {
        reply(
            ctx,
            &o.relay_key,
            &m.from,
            &m.offer_id,
            &m.trade_id,
            Body::Decline {
                reason: reason.into(),
            },
        )
    };
    let open = store::trades(ctx.conn, ctx.profile_id)?
        .into_iter()
        .filter(|t| {
            t.offer.offer.id() == o.id
                && !t.finished
                && t.holdings.as_ref().is_none_or(|h| !h.committed)
        })
        .count();
    if open >= MAX_OPEN_PER_OFFER {
        relay
            .send(&decline(
                "the maker has too many trades open on this offer",
            )?)
            .await?;
        return Ok(());
    }
    let role = match o.side {
        Side::Sell => Role::Seller,
        Side::Buy => Role::Buyer,
    };
    let parse_pk = |s: &str| -> Result<[u8; 33], AppError> {
        let mut b = [0u8; 33];
        hex::decode_to_slice(s, &mut b)
            .map_err(|_| AppError::InvalidInput("a key in the take is malformed".into()))?;
        Ok(b)
    };
    let taker_hns = parse_pk(&req.hns_pubkey)?;
    let taker_btc = parse_pk(&req.btc_pubkey)?;
    let taker_relay = parse_pk(&m.from)?;
    let maker_relay = parse_pk(&m.to)?;
    let (index, hash, secret_nonce) = match role {
        Role::Seller => {
            let Some(h) = req.hash.as_deref() else {
                relay
                    .send(&decline("a buyer's take carries the hash")?)
                    .await?;
                return Ok(());
            };
            let mut hb = [0u8; 32];
            hex::decode_to_slice(h, &mut hb)
                .map_err(|_| AppError::InvalidInput("the take's hash is malformed".into()))?;
            (
                store::take_trade_indexes(ctx.conn, ctx.profile_id, 1)?,
                hb,
                None,
            )
        }
        Role::Buyer => {
            if req.hash.is_some() {
                relay
                    .send(&decline("a seller's take carries no hash")?)
                    .await?;
                return Ok(());
            }
            match store::take_hash(ctx.conn, ctx.profile_id)? {
                Some((i, h, n)) => (i, h, Some(n)),
                None => {
                    relay
                        .send(&decline(
                            "the maker has no trade left on this offer; it must renew it",
                        )?)
                        .await?;
                    return Ok(());
                }
            }
        }
    };
    let (my_hns, my_btc) = keys::trade_pubkeys(&account.swap_xpub, index)?;
    let draft = match role {
        Role::Seller => TermsDraft {
            network: ctx.network.as_str(),
            offer_id: &o.id,
            trade_id: &m.trade_id,
            hns_amount: req.hns_amount,
            price_msat_per_hns: o.signed.offer.price_msat_per_hns,
            hash,
            seller_hns_refund_pubkey: my_hns,
            buyer_hns_claim_pubkey: taker_hns,
            buyer_btc_refund_pubkey: taker_btc,
            seller_btc_claim_pubkey: my_btc,
            seller_relay_pubkey: maker_relay,
            buyer_relay_pubkey: taker_relay,
            start: ctx.now,
        },
        Role::Buyer => TermsDraft {
            network: ctx.network.as_str(),
            offer_id: &o.id,
            trade_id: &m.trade_id,
            hns_amount: req.hns_amount,
            price_msat_per_hns: o.signed.offer.price_msat_per_hns,
            hash,
            seller_hns_refund_pubkey: taker_hns,
            buyer_hns_claim_pubkey: my_hns,
            buyer_btc_refund_pubkey: my_btc,
            seller_btc_claim_pubkey: taker_btc,
            seller_relay_pubkey: taker_relay,
            buyer_relay_pubkey: maker_relay,
            start: ctx.now,
        },
    };
    let terms = Terms::from_draft(&draft, &ctx.policy)?;
    let view = OfferView {
        offer_id: &o.id,
        ..offer_view(&o.signed)
    };
    if let Err(e) = terms.check(&ctx.policy, &view, ctx.now) {
        relay.send(&decline(&e.to_string())?).await?;
        return Ok(());
    }
    let mut t = Trade {
        profile_id: ctx.profile_id.to_owned(),
        trade_id: m.trade_id.clone(),
        network: ctx.network.as_str().to_owned(),
        role,
        maker: true,
        offer: o.signed.clone(),
        trade_index: index,
        relay_key: o.relay_key,
        counterparty_key: Some(m.from.clone()),
        hns_amount: terms.hns_amount,
        btc_sats: terms.btc_amount,
        price_msat: terms.price_msat_per_hns,
        terms: Some(terms.clone()),
        holdings: Some(uncommitted(role, &terms)),
        status: None,
        declined: None,
        last_error: None,
        watchtower: "none".into(),
        hns_lock: None,
        request: None,
        secret_nonce,
        terminal_at: None,
        finished: false,
        version: 0,
        created_at: ctx.now,
        updated_at: ctx.now,
    };
    store::insert_trade(ctx.conn, &t)?;
    store::update_trade(ctx.conn, &mut t, ctx.now)?;
    let accept = reply(
        ctx,
        &o.relay_key,
        &m.from,
        &o.id,
        &m.trade_id,
        Body::Accept {
            terms: Box::new(terms),
        },
    )?;
    relay.send(&accept).await
}

pub fn uncommitted(role: Role, terms: &Terms) -> Holdings {
    Holdings {
        role,
        terms: terms.clone(),
        committed: false,
        secret: None,
        hns_claim: None,
        btc_funding_hex: None,
        btc_refunds: Vec::new(),
        hns_funding_hex: None,
        hns_refund_hex: None,
        btc_claims: Vec::new(),
        btc_lock: None,
        hns_lock: None,
        sent: SentLog::default(),
    }
}

/// The maker's answer to our take: keep it only if it is exactly what we
/// asked for, under the offer we took and the policy.
fn on_accept(ctx: &Ctx<'_>, m: &Message, terms: &Terms) -> Result<(), AppError> {
    let Some(mut t) = store::trade(ctx.conn, ctx.profile_id, &m.trade_id)? else {
        return Err(AppError::InvalidInput(
            "an answer to a trade we do not have".into(),
        ));
    };
    if t.maker || t.terms.is_some() {
        return Ok(());
    }
    let refuse = |why: &str| {
        Err(AppError::InvalidInput(format!(
            "the maker's terms are refused: {why}"
        )))
    };
    if m.from != t.offer.offer.maker_relay_pubkey {
        return refuse("not from the offer's maker");
    }
    let Some(req) = &t.request else {
        return refuse("we have no record of our request");
    };
    let mine = relay_msg::relay_pubkey_hex(&t.relay_key);
    let (my_relay, their_relay) = match t.role {
        Role::Buyer => (&terms.buyer_relay_pubkey, &terms.seller_relay_pubkey),
        Role::Seller => (&terms.seller_relay_pubkey, &terms.buyer_relay_pubkey),
    };
    if *my_relay != mine || *their_relay != m.from {
        return refuse("the relay keys are not ours and the maker's");
    }
    let (my_hns, my_btc) = match t.role {
        Role::Buyer => (
            &terms.buyer_hns_claim_pubkey,
            &terms.buyer_btc_refund_pubkey,
        ),
        Role::Seller => (
            &terms.seller_hns_refund_pubkey,
            &terms.seller_btc_claim_pubkey,
        ),
    };
    if *my_hns != req.hns_pubkey || *my_btc != req.btc_pubkey {
        return refuse("our keys were changed");
    }
    if t.role == Role::Buyer && Some(&terms.hash) != req.hash.as_ref() {
        return refuse("our hash was changed");
    }
    if terms.trade_id != t.trade_id || terms.hns_amount != req.hns_amount {
        return refuse("another trade or amount");
    }
    let offer_id = t.offer.offer.id();
    let view = OfferView {
        offer_id: &offer_id,
        ..offer_view(&t.offer)
    };
    terms.check(&ctx.policy, &view, ctx.now)?;
    t.counterparty_key = Some(m.from.clone());
    t.btc_sats = terms.btc_amount;
    t.holdings = Some(uncommitted(t.role, terms));
    t.terms = Some(terms.clone());
    store::update_trade(ctx.conn, &mut t, ctx.now).map(|_| ())
}

fn counterparty_trade(ctx: &Ctx<'_>, m: &Message, terms_id: &str) -> Result<Trade, AppError> {
    let Some(t) = store::trade(ctx.conn, ctx.profile_id, &m.trade_id)? else {
        return Err(AppError::InvalidInput(
            "a message about a trade we do not have".into(),
        ));
    };
    let Some(terms) = &t.terms else {
        return Err(AppError::InvalidInput(
            "a lock announced before the terms".into(),
        ));
    };
    let theirs = match t.role {
        Role::Buyer => &terms.seller_relay_pubkey,
        Role::Seller => &terms.buyer_relay_pubkey,
    };
    if *theirs != m.from || terms.id() != terms_id {
        return Err(AppError::InvalidInput(
            "a lock announced by someone else or for other terms".into(),
        ));
    }
    Ok(t)
}

/// The Buyer's BTC lock: recorded so the Seller's step can check it on
/// Bitcoin (proven there, never on this message's word).
fn on_btc_locked(ctx: &Ctx<'_>, m: &Message, a: &LockAnnouncement) -> Result<(), AppError> {
    let mut t = counterparty_trade(ctx, m, &a.terms_id)?;
    if t.role != Role::Seller {
        return Ok(());
    }
    crate::noncustodial::exchange::btc_htlc::parse_txid(&a.txid)?;
    if let Some(h) = t.holdings.as_mut() {
        if h.btc_lock.is_none() {
            h.btc_lock = Some(crate::noncustodial::exchange::engine::LockRef {
                txid: a.txid.clone(),
                vout: a.vout,
            });
        }
    }
    store::update_trade(ctx.conn, &mut t, ctx.now).map(|_| ())
}

/// The Seller's HNS lock: shown on the trade; the Buyer's step finds the
/// lock by its address on its own node.
fn on_hns_locked(ctx: &Ctx<'_>, m: &Message, a: &LockAnnouncement) -> Result<(), AppError> {
    let mut t = counterparty_trade(ctx, m, &a.terms_id)?;
    if t.role == Role::Buyer && t.hns_lock.is_none() {
        t.hns_lock = Some((a.txid.clone(), a.vout));
        store::update_trade(ctx.conn, &mut t, ctx.now)?;
    }
    Ok(())
}

/// A decline ends a trade only before our Commit: after it, the chains
/// decide.
fn on_decline(ctx: &Ctx<'_>, m: &Message, reason: &str) -> Result<(), AppError> {
    let Some(mut t) = store::trade(ctx.conn, ctx.profile_id, &m.trade_id)? else {
        return Ok(());
    };
    let expected = t
        .counterparty_key
        .clone()
        .unwrap_or_else(|| t.offer.offer.maker_relay_pubkey.clone());
    if m.from != expected {
        return Err(AppError::InvalidInput("a decline from someone else".into()));
    }
    if t.holdings.as_ref().is_some_and(|h| h.committed) {
        return Ok(());
    }
    t.declined = Some(reason.chars().take(200).collect());
    t.status = Some(Status::Expired);
    t.terminal_at = Some(ctx.now);
    t.finished = true;
    store::update_trade(ctx.conn, &mut t, ctx.now).map(|_| ())
}

async fn step_trades(ctx: &Ctx<'_>) -> Result<Vec<Event>, AppError> {
    let mut events = Vec::new();
    for mut t in store::trades(ctx.conn, ctx.profile_id)? {
        if t.finished || t.network != ctx.network.as_str() {
            continue;
        }
        let Some(mut h) = t.holdings.clone() else {
            // Negotiating: give up after the Buyer's funding window.
            if t.created_at + ctx.policy.buyer_fund_window < ctx.now {
                t.status = Some(Status::Expired);
                t.terminal_at = Some(ctx.now);
                t.finished = true;
                store::update_trade(ctx.conn, &mut t, ctx.now)?;
            }
            continue;
        };
        let rep = runner::run_step(ctx.node, ctx.sources, &mut h, ctx.now, &ctx.policy).await;
        let Some(step) = rep.step else { continue };
        let before = t.status.clone();
        if before.as_ref() != Some(&step.status) {
            match (&step.status, t.role) {
                (Status::AwaitingBuyerCommit { deadline }, Role::Buyer)
                | (Status::AwaitingSellerCommit { deadline }, Role::Seller) => {
                    events.push(Event::NeedsCommit {
                        trade_id: t.trade_id.clone(),
                        deadline: *deadline,
                    })
                }
                (Status::Completed, _) => events.push(Event::Completed {
                    trade_id: t.trade_id.clone(),
                }),
                (Status::Refunded { .. }, _) => events.push(Event::Refunded {
                    trade_id: t.trade_id.clone(),
                }),
                (Status::NeedsAttention { .. }, _) => events.push(Event::NeedsAttention {
                    trade_id: t.trade_id.clone(),
                }),
                _ => {}
            }
        }
        if is_terminal(&step.status) {
            let at = *t.terminal_at.get_or_insert(ctx.now);
            if at + FOLLOW_AFTER_END_SECS < ctx.now {
                t.finished = true;
            }
        } else {
            t.terminal_at = None;
        }
        t.last_error = (!rep.errors.is_empty()).then(|| rep.errors.join("; "));
        t.status = Some(step.status);
        t.holdings = Some(h);
        store::update_trade(ctx.conn, &mut t, ctx.now)?;
    }
    Ok(events)
}

async fn register_watch_jobs(ctx: &Ctx<'_>, w: &dyn Relay) -> Result<(), AppError> {
    for mut t in store::trades(ctx.conn, ctx.profile_id)? {
        if t.finished || t.watchtower == "registered" {
            continue;
        }
        let Some(h) = &t.holdings else { continue };
        if !h.committed {
            continue;
        }
        let job = WatchJob::from_holdings(h);
        t.watchtower = match w.watch(&job).await {
            Ok(_) => "registered".into(),
            Err(e) => {
                eprintln!("exchange: watchtower refused {}: {e}", t.trade_id);
                "failed".into()
            }
        };
        store::update_trade(ctx.conn, &mut t, ctx.now)?;
    }
    Ok(())
}
